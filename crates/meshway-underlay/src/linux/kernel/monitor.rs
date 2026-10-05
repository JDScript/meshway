//! A fixed receive buffer avoids netlink-proto's unbounded multicast queue.
use rtnetlink::{
    MulticastGroup,
    packet_core::{NetlinkMessage, NetlinkPayload},
    packet_route::{
        RouteNetlinkMessage,
        address::AddressAttribute,
        link::{LinkAttribute, LinkFlags},
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
    pub events: VecDeque<Event>,
    pub tun: Option<(u32, String, usize, Vec<std::net::IpAddr>)>,
    pub anchor: Option<super::anchor::Record>,
    pub anchor_deleting: bool,
    pub changing: bool,
    pub wake: Arc<tokio::sync::Notify>,
    pub requested_drain: u64,
    pub drained: u64,
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
    if monitor(socket, &expected, table).await.is_err() {
        shared.conflict.cancel();
    }
}

async fn monitor(socket: AsyncFd<Socket>, expected: &Mutex<Expected>, table: u32) -> Result<()> {
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
            if size == 0 || size >= buffer.len() || source.port_number() != 0 {
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
                // Anchor ownership is independent of TUN installation's changing
                // window. Never suppress its checks while a lease refresh runs.
                if let Some(anchor) = &state.anchor {
                    match &message.payload {
                        NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewLink(link))
                            if link.header.index != anchor.identity.index && link.attributes.iter().any(|a| matches!(a, LinkAttribute::IfAlias(v) if v == &anchor.alias)) => return Err(Error::Conflict),
                        NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewAddress(address))
                            if address.header.index != anchor.identity.index && address.attributes.iter().any(|a| matches!(a, AddressAttribute::Address(std::net::IpAddr::V4(ip)) | AddressAttribute::Local(std::net::IpAddr::V4(ip)) if *ip == anchor.identity.address)) => return Err(Error::Conflict),
                        NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelRoute(route))
                            if !state.anchor_deleting && super::anchor::route_conflicts(anchor.identity.address, None, route) => return Err(Error::Conflict),
                        NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewLink(link))
                            if link.header.index == anchor.identity.index
                                && !(super::anchor::link_matches(anchor, link)
                                    || (state.anchor_deleting && super::anchor::deleting_link_matches(anchor, link))) =>
                        {
                            return Err(Error::Conflict);
                        }
                        NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelLink(link))
                            if link.header.index == anchor.identity.index
                                && !state.anchor_deleting =>
                        {
                            return Err(Error::Conflict);
                        }
                        NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewAddress(address))
                            if address.header.index == anchor.identity.index
                                && !super::anchor::address_matches(anchor, address) =>
                        {
                            return Err(Error::Conflict);
                        }
                        NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelAddress(address))
                            if address.header.index == anchor.identity.index
                                && !state.anchor_deleting =>
                        {
                            return Err(Error::Conflict);
                        }
                        _ => {}
                    }
                }
                match message.payload {
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewRule(rule)) => state
                        .consume(Event {
                            rule: true,
                            deleted: false,
                            bytes: encode_rule(rule),
                        })?,
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelRule(rule)) => state
                        .consume(Event {
                            rule: true,
                            deleted: true,
                            bytes: encode_rule(rule),
                        })?,
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewRoute(route))
                        if route_table(&route) == table =>
                    {
                        state.consume(Event {
                            rule: false,
                            deleted: false,
                            bytes: encode_route(route),
                        })?
                    }
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelRoute(route))
                        if route_table(&route) == table =>
                    {
                        state.consume(Event {
                            rule: false,
                            deleted: true,
                            bytes: encode_route(route),
                        })?
                    }
                    NetlinkPayload::InnerMessage(RouteNetlinkMessage::DelLink(link)) => {
                        if state
                            .tun
                            .as_ref()
                            .is_some_and(|(index, _, _, _)| *index == link.header.index)
                        {
                            return Err(Error::Conflict);
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
                            return Err(Error::Conflict);
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
                            return Err(Error::Conflict);
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
                            return Err(Error::Conflict);
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
                            return Err(Error::Conflict);
                        }
                    }
                    NetlinkPayload::InnerMessage(_) | NetlinkPayload::Noop => {}
                    _ => return Err(Error::Netlink),
                }
                offset += (length + 3) & !3;
            }
            // Every received datagram is finite; cooperate even during event floods.
            tokio::task::yield_now().await;
        }
    }
}
