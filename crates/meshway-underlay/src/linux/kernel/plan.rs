use ipnet::IpNet;
use rtnetlink::{
    RouteMessageBuilder,
    packet_core::{NetlinkMessage, NetlinkPayload, Nla},
    packet_route::{
        AddressFamily, RouteNetlinkMessage,
        route::{
            RouteAttribute, RouteMessage, RoutePreference, RouteProtocol, RouteScope, RouteType,
        },
        rule::{RuleAction, RuleAttribute, RuleMessage},
    },
};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::super::protection::{Mark, Selector};
use crate::{Error, Result};

pub(super) const PROTOCOL: u8 = 242;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RuleSpec {
    pub ipv6: bool,
    pub priority: u32,
    pub table: u32,
    pub kind: RuleKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(super) enum RuleKind {
    Bypass { mark: Mark, end: u32 },
    Lookup { pool: IpNet },
    Sink { pool: IpNet },
    End,
}

impl RuleSpec {
    pub fn message(&self) -> RuleMessage {
        let mut message = RuleMessage::default();
        message.header.family = if self.ipv6 {
            AddressFamily::Inet6
        } else {
            AddressFamily::Inet
        };
        // A nonzero table is an additional deletion discriminator even for the
        // non-lookup actions; it does not redirect those actions to that table.
        message.attributes = vec![
            RuleAttribute::Priority(self.priority),
            RuleAttribute::Table(self.table),
            RuleAttribute::Protocol(RouteProtocol::from(PROTOCOL)),
        ];
        match self.kind {
            RuleKind::Bypass { mark, end } => {
                message.header.action = RuleAction::Goto;
                message.attributes.extend([
                    RuleAttribute::FwMark(mark.value),
                    RuleAttribute::FwMask(mark.mask),
                    RuleAttribute::Goto(end),
                ]);
            }
            RuleKind::Lookup { pool } | RuleKind::Sink { pool } => {
                message.header.action = if matches!(self.kind, RuleKind::Lookup { .. }) {
                    RuleAction::ToTable
                } else {
                    RuleAction::Unreachable
                };
                message.header.dst_len = pool.prefix_len();
                message
                    .attributes
                    .push(RuleAttribute::Destination(pool.network()));
            }
            RuleKind::End => message.header.action = RuleAction::Nop,
        }
        message
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RouteSpec {
    pub pool: IpNet,
    pub table: u32,
    pub ifindex: u32,
    pub source: IpAddr,
}

impl RouteSpec {
    pub fn message(&self) -> Result<RouteMessage> {
        let message = match (self.pool, self.source) {
            (IpNet::V4(pool), IpAddr::V4(source)) => RouteMessageBuilder::<Ipv4Addr>::new()
                .destination_prefix(pool.network(), pool.prefix_len())
                .pref_source(source)
                .output_interface(self.ifindex)
                .table_id(self.table)
                .priority(100)
                .protocol(PROTOCOL.into())
                .scope(RouteScope::Link)
                .kind(RouteType::Unicast)
                .build(),
            (IpNet::V6(pool), IpAddr::V6(source)) => RouteMessageBuilder::<Ipv6Addr>::new()
                .destination_prefix(pool.network(), pool.prefix_len())
                .pref_source(source)
                .output_interface(self.ifindex)
                .table_id(self.table)
                .priority(100)
                .protocol(PROTOCOL.into())
                .scope(RouteScope::Universe)
                .kind(RouteType::Unicast)
                .build(),
            _ => return Err(Error::Target),
        };
        Ok(message)
    }
}

pub(super) fn rule_table(rule: &RuleMessage) -> u32 {
    rule.attributes
        .iter()
        .find_map(|a| {
            if let RuleAttribute::Table(v) = a {
                Some(*v)
            } else {
                None
            }
        })
        .unwrap_or(rule.header.table.into())
}

pub(super) fn route_table(route: &RouteMessage) -> u32 {
    route
        .attributes
        .iter()
        .find_map(|a| {
            if let RouteAttribute::Table(v) = a {
                Some(*v)
            } else {
                None
            }
        })
        .unwrap_or(route.header.table.into())
}

pub(super) fn priority(rule: &RuleMessage) -> u32 {
    rule.attributes
        .iter()
        .find_map(|a| {
            if let RuleAttribute::Priority(v) = a {
                Some(*v)
            } else {
                None
            }
        })
        .unwrap_or(0)
}

pub(super) fn selectors(rules: &[RuleMessage]) -> Result<Vec<Selector>> {
    rules
        .iter()
        .map(|rule| {
            if rule
                .attributes
                .iter()
                .any(|a| matches!(a, RuleAttribute::Other(_)))
            {
                return Err(Error::Conflict);
            }
            let value = rule
                .attributes
                .iter()
                .find_map(|a| {
                    if let RuleAttribute::FwMark(v) = a {
                        Some(*v)
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            let mask = rule
                .attributes
                .iter()
                .find_map(|a| {
                    if let RuleAttribute::FwMask(v) = a {
                        Some(*v)
                    } else {
                        None
                    }
                })
                .unwrap_or(if value == 0 { 0 } else { u32::MAX });
            Ok(Selector { value, mask })
        })
        .collect()
}

pub(super) fn encode_rule(mut message: RuleMessage) -> Vec<u8> {
    let table = rule_table(&message);
    message.header.table = 0;
    message.attributes.retain(|a| {
        !matches!(
            a,
            RuleAttribute::Table(_)
                | RuleAttribute::SuppressPrefixLen(u32::MAX)
                | RuleAttribute::SuppressIfGroup(u32::MAX)
        )
    });
    message.attributes.push(RuleAttribute::Table(table));
    message.attributes.sort_by_key(Nla::kind);
    encode(RouteNetlinkMessage::NewRule(message))
}

pub(super) fn encode_route(mut message: RouteMessage) -> Vec<u8> {
    let table = route_table(&message);
    message.header.table = 0;
    message.attributes.retain(|a| {
        !matches!(
            a,
            RouteAttribute::Table(_)
                | RouteAttribute::CacheInfo(_)
                | RouteAttribute::Preference(RoutePreference::Medium)
        )
    });
    message.attributes.push(RouteAttribute::Table(table));
    message.attributes.sort_by_key(Nla::kind);
    encode(RouteNetlinkMessage::NewRoute(message))
}

fn encode(message: RouteNetlinkMessage) -> Vec<u8> {
    let mut packet = NetlinkMessage::from(message);
    packet.finalize();
    let mut bytes = vec![0; packet.buffer_len()];
    packet.serialize(&mut bytes);
    bytes
}

pub(super) fn decode_rule(bytes: &[u8]) -> Result<RuleMessage> {
    match NetlinkMessage::<RouteNetlinkMessage>::deserialize(bytes)
        .map_err(|_| Error::Journal)?
        .payload
    {
        NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewRule(message)) => Ok(message),
        _ => Err(Error::Journal),
    }
}

pub(super) fn decode_route(bytes: &[u8]) -> Result<RouteMessage> {
    match NetlinkMessage::<RouteNetlinkMessage>::deserialize(bytes)
        .map_err(|_| Error::Journal)?
        .payload
    {
        NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewRoute(message)) => Ok(message),
        _ => Err(Error::Journal),
    }
}

pub(super) fn pools(
    published: &meshway_configuration::AddressPools,
    addresses: &meshway_configuration::NodeAddresses,
    limit: usize,
) -> Result<Vec<IpNet>> {
    let mut pools = Vec::new();
    for (ipv6, values) in [(false, &published.ipv4), (true, &published.ipv6)] {
        for text in values {
            let pool = text.parse::<IpNet>().map_err(|_| Error::Target)?;
            if pool.addr() != pool.network()
                || pool.addr().is_ipv6() != ipv6
                || pool.prefix_len() == 0
                || pools.contains(&pool)
            {
                return Err(Error::Target);
            }
            pools.push(pool);
            if pools.len() > limit {
                return Err(Error::Capacity);
            }
        }
    }
    for address in [
        addresses.ipv4.map(IpAddr::V4),
        addresses.ipv6.map(IpAddr::V6),
    ]
    .into_iter()
    .flatten()
    {
        if !pools.iter().any(|pool| pool.contains(&address)) {
            return Err(Error::Target);
        }
    }
    Ok(pools)
}

/// Positive capture is limited to locally assigned families. Every published
/// pool still has a negative boundary; absence of a local address supplies no
/// preferred underlay source and no origination/delivery authority.
pub(super) fn positive_routes(
    pools: &[IpNet],
    addresses: &meshway_configuration::NodeAddresses,
    table: u32,
    ifindex: u32,
) -> Vec<RouteSpec> {
    pools
        .iter()
        .filter_map(|pool| {
            let source = match pool {
                IpNet::V4(_) => addresses.ipv4.map(IpAddr::V4),
                IpNet::V6(_) => addresses.ipv6.map(IpAddr::V6),
            }?;
            Some(RouteSpec {
                pool: *pool,
                table,
                ifindex,
                source,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_roundtrip_retains_exact_scope_and_nonzero_deletion_discriminators() {
        for kind in [
            RuleKind::Bypass {
                mark: Mark {
                    value: 0x100,
                    mask: 0x100,
                },
                end: 100,
            },
            RuleKind::Lookup {
                pool: "100.96.0.0/11".parse().unwrap(),
            },
            RuleKind::Sink {
                pool: "100.96.0.0/11".parse().unwrap(),
            },
            RuleKind::End,
        ] {
            let spec = RuleSpec {
                ipv6: false,
                priority: 10,
                table: 5_000_000,
                kind,
            };
            let bytes = encode_rule(spec.message());
            let decoded = decode_rule(&bytes).unwrap();
            assert_eq!(priority(&decoded), 10);
            assert_eq!(rule_table(&decoded), 5_000_000);
            assert_eq!(encode_rule(decoded), bytes);
        }
    }
}
