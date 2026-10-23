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

mod classify;
mod decode;
#[cfg(test)]
mod tests;

use classify::network_changed;
pub(super) use classify::{external, overlap_route, pool_addresses};
#[cfg(test)]
use decode::Facts;
use decode::Observed;

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

    /// The overlap capture reports while one lasts.
    pub fn overlap(&self) -> Option<crate::PoolOverlap> {
        self.overlaps
            .iter()
            .next()
            .map(|((_, address), interface)| crate::PoolOverlap {
                interface: interface.clone(),
                address: *address,
            })
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
    // RUSTSEC-2026-0386: the AsyncFd owns this fd for life; nothing dups or closes it.
    #[allow(deprecated)]
    let socket = AsyncFd::new(socket)?;
    Ok(socket)
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
        shared.end(ended(&_error));
    }
}

/// Lost, truncated or unreadable notifications, and the monitor's own
/// failure, prove nothing about ownership; a notification read completely
/// and found foreign does ([`super::ended`]).
fn ended(error: &Error) -> crate::OwnershipEnd {
    super::ended("monitor", error)
}

/// A received datagram the monitor can decode: neither empty nor filling the
/// whole buffer (a notification the kernel truncated to fit).
fn complete(size: usize, capacity: usize) -> Result<()> {
    if size == 0 || size >= capacity {
        return Err(Error::Netlink);
    }
    Ok(())
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
            let decodable = complete(size, buffer.len());
            #[cfg(test)]
            if decodable.is_err()
                && let Ok(mut state) = expected.lock()
            {
                Facts {
                    kind: "DATAGRAM",
                    ..Facts::default()
                }
                .reject(&mut state, "invalid_datagram");
            }
            decodable?;
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
                        overlapping.extend(pool_addresses(&state, address));
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
                            let flags = super::network::address_flags(&address);
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
                let overlap = (state.overlaps.len() != overlaps).then(|| state.overlap());
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
