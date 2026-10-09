//! One accepted State report → the reporter's complete current peer facts.
use super::super::{REPORT_TTL_SECONDS, ReportAdmission};
use chrono::{DateTime, Duration, Utc};
use meshway_configuration::netmap::NodeNetmap;
use meshway_model::state::{
    EndpointSource, ListenerPhase, MAX_LOCAL_ENDPOINTS, ResourceRef, SocketNetwork,
};
use meshway_protocol::netmap::{PeerExitFacts, PeerFacts, PeerListenerFacts};
use meshway_protocol::state::StateReport;
use meshway_transport_spec::Listener;
use std::collections::BTreeMap;

/// Same bound as local endpoints: the newest few STUN mappings per listener.
const MAX_STUN_CANDIDATES: usize = 8;

/// Only listeners the reporter's own netmap declares contribute, and exit
/// readiness only for the role that netmap assigns to this instance. None
/// when the reporter has no netmap: it then discloses nothing.
pub(crate) fn peer_facts(
    admission: &ReportAdmission,
    report: &StateReport,
    reporter: Option<&NodeNetmap>,
    now: DateTime<Utc>,
) -> Option<PeerFacts> {
    let reporter = reporter.filter(|netmap| {
        netmap.network_id == admission.network_id
            && netmap.node_id == admission.node_id
            && netmap.self_.enabled
    })?;
    let state = &report.state;
    let mut listeners = BTreeMap::new();
    for (key, listener) in &state.listeners {
        let Some(declaration) = reporter.self_.listeners.get(key) else {
            continue;
        };
        let network = match declaration.declaration() {
            Listener::NativeUdp(_) => SocketNetwork::Udp,
            Listener::NativeTcp(_) | Listener::Vless(_) | Listener::Trojan(_) => SocketNetwork::Tcp,
        };
        let sockets: Vec<_> = listener
            .socket_ids
            .iter()
            .filter_map(|id| state.sockets.get(id))
            .filter(|socket| {
                socket.resource == ResourceRef::Listener { key: key.clone() }
                    && socket.network == network
            })
            .collect();
        let listening = listener.phase == ListenerPhase::Listening;
        let mut local_endpoints = Vec::new();
        let mut stun_candidates = Vec::new();
        if listening {
            for socket in &sockets {
                for endpoint in socket.local_endpoints.iter().flatten() {
                    if local_endpoints.len() < MAX_LOCAL_ENDPOINTS
                        && !local_endpoints.iter().any(
                            |known: &meshway_model::state::LocalEndpoint| {
                                known.address == endpoint.address
                            },
                        )
                    {
                        local_endpoints.push(endpoint.clone());
                    }
                }
                for candidate in &socket.endpoints {
                    if stun_candidates.len() < MAX_STUN_CANDIDATES
                        && matches!(candidate.origin, EndpointSource::Stun { .. })
                        && candidate.expires_at > now
                    {
                        stun_candidates.push(candidate.clone());
                    }
                }
            }
        }
        listeners.insert(
            key.clone(),
            PeerListenerFacts {
                phase: listener.phase,
                network,
                local_endpoints,
                stun_candidates,
            },
        );
    }
    let exit = match (&reporter.exit_role, &state.exit_capability) {
        (Some(role), Some(capability))
            if capability.instance_id == admission.instance_id
                && capability.observed_at <= report.observed_at
                && capability.observed_at + Duration::seconds(REPORT_TTL_SECONDS) > now =>
        {
            Some(PeerExitFacts {
                role_generation: role.generation,
                status: capability.status.clone(),
                installed_families: capability
                    .installed_families
                    .iter()
                    .filter(|family| **family == role.family)
                    .copied()
                    .collect(),
            })
        }
        _ => None,
    };
    let expires_at = (report.observed_at + Duration::seconds(REPORT_TTL_SECONDS))
        .min(now + Duration::seconds(REPORT_TTL_SECONDS))
        .min(admission.lease_expires_at);
    Some(PeerFacts {
        node_id: admission.node_id,
        listeners,
        exit,
        observed_at: report.observed_at,
        expires_at,
    })
}
