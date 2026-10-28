//! DNS-service and resolver decisions on the installed netmap. These pure
//! checks are not I/O grants; callers retain the original owner-issued guard.
use meshway_configuration::{
    DnsTransport,
    netmap::{DnsPlan, DnsService, NodeNetmap, ResolverAction},
};
use meshway_model::NodeId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolverRouteError {
    Missing,
    Binding,
}

pub fn netmap_resolver(
    netmap: &NodeNetmap,
    server_key: &str,
) -> Result<ResolverAction, ResolverRouteError> {
    if !netmap.self_.enabled
        || !netmap
            .projection
            .configuration
            .dns
            .servers
            .contains_key(server_key)
    {
        return Err(ResolverRouteError::Missing);
    }
    netmap
        .dns
        .resolvers
        .get(server_key)
        .copied()
        .ok_or(ResolverRouteError::Missing)
}

/// [`crate::compile::resolver_plan_gaps`] of the installed netmap: its DNS
/// settings, plan and resolver decisions.
pub fn netmap_resolver_plan_gaps(netmap: &NodeNetmap) -> Vec<crate::compile::ResolverPlanGap> {
    crate::compile::resolver_plan_gaps(
        &netmap.projection.configuration.dns,
        matches!(
            netmap.dns.egress,
            DnsPlan::ViaExit { .. } | DnsPlan::Unavailable { .. }
        ),
        Some(&netmap.dns.resolvers),
    )
}

/// Source/path must come from authenticated ingress, never a socket tuple.
pub fn netmap_dns_service(
    netmap: &meshway_configuration::netmap::NodeNetmap,
    service: &DnsService,
    source: NodeId,
    exit: NodeId,
    path: &[NodeId],
    transport: DnsTransport,
) -> Result<(), ResolverRouteError> {
    let published = match &netmap.dns.egress {
        DnsPlan::ViaExit { service: own } => {
            netmap.node_id == source
                && own.as_ref() == service
                && netmap
                    .egress
                    .as_ref()
                    .is_some_and(|plan| plan.generation == service.generation)
                && netmap.dns.profile == service.profile
                && netmap.self_.addresses == service.source_addresses
                && netmap
                    .projection
                    .configuration
                    .nodes
                    .get(&exit)
                    .is_some_and(|peer| {
                        peer.addresses.ipv4.map(std::net::IpAddr::V4) == Some(service.address.0)
                    })
        }
        DnsPlan::ServeExitResolver { services } => {
            netmap.node_id == exit
                && netmap.self_.exit_enabled
                && netmap.exit_role.as_ref() == Some(&service.role)
                && netmap.self_.addresses.ipv4.map(std::net::IpAddr::V4) == Some(service.address.0)
                && netmap
                    .projection
                    .configuration
                    .nodes
                    .get(&source)
                    .is_some_and(|peer| peer.addresses == service.source_addresses)
                && services.contains(service)
        }
        _ => false,
    };
    if !netmap.self_.enabled
        || !published
        || source != service.source
        || exit != service.exit
        || source == exit
        || path != service.path.nodes
        || path.first() != Some(&source)
        || path.last() != Some(&exit)
        || !(2..=crate::compile::MAX_HOPS).contains(&path.len())
        || path.iter().collect::<std::collections::BTreeSet<_>>().len() != path.len()
        || !service.address.0.is_ipv4()
        || service.source_addresses.ipv4.is_none()
        || !service.transports.contains(&transport)
        || service.transports.len() > 2
        || service.max_pending == 0
        || service.max_pending > 128
        || service.timeout_ms == 0
        || service.timeout_ms > 5_000
    {
        return Err(ResolverRouteError::Binding);
    }
    Ok(())
}
