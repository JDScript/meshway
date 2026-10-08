//! A fixed receive buffer avoids netlink-proto's unbounded multicast queue.
use rtnetlink::{
    MulticastGroup,
    packet_core::{NetlinkMessage, NetlinkPayload},
    packet_route::{
        RouteNetlinkMessage,
        address::AddressAttribute,
        link::{LinkAttribute, LinkFlags},
        route::RouteAttribute,
    },
    sys::{Socket, SocketAddr, protocols::NETLINK_ROUTE},
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::io::unix::AsyncFd;

use super::{
    super::protection::Shared,
    plan::{encode_route, encode_rule, route_table},
};
use crate::{Error, Result};

#[derive(PartialEq, Eq)]
pub(super) struct Event {
    pub rule: bool,
    pub deleted: bool,
    pub bytes: Vec<u8>,
}

#[derive(Default)]
pub(super) struct Expected {
    #[cfg(test)]
    pub first_conflict: Option<String>,
    pub events: VecDeque<Event>,
    pub pools: Vec<ipnet::IpNet>,
    pub tun: Option<(u32, String, usize, Vec<std::net::IpAddr>)>,
    pub anchor: Option<super::anchor::Record>,
    pub anchor_deleting: bool,
    pub changing: bool,
    pub wake: Arc<tokio::sync::Notify>,
    pub requested_drain: u64,
    pub drained: u64,
}

// Kept inside the kernel-test binary: one fixed-field record on the first
// rejection, without installing a subscriber or logging raw netlink payloads.
#[cfg(test)]
#[derive(Default)]
struct Facts {
    kind: &'static str,
    index: Option<u32>,
    address: Option<std::net::IpAddr>,
    local: Option<std::net::IpAddr>,
    prefix: Option<u8>,
    flags: Option<u32>,
}
#[cfg(test)]
impl Facts {
    fn from_payload(payload: &NetlinkPayload<RouteNetlinkMessage>) -> Self {
        use rtnetlink::packet_route::route::{RouteAddress, RouteAttribute};
        match payload {
            NetlinkPayload::InnerMessage(
                RouteNetlinkMessage::NewAddress(address) | RouteNetlinkMessage::DelAddress(address),
            ) => Self {
                kind: if matches!(
                    payload,
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewAddress(_))
                ) {
                    "RTM_NEWADDR"
                } else {
                    "RTM_DELADDR"
                },
                index: Some(address.header.index),
                address: address.attributes.iter().find_map(|a| {
                    if let AddressAttribute::Address(ip) = a {
                        Some(*ip)
                    } else {
                        None
                    }
                }),
                local: address.attributes.iter().find_map(|a| {
                    if let AddressAttribute::Local(ip) = a {
                        Some(*ip)
                    } else {
                        None
                    }
                }),
                prefix: Some(address.header.prefix_len),
                flags: Some(
                    address
                        .attributes
                        .iter()
                        .find_map(|a| {
                            if let AddressAttribute::Flags(flags) = a {
                                Some(flags.bits())
                            } else {
                                None
                            }
                        })
                        .unwrap_or(u32::from(address.header.flags.bits())),
                ),
            },
            NetlinkPayload::InnerMessage(
                RouteNetlinkMessage::NewLink(link) | RouteNetlinkMessage::DelLink(link),
            ) => Self {
                kind: if matches!(
                    payload,
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewLink(_))
                ) {
                    "RTM_NEWLINK"
                } else {
                    "RTM_DELLINK"
                },
                index: Some(link.header.index),
                flags: Some(link.header.flags.bits()),
                ..Self::default()
            },
            NetlinkPayload::InnerMessage(
                RouteNetlinkMessage::NewRoute(route) | RouteNetlinkMessage::DelRoute(route),
            ) => Self {
                kind: if matches!(
                    payload,
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewRoute(_))
                ) {
                    "RTM_NEWROUTE"
                } else {
                    "RTM_DELROUTE"
                },
                index: route.attributes.iter().find_map(|a| {
                    if let RouteAttribute::Oif(index) = a {
                        Some(*index)
                    } else {
                        None
                    }
                }),
                address: route.attributes.iter().find_map(|a| match a {
                    RouteAttribute::Destination(RouteAddress::Inet(ip)) => Some((*ip).into()),
                    RouteAttribute::Destination(RouteAddress::Inet6(ip)) => Some((*ip).into()),
                    _ => None,
                }),
                prefix: Some(route.header.destination_prefix_length),
                flags: Some(route.header.flags.bits()),
                ..Self::default()
            },
            NetlinkPayload::InnerMessage(
                RouteNetlinkMessage::NewRule(rule) | RouteNetlinkMessage::DelRule(rule),
            ) => Self {
                kind: if matches!(
                    payload,
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewRule(_))
                ) {
                    "RTM_NEWRULE"
                } else {
                    "RTM_DELRULE"
                },
                flags: Some(rule.header.flags.bits()),
                ..Self::default()
            },
            _ => Self {
                kind: "OTHER",
                ..Self::default()
            },
        }
    }

    fn reject(&self, state: &mut Expected, branch: &'static str) {
        if state.first_conflict.is_some() {
            return;
        }
        let known = state.tun.as_ref().map(|(index, _, mtu, addresses)| {
            (
                *index,
                *mtu,
                addresses.iter().find(|ip| ip.is_ipv4()).copied(),
                addresses.iter().find(|ip| ip.is_ipv6()).copied(),
            )
        });
        state.first_conflict = Some(format!(
            "branch={branch} rt_type={} index={:?} address={:?} local={:?} prefix={:?} flags={:?} changing={} known_tun={known:?} requested={} drained={} expected_events={}",
            self.kind,
            self.index,
            self.address,
            self.local,
            self.prefix,
            self.flags,
            state.changing,
            state.requested_drain,
            state.drained,
            state.events.len(),
        ));
    }
}

impl Expected {
    pub fn add(&mut self, event: Event) -> Result<()> {
        if self.events.len() >= 1024 {
            return Err(Error::Capacity);
        }
        self.events.push_back(event);
        Ok(())
    }

    fn consume(&mut self, event: Event) -> Result<()> {
        let position = self
            .events
            .iter()
            .position(|expected| *expected == event)
            .ok_or(Error::Conflict)?;
        self.events.remove(position);
        Ok(())
    }
}

pub(super) fn open() -> Result<AsyncFd<Socket>> {
    let mut socket = Socket::new(NETLINK_ROUTE)?;
    socket.set_non_blocking(true)?;
    socket.bind(&SocketAddr::new(0, 0))?;
    for group in [
        MulticastGroup::Link,
        MulticastGroup::Ipv4Route,
        MulticastGroup::Ipv6Route,
        MulticastGroup::Ipv4Rule,
        MulticastGroup::Ipv6Rule,
        MulticastGroup::Ipv4Ifaddr,
        MulticastGroup::Ipv6Ifaddr,
    ] {
        socket.add_membership(group as u32)?;
    }
    Ok(AsyncFd::new(socket)?)
}

pub(super) async fn run(
    socket: AsyncFd<Socket>,
    shared: Arc<Shared>,
    expected: Arc<Mutex<Expected>>,
    table: u32,
) {
    if let Err(_error) = monitor(socket, &expected, table, &shared.network_changes).await {
        tracing::warn!(error = %_error, "Underlay kernel monitor stopped");
        #[cfg(test)]
        if let Ok(mut state) = expected.lock() {
            Facts {
                kind: match _error {
                    Error::Io(_) => "IO_ERROR",
                    Error::Netlink => "NETLINK_ERROR",
                    Error::Conflict => "CONFLICT",
                    Error::Worker => "WORKER_ERROR",
                    _ => "OTHER_ERROR",
                },
                ..Facts::default()
            }
            .reject(&mut state, "monitor_error");
        }
        shared.conflict.cancel();
    }
}

async fn monitor(
    socket: AsyncFd<Socket>,
    expected: &Mutex<Expected>,
    table: u32,
    changes: &tokio::sync::watch::Sender<u64>,
) -> Result<()> {
    let mut buffer = vec![0; 65536];
    let wake = expected.lock().map_err(|_| Error::Worker)?.wake.clone();
    loop {
        let mut ready = tokio::select! {
            result = socket.readable() => Some(result?),
            _ = wake.notified() => None,
        };
        // Only acknowledge requests already present before this drain began.
        // A later request retains its Notify permit for a subsequent drain.
        let through = expected.lock().map_err(|_| Error::Worker)?.requested_drain;
        loop {
            let (size, source) = match socket.get_ref().recv_from(&mut &mut buffer[..], 0) {
                Ok(received) => received,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if let Some(ready) = &mut ready {
                        ready.clear_ready();
                    }
                    let mut state = expected.lock().map_err(|_| Error::Worker)?;
                    state.drained = state.drained.max(through);
                    break;
                }
                Err(error) => return Err(error.into()),
            };
            // Only kernel-originated notifications are authoritative. A process
            // with NET_ADMIN can multicast to this group (Docker does so during
            // interface replacement); its message must neither mutate our
            // projection nor permanently revoke the kernel owner's protection.
            if source.port_number() != 0 {
                tokio::task::yield_now().await;
                continue;
            }
            if size == 0 || size >= buffer.len() {
                #[cfg(test)]
                if let Ok(mut state) = expected.lock() {
                    Facts {
                        kind: "DATAGRAM",
                        ..Facts::default()
                    }
                    .reject(&mut state, "invalid_datagram");
                }
                return Err(Error::Conflict);
            }
            let mut offset = 0;
            while offset < size {
                let header = buffer.get(offset..offset + 4).ok_or(Error::Netlink)?;
                let length =
                    u32::from_ne_bytes(header.try_into().map_err(|_| Error::Netlink)?) as usize;
                if length < 16 || length > size - offset {
                    return Err(Error::Netlink);
                }
                let message = NetlinkMessage::<RouteNetlinkMessage>::deserialize(
                    &buffer[offset..offset + length],
                )
                .map_err(|_| Error::Netlink)?;
                let mut state = expected.lock().map_err(|_| Error::Worker)?;
                #[cfg(test)]
                let facts = Facts::from_payload(&message.payload);
                macro_rules! conflict {
                    ($branch:literal) => {{
                        tracing::warn!(reason = $branch, "Underlay kernel protection conflict");
                        #[cfg(test)]
                        facts.reject(&mut state, $branch);
                        return Err(Error::Conflict);
                    }};
                }
                let changed = network_changed(&state, &message.payload);
                let owned =
                    state
                        .tun
                        .as_ref()
                        .map(|(index, _, _, addresses)| super::capture::Owned {
                            index: *index,
                            addresses,
                        });
                // The same predicate guards install/retention dumps. Creation
                // only exempts the descriptor's exact /32 or /128 local route;
                // it never suppresses foreign conflicts in a changing window.
                match &message.payload {
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewAddress(address))
                        if super::capture::address_conflicts(&state.pools, owned, address) =>
                    {
                        conflict!("capture_address");
                    }
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewRoute(route))
                        if super::capture::route_conflicts(&state.pools, owned, route) =>
                    {
                        conflict!("capture_route");
                    }
                    _ => {}
                }
                // Anchor ownership is independent of TUN installation's changing
                // window. Never suppress its checks while a lease refresh runs.
                if let Some(anchor) = &state.anchor {
                    match &message.payload {
                        NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewLink(link))
                            if link.header.index != anchor.identity.index && link.attributes.iter().any(|a| matches!(a, LinkAttribute::IfAlias(v) if v == &anchor.alias)) => conflict!("anchor_alias"),
                        NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewAddress(address))
                            if address.header.index != anchor.identity.index && address.attributes.iter().any(|a| matches!(a, AddressAttribute::Address(std::net::IpAddr::V4(ip)) | AddressAttribute::Local(std::net::IpAddr::V4(ip)) if *ip == anchor.identity.address)) => conflict!("anchor_address_collision"),
                        NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelRoute(route))
                            if !state.anchor_deleting && super::anchor::route_conflicts(anchor.identity.address, None, route) => conflict!("anchor_route_deleted"),
                        NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewLink(link))
                            if link.header.index == anchor.identity.index
                                && !(super::anchor::link_matches(anchor, link)
                                     || (state.anchor_deleting && super::anchor::deleting_link_matches(anchor, link))) =>
                        {
                            conflict!("anchor_link_changed");
                        }
                        NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelLink(link))
                            if link.header.index == anchor.identity.index
                                && !state.anchor_deleting =>
                        {
                            conflict!("anchor_link_deleted");
                        }
                        NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewAddress(address))
                            if address.header.index == anchor.identity.index
                                && !super::anchor::address_matches(anchor, address) =>
                        {
                            conflict!("anchor_address_changed");
                        }
                        NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelAddress(address))
                            if address.header.index == anchor.identity.index
                                && !state.anchor_deleting =>
                        {
                            conflict!("anchor_address_deleted");
                        }
                        _ => {}
                    }
                }
                match message.payload {
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewRule(rule)) => {
                        if state
                            .consume(Event {
                                rule: true,
                                deleted: false,
                                bytes: encode_rule(rule),
                            })
                            .is_err()
                        {
                            conflict!("unexpected_new_rule");
                        }
                    }
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelRule(rule)) => {
                        if state
                            .consume(Event {
                                rule: true,
                                deleted: true,
                                bytes: encode_rule(rule),
                            })
                            .is_err()
                        {
                            conflict!("unexpected_del_rule");
                        }
                    }
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewRoute(route))
                        if route_table(&route) == table =>
                    {
                        if state
                            .consume(Event {
                                rule: false,
                                deleted: false,
                                bytes: encode_route(route),
                            })
                            .is_err()
                        {
                            conflict!("unexpected_new_route");
                        }
                    }
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelRoute(route))
                        if route_table(&route) == table =>
                    {
                        if state
                            .consume(Event {
                                rule: false,
                                deleted: true,
                                bytes: encode_route(route),
                            })
                            .is_err()
                        {
                            conflict!("unexpected_del_route");
                        }
                    }
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelLink(link)) => {
                        if state
                            .tun
                            .as_ref()
                            .is_some_and(|(index, _, _, _)| *index == link.header.index)
                        {
                            conflict!("tun_link_deleted");
                        }
                    }
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewLink(link))
                        if !state.changing =>
                    {
                        if let Some((index, name, mtu, _)) = &state.tun
                            && *index == link.header.index
                            && (!link.header.flags.contains(LinkFlags::Up)
                                || !link
                                    .attributes
                                    .iter()
                                    .any(|a| matches!(a, LinkAttribute::IfName(v) if v == name))
                                || !link.attributes.iter().any(
                                    |a| matches!(a, LinkAttribute::Mtu(v) if *v as usize == *mtu),
                                ))
                        {
                            conflict!("tun_link_changed");
                        }
                    }
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelAddress(address))
                        if !state.changing =>
                    {
                        if state
                            .tun
                            .as_ref()
                            .is_some_and(|(index, _, _, _)| *index == address.header.index)
                        {
                            conflict!("tun_address_deleted");
                        }
                    }
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewAddress(address))
                        if !state.changing =>
                    {
                        if let Some((index, _, _, known)) = &state.tun
                            && *index == address.header.index
                        {
                            let flags = address
                                .attributes
                                .iter()
                                .find_map(|a| {
                                    if let AddressAttribute::Flags(flags) = a {
                                        Some(flags.bits())
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or(u32::from(address.header.flags.bits()));
                            if flags & 0xc8 != 0x80 || !address.attributes.iter().any(|a| matches!(a, AddressAttribute::Address(ip) | AddressAttribute::Local(ip) if known.contains(ip) && address.header.prefix_len == if ip.is_ipv4() { 32 } else { 128 })) {
                            conflict!("tun_address_changed");
                        }
                        }
                    }
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewRoute(route)) => {
                        if let Some(anchor) = &state.anchor
                            && super::anchor::route_conflicts(
                                anchor.identity.address,
                                Some(anchor.identity.index),
                                &route,
                            )
                        {
                            conflict!("anchor_route_collision");
                        }
                    }
                    NetlinkPayload::InnerMessage(_) | NetlinkPayload::Noop => {}
                    _ => return Err(Error::Netlink),
                }
                offset += (length + 3) & !3;
                if changed {
                    changes.send_modify(|epoch| *epoch = epoch.wrapping_add(1));
                }
            }
            // Every received datagram is finite; cooperate even during event floods.
            tokio::task::yield_now().await;
        }
    }
}

fn network_changed(state: &Expected, payload: &NetlinkPayload<RouteNetlinkMessage>) -> bool {
    let external = |index| {
        index != 1
            && state
                .tun
                .as_ref()
                .is_none_or(|(owned, _, _, _)| *owned != index)
            && state
                .anchor
                .as_ref()
                .is_none_or(|anchor| anchor.identity.index != index)
    };
    match payload {
        NetlinkPayload::InnerMessage(
            RouteNetlinkMessage::NewLink(link) | RouteNetlinkMessage::DelLink(link),
        ) => external(link.header.index),
        NetlinkPayload::InnerMessage(
            RouteNetlinkMessage::NewAddress(address) | RouteNetlinkMessage::DelAddress(address),
        ) => external(address.header.index),
        NetlinkPayload::InnerMessage(
            RouteNetlinkMessage::NewRoute(route) | RouteNetlinkMessage::DelRoute(route),
        ) => {
            route_table(route) == 254
                && route.attributes.iter().all(|attribute| match attribute {
                    RouteAttribute::Oif(index) => external(*index),
                    _ => true,
                })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtnetlink::packet_route::address::AddressMessage;

    #[test]
    fn first_conflict_is_bounded_and_omits_unselected_attributes_and_expected_bytes() {
        let mut address = AddressMessage::default();
        address.header.index = 9;
        address.header.prefix_len = 64;
        address
            .attributes
            .push(AddressAttribute::Address("fe80::1234".parse().unwrap()));
        address
            .attributes
            .push(AddressAttribute::Label("unselected-private-label".into()));
        let facts = Facts::from_payload(&NetlinkPayload::InnerMessage(
            RouteNetlinkMessage::NewAddress(address),
        ));
        let mut expected = Expected {
            tun: Some((
                9,
                "unselected-tun-name".into(),
                1280,
                vec!["100.96.0.1".parse().unwrap()],
            )),
            requested_drain: 4,
            drained: 3,
            ..Default::default()
        };
        expected.events.push_back(Event {
            rule: false,
            deleted: false,
            bytes: b"unselected-raw-message".to_vec(),
        });
        facts.reject(&mut expected, "tun_address_changed");
        let first = expected.first_conflict.clone().unwrap();
        assert!(first.len() < 1024);
        assert!(first.contains("RTM_NEWADDR") && first.contains("fe80::1234"));
        assert!(!first.contains("unselected-"));
        facts.reject(&mut expected, "later_conflict");
        assert_eq!(expected.first_conflict.as_deref(), Some(first.as_str()));
    }
}
