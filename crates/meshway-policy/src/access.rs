//! Identity-level Mesh Access. Execution availability is checked by the caller.
use crate::SubjectContext;
use meshway_configuration::{
    NetworkConfiguration, execution::BusinessProtocol, netmap::ServiceSet,
};
use meshway_model::NodeId;
use std::ops::RangeInclusive;

pub(crate) enum TrafficGrant {
    Any,
    Tcp(RangeInclusive<u16>),
    Udp(RangeInclusive<u16>),
    IcmpEcho,
}

impl TrafficGrant {
    /// Every accepted token describes a nonempty set of supported traffic.
    /// Configuration validation and both concrete/existential queries share it.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        if value == "*" {
            return Some(Self::Any);
        }
        let (protocol, ports) = value.split_once(':')?;
        if protocol == "icmp" {
            return (ports == "*").then_some(Self::IcmpEcho);
        }
        fn port(value: &str) -> Option<u16> {
            value
                .parse()
                .ok()
                .filter(|port: &u16| *port > 0 && port.to_string() == value)
        }
        let range = if ports == "*" {
            1..=u16::MAX
        } else if let Some((start, end)) = ports.split_once('-') {
            let (start, end) = (port(start)?, port(end)?);
            if start > end {
                return None;
            }
            start..=end
        } else {
            let port = port(ports)?;
            port..=port
        };
        match protocol {
            "tcp" => Some(Self::Tcp(range)),
            "udp" => Some(Self::Udp(range)),
            _ => None,
        }
    }

    /// Expanded service entries of one token, as consumed by netmap grants.
    pub(crate) fn services(&self) -> Vec<(BusinessProtocol, Option<(u16, u16)>)> {
        match self {
            Self::Any => vec![
                (BusinessProtocol::Tcp, Some((1, u16::MAX))),
                (BusinessProtocol::Udp, Some((1, u16::MAX))),
                (BusinessProtocol::IcmpEcho, None),
            ],
            Self::Tcp(range) => vec![(BusinessProtocol::Tcp, Some((*range.start(), *range.end())))],
            Self::Udp(range) => vec![(BusinessProtocol::Udp, Some((*range.start(), *range.end())))],
            Self::IcmpEcho => vec![(BusinessProtocol::IcmpEcho, None)],
        }
    }
}

/// Whether current Mesh identities share at least one explicitly granted
/// business protocol/port. Does not require enabled execution, keys, instances,
/// transport capabilities, a feasible route, or an existing demand.
/// The caller supplies current trusted bindings and registry eligibility.
pub fn mesh_access_allows_any(
    configuration: &NetworkConfiguration,
    subjects: &SubjectContext,
    source: NodeId,
    target: NodeId,
) -> bool {
    mesh_grants(configuration, subjects, source, target)
        .any(|rule| token_services(&rule.ip).next().is_some())
}

/// Union of every service any rule grants from `source` to `target`.
/// Rule order is irrelevant.
pub(crate) fn mesh_services(
    configuration: &NetworkConfiguration,
    subjects: &SubjectContext,
    source: NodeId,
    target: NodeId,
) -> ServiceSet {
    ServiceSet::from_grants(
        mesh_grants(configuration, subjects, source, target)
            .flat_map(|rule| token_services(&rule.ip)),
    )
}

pub(crate) fn token_services(
    tokens: &[String],
) -> impl Iterator<Item = (BusinessProtocol, Option<(u16, u16)>)> + '_ {
    tokens
        .iter()
        .filter_map(|token| TrafficGrant::parse(token))
        .flat_map(|grant| grant.services())
}

fn mesh_grants<'a>(
    configuration: &'a NetworkConfiguration,
    subjects: &'a SubjectContext,
    source: NodeId,
    target: NodeId,
) -> impl Iterator<Item = &'a meshway_configuration::AccessRule> {
    // Wildcard and explicit node selectors alone do not check active identity.
    let active = subjects.node_is_active(configuration, source)
        && subjects.node_is_active(configuration, target);
    let matches = move |selectors: &[String], node| {
        selectors
            .iter()
            .any(|selector| subjects.matches(configuration, selector, node))
    };
    configuration.access.iter().filter(move |rule| {
        active
            && ((matches(&rule.src, source) && matches(&rule.dst, target))
                || (rule.bidirectional && matches(&rule.dst, source) && matches(&rule.src, target)))
    })
}
