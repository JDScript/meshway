//! App projection from current connection/DNS handles to the diagnostic provider.
use crate::managed_connections::Prober;
use meshway_agent_connections::observations as connection;
pub use meshway_agent_diagnostics::{
    AuthenticatedChannel, Evidence, Incoming, LegacyProbeObservation, LiveResponse, LiveState,
    LiveStatus, MAX_PEERS, Operation, ProbeError, ProbeObservation, ProbeRequest, ProbeResponse,
    PublicIdentity, Request, SCHEMA_VERSION, WhoisObservation, dns, live_status, probe,
    socket_path, whois,
};
use std::{fs::File, future::Future, io, path::Path, pin::Pin, sync::Arc};

pub(crate) struct Server(meshway_agent_diagnostics::Server);
impl Server {
    pub(crate) async fn bind(path: &Path, lock: &File) -> io::Result<Self> {
        Ok(Self(
            meshway_agent_diagnostics::Server::bind(path, lock).await?,
        ))
    }
    #[cfg(test)]
    pub(crate) async fn serve(&self, prober: Prober) -> io::Result<()> {
        self.serve_with_dns(Some(prober), None).await
    }
    pub(crate) async fn serve_with_dns(
        &self,
        prober: Option<Prober>,
        dns: Option<meshway_agent_dns::Guard>,
    ) -> io::Result<()> {
        self.0.serve(Arc::new(Current { prober, dns })).await
    }
}

struct Current {
    prober: Option<Prober>,
    dns: Option<meshway_agent_dns::Guard>,
}
#[cfg(test)]
mod dns_tests;
impl meshway_agent_diagnostics::Provider for Current {
    fn live_status(&self, peers: bool, offset: u32, limit: u32) -> Result<LiveStatus, ProbeError> {
        let value = self
            .prober
            .as_ref()
            .ok_or(ProbeError::Unavailable)?
            .live_status(peers, offset, limit)
            .map_err(probe_error)?;
        Ok(LiveStatus {
            schema_version: SCHEMA_VERSION,
            state: LiveState::Installed,
            generation: value.generation,
            installed_revision: value.installed_revision,
            source_revision: value.source_revision,
            lease_remaining_ms: value.lease_remaining.as_millis().min(u64::MAX as u128) as u64,
            local_node_id: value.local_node_id,
            local_identity: value.local_identity.map(identity),
            network_id: None,
            authenticated_channel_count: value.authenticated_channel_count,
            peers: value.peers.map(|peers| {
                peers
                    .into_iter()
                    .map(|peer| AuthenticatedChannel {
                        peer_node_id: peer.peer_node_id,
                        initiator_node_id: peer.initiator_node_id,
                        transport: peer.transport,
                    })
                    .collect()
            }),
            peer_offset: value.peer_offset,
            next_peer_offset: value.next_peer_offset,
            evidence: Evidence::CurrentAuthenticatedChannelRegistry,
        })
    }
    fn whois(&self, query: &str) -> Result<WhoisObservation, ProbeError> {
        let value = self
            .prober
            .as_ref()
            .ok_or(ProbeError::Unavailable)?
            .whois(query)
            .map_err(probe_error)?;
        Ok(WhoisObservation {
            schema_version: SCHEMA_VERSION,
            generation: value.generation,
            installed_revision: value.installed_revision,
            source_revision: value.source_revision,
            identity: identity(value.identity),
            evidence: Evidence::CurrentTrustedInventory,
        })
    }
    fn probe_neighbor_observed(
        &self,
        request: ProbeRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProbeObservation, ProbeError>> + Send + '_>> {
        Box::pin(async move {
            request.validate()?;
            let value = self
                .prober
                .as_ref()
                .ok_or(ProbeError::Unavailable)?
                .probe_neighbor_observed(connection::ProbeRequest {
                    peer: request.peer,
                    timeout: std::time::Duration::from_millis(request.timeout_ms),
                })
                .await
                .map_err(probe_error)?;
            Ok(ProbeObservation {
                generation: value.generation,
                installed_revision: value.installed_revision,
                source_revision: value.source_revision,
                evidence: Evidence::DirectAuthenticatedNeighborRoundTrip,
                peer_node_id: value.peer_node_id,
                peer_name: value.peer_name,
                local_node_id: value.local_node_id,
                initiator_node_id: value.initiator_node_id,
                transport: value.transport,
                rtt_ns: value.round_trip.as_nanos().min(u64::MAX as u128) as u64,
            })
        })
    }
    fn dns_status(&self) -> dns::Status {
        let Some(guard) = &self.dns else {
            return dns_unavailable();
        };
        let status = guard.status();
        use meshway_agent_dns::StatusState;
        dns::Status {
            schema_version: SCHEMA_VERSION,
            state: match status.state {
                StatusState::Active => dns::State::Active,
                StatusState::Disabled => dns::State::Disabled,
                StatusState::Expired => dns::State::Expired,
                StatusState::Unavailable => dns::State::Unavailable,
            },
            generation: status.generation,
            revision_floor: status.revision_floor,
            active_revision: status.active_revision,
            zone: status.zone,
            ttl: status.ttl,
            lease_remaining_ms: status.lease_remaining_ms,
            listener: status.listener,
            listener_bound: status.listener_bound,
            network_id: status.network_id,
            node_id: status.node_id,
            evidence: dns::StatusEvidence::CurrentDnsGuard,
        }
    }
}
fn identity(value: connection::PublicIdentity) -> PublicIdentity {
    PublicIdentity {
        node_id: value.node_id,
        name: value.name,
        mesh_ipv4: value.mesh_ipv4,
        mesh_ipv6: value.mesh_ipv6,
    }
}
fn probe_error(value: connection::ProbeError) -> ProbeError {
    match value {
        connection::ProbeError::Unavailable => ProbeError::Unavailable,
        connection::ProbeError::UnconfiguredPeer => ProbeError::UnconfiguredPeer,
        connection::ProbeError::AmbiguousPeer => ProbeError::AmbiguousPeer,
        connection::ProbeError::ChannelUnavailable => ProbeError::ChannelUnavailable,
        connection::ProbeError::AuthenticationFailed => ProbeError::AuthenticationFailed,
        connection::ProbeError::Timeout => ProbeError::Timeout,
        connection::ProbeError::Busy => ProbeError::Busy,
        connection::ProbeError::InvalidRequest => ProbeError::InvalidRequest,
        connection::ProbeError::UnknownIdentity => ProbeError::UnknownIdentity,
    }
}
fn dns_unavailable() -> dns::Status {
    dns::Status {
        schema_version: SCHEMA_VERSION,
        state: dns::State::Unavailable,
        generation: 0,
        revision_floor: 0,
        active_revision: None,
        zone: None,
        ttl: None,
        lease_remaining_ms: 0,
        listener: None,
        listener_bound: false,
        network_id: None,
        node_id: None,
        evidence: dns::StatusEvidence::CurrentDnsGuard,
    }
}
