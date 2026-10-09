//! Exact published local resolver bindings. This pure check is not an I/O grant.
use meshway_configuration::{
    DnsTransport,
    execution::{
        BusinessProtocol, BusinessTarget, DemandDecision, LocalEgressAction, LocalEgressPermit,
        NodeConfiguration,
    },
};
use meshway_model::RequestId;

/// Pure validation only. The observed source/path must come from authenticated
/// data-plane ingress, never from remoteAddr or request-supplied metadata.
pub fn dns_service_permit(
    configuration: &NodeConfiguration,
    permit: &meshway_configuration::execution::DnsServicePermit,
    source: meshway_model::NodeId,
    exit: meshway_model::NodeId,
    path: &[meshway_model::NodeId],
    transport: DnsTransport,
    now_ms: u64,
) -> Result<(), ResolverRouteError> {
    use meshway_configuration::execution::DnsEgress;
    if permit.expires_at_ms <= now_ms {
        return Err(ResolverRouteError::Expired);
    }
    let published = match &configuration.dns.egress {
        DnsEgress::ViaExit {
            exit,
            service,
            generation,
        } => {
            configuration.node_id == source
                && *exit == permit.exit
                && service.as_ref() == permit
                && *generation == permit.generation
                && configuration.egress_capture.generation == *generation
                && configuration.dns.profile == permit.profile
                && configuration.addresses == permit.source_addresses
                && configuration.peers.get(exit).is_some_and(|peer| {
                    peer.addresses.ipv4.map(std::net::IpAddr::V4) == Some(permit.address.0)
                })
        }
        DnsEgress::ServeExitResolver { permit: permits } => {
            configuration.node_id == exit
                && configuration.exit_enabled
                && configuration.exit_role.as_ref() == Some(&permit.role)
                && configuration.addresses.ipv4.map(std::net::IpAddr::V4) == Some(permit.address.0)
                && configuration
                    .peers
                    .get(&source)
                    .is_some_and(|peer| peer.addresses == permit.source_addresses)
                && permits.contains(permit)
        }
        _ => false,
    };
    if !configuration.enabled
        || !published
        || source != permit.source
        || exit != permit.exit
        || source == exit
        || path != permit.path.nodes
        || path.first() != Some(&source)
        || path.last() != Some(&exit)
        || !(2..=crate::compile::MAX_HOPS).contains(&path.len())
        || path.iter().collect::<std::collections::BTreeSet<_>>().len() != path.len()
        || !permit.address.0.is_ipv4()
        || permit.source_addresses.ipv4.is_none()
        || !permit.transports.contains(&transport)
        || permit.transports.is_empty()
        || permit.transports.len() > 2
        || permit.max_pending == 0
        || permit.max_pending > 128
        || permit.timeout_ms == 0
        || permit.timeout_ms > 5_000
    {
        return Err(ResolverRouteError::Binding);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolverRouteError {
    Missing,
    Binding,
    Expired,
}

/// The caller must additionally retain and enforce its Control execution guard.
/// The host-provided ID only selects a published grant; it supplies no authority.
pub fn local_resolver_route<'a>(
    configuration: &'a NodeConfiguration,
    server_key: &str,
    demand_id: RequestId,
    now_ms: u64,
) -> Result<&'a LocalEgressPermit, ResolverRouteError> {
    let server = configuration
        .dns
        .settings
        .servers
        .get(server_key)
        .ok_or(ResolverRouteError::Missing)?;
    let permit = configuration
        .local_egress
        .get(&demand_id)
        .ok_or(ResolverRouteError::Missing)?;
    let address = match permit.target {
        BusinessTarget::Ip { address } | BusinessTarget::Domain { address, .. } => address.0,
        BusinessTarget::Mesh { .. } => return Err(ResolverRouteError::Binding),
    };
    let protocol = match server.transport {
        DnsTransport::Udp => BusinessProtocol::Udp,
        DnsTransport::Tcp => BusinessProtocol::Tcp,
    };
    let action_matches = matches!(
        (permit.action, configuration.decisions.get(&demand_id)),
        (LocalEgressAction::Direct, Some(DemandDecision::Direct))
            | (
                LocalEgressAction::Unmanaged,
                Some(DemandDecision::Unmanaged)
            )
    );
    if !configuration.enabled
        || permit.demand_id != demand_id
        || permit.source != configuration.node_id
        || address != server.server
        || address.is_unspecified()
        || address.is_multicast()
        || permit.protocol != protocol
        || permit.port != Some(server.server_port)
        || permit.profile != configuration.dns.profile
        || !action_matches
    {
        return Err(ResolverRouteError::Binding);
    }
    if permit.expires_at_ms <= now_ms {
        return Err(ResolverRouteError::Expired);
    }
    Ok(permit)
}

/// Local socket decision for one configured resolver under the installed
/// netmap. Only `Direct` and `Unmanaged` permit a local socket.
pub fn netmap_resolver(
    netmap: &meshway_configuration::netmap::NodeNetmap,
    server_key: &str,
) -> Result<meshway_configuration::netmap::ResolverAction, ResolverRouteError> {
    if !netmap.self_.enabled || !netmap.dns.settings.servers.contains_key(server_key) {
        return Err(ResolverRouteError::Missing);
    }
    netmap
        .dns
        .resolvers
        .get(server_key)
        .copied()
        .ok_or(ResolverRouteError::Missing)
}

/// Validate a Mesh resolver service against the installed netmap at its
/// source or resolver. The observed source/path come from authenticated ingress.
pub fn netmap_dns_service(
    leased: crate::admission::netmap::Leased<'_>,
    service: &meshway_configuration::netmap::DnsService,
    source: meshway_model::NodeId,
    exit: meshway_model::NodeId,
    path: &[meshway_model::NodeId],
    transport: DnsTransport,
    now_ms: u64,
) -> Result<(), ResolverRouteError> {
    use meshway_configuration::netmap::DnsPlan;
    let netmap = leased.netmap;
    if leased.lease.netmap.serial != netmap.serial
        || leased.lease.netmap.revision != netmap.revision
    {
        return Err(ResolverRouteError::Binding);
    }
    if !leased.lease.valid_at(now_ms) {
        return Err(ResolverRouteError::Expired);
    }
    let published = match &netmap.dns.egress {
        DnsPlan::ViaExit { service: own } => {
            netmap.node_id == source
                && own.as_ref() == service
                && netmap
                    .egress
                    .as_ref()
                    .is_some_and(|plan| plan.capture.generation == service.generation)
                && netmap.dns.profile == service.profile
                && netmap.self_.addresses == service.source_addresses
                && netmap.peers.get(&exit).is_some_and(|peer| {
                    peer.addresses.ipv4.map(std::net::IpAddr::V4) == Some(service.address.0)
                })
        }
        DnsPlan::ServeExitResolver { services } => {
            netmap.node_id == exit
                && netmap.self_.exit_enabled
                && netmap.exit_role.as_ref() == Some(&service.role)
                && netmap.self_.addresses.ipv4.map(std::net::IpAddr::V4) == Some(service.address.0)
                && netmap
                    .peers
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
