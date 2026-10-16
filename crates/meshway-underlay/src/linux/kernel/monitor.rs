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
    collections::{BTreeMap, VecDeque},
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
    pub exit_rules: Vec<Vec<u8>>,
    pub exit_tables: Vec<u32>,
    pub exit_tun: Option<u32>,
    /// Stops the installed exit role. An exit-owned route or rule removed
    /// outside its owner stops that role alone: its owner retires every
    /// projection and cleans up; the Host keeps its own resources.
    pub exit_stop: Option<tokio_util::sync::CancellationToken>,
    pub probe_tun: Option<u32>,
    pub egress_rules: Vec<Vec<u8>>,
    pub egress_table: Option<u32>,
    /// Route events the source-capture owner's own mutations of its table
    /// still owe, by (deleted, route): one per add or delete it issued.
    pub egress_events: BTreeMap<(bool, Vec<u8>), usize>,
    pub tun: Option<(u32, String, usize, Vec<std::net::IpAddr>)>,
    // Observation-only tombstone, kept until the synchronous TUN close's
    // notifications have drained. It grants no capture/ownership exception.
    pub retiring_tun: Option<u32>,
    pub anchor: Option<super::anchor::Record>,
    /// The DNS anchor being created: its name, then its index once known.
    pub anchor_creating: Option<(String, Option<u32>)>,
    pub anchor_deleting: bool,
    pub changing: bool,
    pub wake: Arc<tokio::sync::Notify>,
    pub requested_drain: u64,
    pub drained: u64,
    /// The capture block's own rules (encoded), as its journal records them.
    /// One removed outside its owner is reinstalled, never a conflict.
    pub own_rules: Vec<Vec<u8>>,
    /// External addresses inside a Mesh pool, with their interface label.
    pub overlaps: BTreeMap<(u32, std::net::IpAddr), String>,
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

/// The routing facts a conflict report names: table, output interface and
/// destination prefix length of a route; never addresses or raw payloads.
#[derive(Default)]
struct Observed {
    table: Option<u32>,
    interface: Option<u32>,
    prefix: Option<u8>,
}

impl Observed {
    fn of(payload: &NetlinkPayload<RouteNetlinkMessage>) -> Self {
        match payload {
            NetlinkPayload::InnerMessage(
                RouteNetlinkMessage::NewRoute(route) | RouteNetlinkMessage::DelRoute(route),
            ) => Self {
                table: Some(route_table(route)),
                interface: route
                    .attributes
                    .iter()
                    .find_map(|attribute| match attribute {
                        RouteAttribute::Oif(index) => Some(*index),
                        _ => None,
                    }),
                prefix: Some(route.header.destination_prefix_length),
            },
            NetlinkPayload::InnerMessage(
                RouteNetlinkMessage::NewRule(rule) | RouteNetlinkMessage::DelRule(rule),
            ) => Self {
                table: Some(super::plan::rule_table(rule)),
                ..Self::default()
            },
            _ => Self::default(),
        }
    }
}

impl Expected {
    /// An address message whose pool addresses are all recorded overlaps
    /// of its interface.
    pub fn overlapping(&self, address: &rtnetlink::packet_route::address::AddressMessage) -> bool {
        address
            .attributes
            .iter()
            .filter_map(|a| match a {
                AddressAttribute::Local(ip) | AddressAttribute::Address(ip) => Some(*ip),
                _ => None,
            })
            .filter(|ip| self.pools.iter().any(|pool| pool.contains(ip)))
            .all(|ip| self.overlaps.contains_key(&(address.header.index, ip)))
    }

    pub fn add(&mut self, event: Event) -> Result<()> {
        if self.events.len() >= 1024 {
            return Err(Error::Capacity);
        }
        self.events.push_back(event);
        Ok(())
    }

    /// Count one owed egress-table route event; nothing else is consumed.
    pub fn expect_egress(&mut self, deleted: bool, route: Vec<u8>) {
        *self.egress_events.entry((deleted, route)).or_default() += 1;
    }

    fn egress_event(&mut self, deleted: bool, route: Vec<u8>) -> bool {
        let key = (deleted, route);
        match self.egress_events.get_mut(&key) {
            Some(count) if *count > 1 => *count -= 1,
            Some(_) => {
                self.egress_events.remove(&key);
            }
            None => return false,
        }
        true
    }

    /// An exit-owned rule (also once its input TUN is gone: the kernel then
    /// adds its IIF_DETACHED state flag).
    fn exit_owned_rule(&self, rule: &rtnetlink::packet_route::rule::RuleMessage) -> bool {
        let mut attached = rule.clone();
        attached
            .header
            .flags
            .remove(rtnetlink::packet_route::rule::RuleFlags::IifDetached);
        self.exit_rules.contains(&encode_rule(rule.clone()))
            || self.exit_rules.contains(&encode_rule(attached))
    }

    /// Stop the installed exit role for the removal of one of its objects;
    /// false when no role is installed to take it.
    fn stop_exit(&self, reason: &'static str) -> bool {
        let Some(stop) = &self.exit_stop else {
            return false;
        };
        tracing::warn!(
            reason,
            "Exit-owned kernel object removed outside its owner; stopping the exit role"
        );
        stop.cancel();
        true
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
    if let Err(_error) = monitor(socket, &expected, table, &shared).await {
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
        // Lost or unreadable notifications prove nothing about ownership;
        // a notification read and found foreign does.
        shared.end(match _error {
            Error::Io(_) | Error::Netlink => crate::OwnershipEnd::Unavailable {
                check: "monitor_unavailable",
            },
            _ => crate::OwnershipEnd::Conflict { check: "monitor" },
        });
    }
}

async fn monitor(
    socket: AsyncFd<Socket>,
    expected: &Mutex<Expected>,
    table: u32,
    shared: &Shared,
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
                let observed = Observed::of(&message.payload);
                macro_rules! conflict {
                    ($branch:literal) => {{
                        tracing::warn!(
                            reason = $branch,
                            table = observed.table,
                            interface = observed.interface,
                            prefix = observed.prefix,
                            "Underlay kernel protection conflict"
                        );
                        #[cfg(test)]
                        facts.reject(&mut state, $branch);
                        return Err(Error::Conflict);
                    }};
                }
                let changed = network_changed(&state, &message.payload, table);
                let overlaps = state.overlaps.len();
                let mut repair = false;
                let mut overlapping = Vec::new();
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
                        // A network handing this Node an address inside a
                        // Mesh pool: capture fails closed while it lasts.
                        if !external(&state, address.header.index) {
                            conflict!("capture_address");
                        }
                        let label = address
                            .attributes
                            .iter()
                            .find_map(|a| match a {
                                AddressAttribute::Label(label) => Some(label.clone()),
                                _ => None,
                            })
                            .unwrap_or_else(|| format!("if{}", address.header.index));
                        for ip in address.attributes.iter().filter_map(|a| match a {
                            AddressAttribute::Local(ip) | AddressAttribute::Address(ip) => {
                                Some(*ip)
                            }
                            _ => None,
                        }) {
                            if state.pools.iter().any(|pool| pool.contains(&ip)) {
                                overlapping.push(((address.header.index, ip), label.clone()));
                            }
                        }
                    }
                    // The kernel's own local routes of such an address are
                    // part of that overlap, not a separate conflict.
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewRoute(route))
                        if super::capture::route_conflicts(&state.pools, owned, route)
                            && !overlap_route(&state, route) =>
                    {
                        conflict!("capture_route");
                    }
                    _ => {}
                }
                state.overlaps.extend(overlapping);
                match &message.payload {
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelAddress(address)) => {
                        let index = address.header.index;
                        for ip in address.attributes.iter().filter_map(|a| match a {
                            AddressAttribute::Local(ip) | AddressAttribute::Address(ip) => {
                                Some(*ip)
                            }
                            _ => None,
                        }) {
                            state.overlaps.remove(&(index, ip));
                        }
                    }
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelLink(link)) => {
                        let index = link.header.index;
                        state.overlaps.retain(|(owner, _), _| *owner != index);
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
                            if !state.anchor_deleting && state.egress_table != Some(route_table(route)) && super::anchor::route_conflicts(anchor.identity.address, None, route) => conflict!("anchor_route_deleted"),
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
                        let exit = state.exit_owned_rule(&rule);
                        let bytes = encode_rule(rule);
                        let own = state.own_rules.contains(&bytes);
                        if state
                            .consume(Event {
                                rule: true,
                                deleted: true,
                                bytes,
                            })
                            .is_err()
                            && !(exit && state.stop_exit("unexpected_del_rule"))
                        {
                            // A network manager removing rules it does not
                            // own: the capture owner reinstalls them.
                            if !own {
                                conflict!("unexpected_del_rule");
                            }
                            repair = true;
                        }
                    }
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewRoute(route))
                        if state.egress_table == Some(route_table(&route))
                            && state.egress_event(false, encode_route(route.clone())) => {}
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewRoute(route))
                        if route_table(&route) == table
                            || state.egress_table == Some(route_table(&route))
                            || state.exit_tables.contains(&route_table(&route)) =>
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
                        if state.egress_table == Some(route_table(&route))
                            && state.egress_event(true, encode_route(route.clone())) => {}
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelRoute(route))
                        if route_table(&route) == table
                            || state.egress_table == Some(route_table(&route))
                            || state.exit_tables.contains(&route_table(&route)) =>
                    {
                        let exit = state.exit_tables.contains(&route_table(&route));
                        let own = route_table(&route) == table;
                        if state
                            .consume(Event {
                                rule: false,
                                deleted: true,
                                bytes: encode_route(route),
                            })
                            .is_err()
                            && !(exit && state.stop_exit("unexpected_del_route"))
                        {
                            if !own {
                                conflict!("unexpected_del_route");
                            }
                            repair = true;
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
                let overlap = (state.overlaps.len() != overlaps).then(|| {
                    state
                        .overlaps
                        .iter()
                        .next()
                        .map(|((_, address), interface)| crate::PoolOverlap {
                            interface: interface.clone(),
                            address: *address,
                        })
                });
                drop(state);
                if let Some(overlap) = overlap {
                    if let Some(found) = &overlap {
                        tracing::warn!(interface = %found.interface, address = %found.address,
                            "External address inside a Mesh pool; capture fails closed until it leaves");
                        shared.suspend()?;
                    }
                    shared.network.overlap.send_replace(overlap);
                }
                if repair {
                    tracing::warn!("Owned capture object removed outside its owner; reinstalling");
                    shared
                        .network
                        .repairs
                        .send_modify(|count| *count = count.wrapping_add(1));
                }
                if changed {
                    shared
                        .network
                        .dirty
                        .send_modify(|count| *count = count.wrapping_add(1));
                }
            }
            // Every received datagram is finite; cooperate even during event floods.
            tokio::task::yield_now().await;
        }
    }
}

/// An interface that is neither loopback nor one of this owner's own (Mesh,
/// probe and exit TUNs, the DNS anchor, including while created or retired).
pub(super) fn external(state: &Expected, index: u32) -> bool {
    index != 1
        && state.probe_tun != Some(index)
        && state.exit_tun != Some(index)
        && state.retiring_tun != Some(index)
        && state
            .tun
            .as_ref()
            .is_none_or(|(owned, _, _, _)| *owned != index)
        && state
            .anchor
            .as_ref()
            .is_none_or(|anchor| anchor.identity.index != index)
        && state
            .anchor_creating
            .as_ref()
            .is_none_or(|(_, creating)| *creating != Some(index))
}

/// A local-table route of an external interface that holds an address
/// inside a Mesh pool: the kernel's own consequence of that address.
pub(super) fn overlap_route(
    state: &Expected,
    route: &rtnetlink::packet_route::route::RouteMessage,
) -> bool {
    route_table(route) == 255
        && route.attributes.iter().any(|attribute| {
            matches!(attribute, RouteAttribute::Oif(index)
                if state.overlaps.keys().any(|(owner, _)| owner == index))
        })
}

/// Marks an external notification for classification: links that are up,
/// addresses and routes of external interfaces, in any table but this
/// owner's own (capture, source capture, exit) and local.
fn network_changed(
    state: &Expected,
    payload: &NetlinkPayload<RouteNetlinkMessage>,
    table: u32,
) -> bool {
    let external = |index| external(state, index);
    match payload {
        // A link that is not up carries no path: creating or deleting one
        // (a VPN daemon's idle ppp0, say) changes no route to anything. An up
        // link going down also deletes its addresses and routes, reported below.
        NetlinkPayload::InnerMessage(
            RouteNetlinkMessage::NewLink(link) | RouteNetlinkMessage::DelLink(link),
        ) => {
            link.header.flags.contains(LinkFlags::Up)
                && external(link.header.index)
                && !state.anchor_creating.as_ref().is_some_and(|(name, _)| {
                    link.attributes
                        .iter()
                        .any(|a| matches!(a, LinkAttribute::IfName(v) if v == name))
                })
        }
        NetlinkPayload::InnerMessage(
            RouteNetlinkMessage::NewAddress(address) | RouteNetlinkMessage::DelAddress(address),
        ) => external(address.header.index),
        NetlinkPayload::InnerMessage(
            RouteNetlinkMessage::NewRoute(route) | RouteNetlinkMessage::DelRoute(route),
        ) => {
            let owner = route_table(route);
            owner != table
                && owner != 255
                && state.egress_table != Some(owner)
                && !state.exit_tables.contains(&owner)
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

    #[test]
    fn owed_capture_table_events_are_consumed_once_each() {
        let mut expected = Expected::default();
        expected.expect_egress(false, vec![1]);
        expected.expect_egress(false, vec![1]);
        expected.expect_egress(true, vec![1]);
        assert!(!expected.egress_event(true, vec![2]));
        assert!(expected.egress_event(false, vec![1]));
        assert!(expected.egress_event(false, vec![1]));
        assert!(!expected.egress_event(false, vec![1]));
        assert!(expected.egress_event(true, vec![1]));
        assert!(expected.egress_events.is_empty());
    }

    #[test]
    fn a_dns_anchor_being_created_is_not_an_external_network_change() {
        use rtnetlink::packet_route::link::LinkMessage;
        let state = Expected {
            anchor_creating: Some(("mwd0123456789".into(), None)),
            ..Default::default()
        };
        let link = |index, name: &str| {
            let mut link = LinkMessage::default();
            link.header.index = index;
            link.header.flags = LinkFlags::Up;
            link.attributes.push(LinkAttribute::IfName(name.into()));
            NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewLink(link))
        };
        let address = |index| {
            let mut address = AddressMessage::default();
            address.header.index = index;
            NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewAddress(address))
        };
        assert!(!network_changed(&state, &link(7, "mwd0123456789"), 0));
        assert!(network_changed(&state, &address(7), 0));
        let state = Expected {
            anchor_creating: Some(("mwd0123456789".into(), Some(7))),
            ..Default::default()
        };
        assert!(!network_changed(&state, &address(7), 0));
        assert!(!network_changed(&state, &link(7, "mwd0123456789"), 0));
        assert!(network_changed(&state, &link(8, "eth1"), 0));
        assert!(network_changed(&state, &address(8), 0));
    }

    #[test]
    fn a_link_that_is_not_up_is_not_a_network_change() {
        use rtnetlink::packet_route::link::LinkMessage;
        let mut idle = LinkMessage::default();
        idle.header.index = 9;
        idle.attributes.push(LinkAttribute::IfName("ppp0".into()));
        let state = Expected::default();
        for message in [
            RouteNetlinkMessage::NewLink(idle.clone()),
            RouteNetlinkMessage::DelLink(idle.clone()),
        ] {
            assert!(!network_changed(
                &state,
                &NetlinkPayload::InnerMessage(message),
                0
            ));
        }
        idle.header.flags = LinkFlags::Up;
        assert!(network_changed(
            &state,
            &NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelLink(idle)),
            0
        ));
    }
}
