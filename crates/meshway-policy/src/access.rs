//! Identity-level Mesh Access. Execution availability is checked by the caller.
use crate::SubjectContext;
use meshway_configuration::{
    NetworkConfiguration, execution::BusinessProtocol, netmap::ServiceSet,
};
use meshway_model::NodeId;
use meshway_transport_spec::Port;
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

    fn allows(&self, protocol: BusinessProtocol, port: Option<Port>) -> bool {
        match (self, protocol, port) {
            (Self::Any | Self::IcmpEcho, BusinessProtocol::IcmpEcho, None) => true,
            (Self::Any, BusinessProtocol::Tcp | BusinessProtocol::Udp, Some(_)) => true,
            (Self::Tcp(range), BusinessProtocol::Tcp, Some(port))
            | (Self::Udp(range), BusinessProtocol::Udp, Some(port)) => range.contains(&port.get()),
            _ => false,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum TrafficQuery {
    Any,
    Exact {
        protocol: BusinessProtocol,
        port: Option<Port>,
    },
}

impl TrafficQuery {
    pub(crate) fn matches(self, entries: &[String]) -> bool {
        entries
            .iter()
            .filter_map(|entry| TrafficGrant::parse(entry))
            .any(|grant| match self {
                Self::Any => true,
                Self::Exact { protocol, port } => grant.allows(protocol, port),
            })
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
    allows_mesh(configuration, subjects, source, target, TrafficQuery::Any)
}

/// Union of every service any rule grants from `source` to `target`; equal
/// to `allows_mesh` for each exact protocol/port. Rule order is irrelevant.
pub(crate) fn mesh_services(
    configuration: &NetworkConfiguration,
    subjects: &SubjectContext,
    source: NodeId,
    target: NodeId,
) -> ServiceSet {
    if !subjects.node_is_active(configuration, source)
        || !subjects.node_is_active(configuration, target)
    {
        return ServiceSet::default();
    }
    let matches = |selectors: &[String], node| {
        selectors
            .iter()
            .any(|selector| subjects.matches(configuration, selector, node))
    };
    ServiceSet::from_grants(
        configuration
            .access
            .iter()
            .filter(|rule| {
                (matches(&rule.src, source) && matches(&rule.dst, target))
                    || (rule.bidirectional
                        && matches(&rule.dst, source)
                        && matches(&rule.src, target))
            })
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

pub(crate) fn allows_mesh(
    configuration: &NetworkConfiguration,
    subjects: &SubjectContext,
    source: NodeId,
    target: NodeId,
    traffic: TrafficQuery,
) -> bool {
    // Wildcard and explicit node selectors alone do not check active identity.
    if !subjects.node_is_active(configuration, source)
        || !subjects.node_is_active(configuration, target)
    {
        return false;
    }
    let matches = |selectors: &[String], node| {
        selectors
            .iter()
            .any(|selector| subjects.matches(configuration, selector, node))
    };
    configuration.access.iter().any(|rule| {
        traffic.matches(&rule.ip)
            && ((matches(&rule.src, source) && matches(&rule.dst, target))
                || (rule.bidirectional && matches(&rule.dst, source) && matches(&rule.src, target)))
    })
}
