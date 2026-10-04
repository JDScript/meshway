//! Exact published local resolver bindings. This pure check is not an I/O grant.
use meshway_configuration::{
    DnsTransport,
    execution::{
        BusinessProtocol, BusinessTarget, DemandDecision, LocalEgressAction, LocalEgressPermit,
        NodeConfiguration,
    },
};
use meshway_model::RequestId;

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
