//! Decoding a notification into the facts a conflict report names.
use super::*;

// Kept inside the kernel-test binary: one fixed-field record on the first
// rejection, without installing a subscriber or logging raw netlink payloads.
#[cfg(test)]
#[derive(Default)]
pub(super) struct Facts {
    pub(super) kind: &'static str,
    pub(super) index: Option<u32>,
    pub(super) address: Option<std::net::IpAddr>,
    pub(super) local: Option<std::net::IpAddr>,
    pub(super) prefix: Option<u8>,
    pub(super) flags: Option<u32>,
}
#[cfg(test)]
impl Facts {
    pub(super) fn from_payload(payload: &NetlinkPayload<RouteNetlinkMessage>) -> Self {
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
                flags: Some(super::super::network::address_flags(address)),
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

    pub(super) fn reject(&self, state: &mut Expected, branch: &'static str) {
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
pub(super) struct Observed {
    pub(super) table: Option<u32>,
    pub(super) interface: Option<u32>,
    pub(super) prefix: Option<u8>,
}

impl Observed {
    pub(super) fn of(payload: &NetlinkPayload<RouteNetlinkMessage>) -> Self {
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
                table: Some(super::super::plan::rule_table(rule)),
                ..Self::default()
            },
            _ => Self::default(),
        }
    }
}
