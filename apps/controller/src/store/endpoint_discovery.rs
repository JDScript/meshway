use super::{Store, connection_runtime, connections};
use crate::Error;
use chrono::{DateTime, Duration, Utc};
use meshway_protocol::*;
use rusqlite::OptionalExtension;
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
};

#[cfg(test)]
mod tests;

#[derive(Clone)]
struct SocketWatermark {
    kind: UdpDiscoveryResourceKind,
    resource: String,
    bound: String,
    // Keep at most four observer watermarks per live socket. Retired socket
    // ordinals cannot return; a generation change retires every old ordinal.
    observations: BTreeMap<String, StunEndpointObservation>,
    retired_observed_floor: Option<DateTime<Utc>>,
}

pub(super) struct StoredDiscovery {
    request: ReportEndpointDiscoveryRequest,
    reported_at: DateTime<Utc>,
    generation: u32,
    highest_socket: u32,
    generation_floor: Option<DateTime<Utc>>,
    latest_observed: Option<DateTime<Utc>>,
    sockets: BTreeMap<u32, SocketWatermark>,
}

impl Store {
    pub fn report_endpoint_discovery(
        &mut self,
        token: &str,
        request: &ReportEndpointDiscoveryRequest,
    ) -> Result<ReportEndpointDiscoveryResponse, Error> {
        self.report_endpoint_discovery_at(token, request, Utc::now())
    }

    fn report_endpoint_discovery_at(
        &mut self,
        token: &str,
        request: &ReportEndpointDiscoveryRequest,
        now: DateTime<Utc>,
    ) -> Result<ReportEndpointDiscoveryResponse, Error> {
        let id = connection_runtime::bound_node(&self.connection, token)?;
        let epoch: Option<u32> = self
            .connection
            .query_row(
                "SELECT epoch FROM connection_runtime_leases WHERE node_id=?1",
                [&id],
                |row| row.get(0),
            )
            .optional()?;
        if epoch != Some(request.epoch) {
            return Err(Error::Conflict);
        }
        let current =
            connections::projection(&connections::admin_snapshot(&self.connection)?, &id)?;
        if request.revision != current.payload.revision || request.digest != current.digest {
            return Err(Error::Unprocessable(
                "discovery revision/digest does not match current node connections",
            ));
        }
        let previous = self
            .discoveries
            .get(&id)
            .filter(|old| old.request.epoch == request.epoch);
        if let Some(old) = previous {
            if request.sequence < old.request.sequence
                || (request.sequence == old.request.sequence && request != &old.request)
            {
                return Err(Error::Conflict);
            }
            // Exact retries remain valid after expiry, but cannot renew any time.
            if request == &old.request {
                return Ok(response(old));
            }
        }
        let generation = request
            .sockets
            .first()
            .map(|socket| socket.generation)
            .unwrap_or_else(|| previous.map_or(0, |old| old.generation));
        if previous.is_some_and(|old| generation < old.generation) {
            return Err(Error::Conflict);
        }
        let same_generation = previous.is_some_and(|old| generation == old.generation);
        let mut highest_socket = if same_generation {
            previous.map_or(0, |old| old.highest_socket)
        } else {
            0
        };
        let generation_floor = previous.and_then(|old| {
            if same_generation {
                old.generation_floor
            } else {
                old.latest_observed
            }
        });
        let mut latest_observed = previous.and_then(|old| old.latest_observed);
        for socket in &request.sockets {
            let old = previous
                .filter(|_| same_generation)
                .and_then(|old| old.sockets.get(&socket.socket_id));
            if let Some(old) = old {
                if old.kind != socket.resource_kind
                    || old.resource != socket.resource_id
                    || old.bound != socket.bound_address
                {
                    return Err(Error::Conflict);
                }
            } else if same_generation && socket.socket_id <= highest_socket {
                return Err(Error::Conflict);
            }
        }
        validate(request, &current.payload, now)?;
        let mut sockets = BTreeMap::new();
        for socket in &request.sockets {
            let old = previous
                .filter(|_| same_generation)
                .and_then(|old| old.sockets.get(&socket.socket_id));
            // Compare all new IDs to the previous high watermark, not iteration
            // order: complete snapshots may list sockets in any order.
            let mut watermark = old.cloned().unwrap_or_else(|| SocketWatermark {
                kind: socket.resource_kind,
                resource: socket.resource_id.clone(),
                bound: socket.bound_address.clone(),
                observations: BTreeMap::new(),
                retired_observed_floor: None,
            });
            let observer_keys = socket
                .observations
                .iter()
                .map(|observation| {
                    address(&observation.observer, false).map(|address| address.to_string())
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            // A compact time floor replaces retired observer history, allowing
            // observer rotation while bounding storage to the current report.
            for (observer, prior) in &watermark.observations {
                if !observer_keys.contains(observer) {
                    watermark.retired_observed_floor = Some(
                        watermark
                            .retired_observed_floor
                            .map_or(prior.observed_at, |floor| floor.max(prior.observed_at)),
                    );
                }
            }
            for observation in &socket.observations {
                if generation_floor.is_some_and(|floor| observation.observed_at <= floor) {
                    return Err(Error::Conflict);
                }
                let observer_key = address(&observation.observer, false)?.to_string();
                if !watermark.observations.contains_key(&observer_key)
                    && watermark
                        .retired_observed_floor
                        .is_some_and(|floor| observation.observed_at <= floor)
                {
                    return Err(Error::Conflict);
                }
                if let Some(prior) = watermark.observations.get(&observer_key)
                    && (observation.observed_at < prior.observed_at
                        || (observation.observed_at == prior.observed_at && observation != prior))
                {
                    return Err(Error::Conflict);
                }
                watermark
                    .observations
                    .insert(observer_key, observation.clone());
                latest_observed = Some(latest_observed.map_or(observation.observed_at, |old| {
                    old.max(observation.observed_at)
                }));
            }
            watermark
                .observations
                .retain(|observer, _| observer_keys.contains(observer));
            sockets.insert(socket.socket_id, watermark);
        }
        highest_socket = highest_socket.max(
            request
                .sockets
                .iter()
                .map(|socket| socket.socket_id)
                .max()
                .unwrap_or(0),
        );
        let stored = StoredDiscovery {
            request: request.clone(),
            reported_at: now,
            generation,
            highest_socket,
            generation_floor,
            latest_observed,
            sockets,
        };
        let result = response(&stored);
        self.discoveries.insert(id, stored);
        Ok(result)
    }

    pub fn endpoint_discoveries(&self) -> Result<EndpointDiscoveriesSnapshot, Error> {
        self.endpoint_discoveries_at(Utc::now())
    }

    fn endpoint_discoveries_at(
        &self,
        now: DateTime<Utc>,
    ) -> Result<EndpointDiscoveriesSnapshot, Error> {
        let snapshot = connections::admin_snapshot(&self.connection)?;
        let runtimes = self.connection_runtimes_at(now)?;
        let mut nodes = Vec::with_capacity(snapshot.nodes.len());
        for node in &snapshot.nodes {
            let id = &node.node.id;
            let revoked = node.node.revoked_at.is_some();
            let runtime = runtimes.nodes.iter().find(|runtime| &runtime.node_id == id);
            let mut view = NodeEndpointDiscovery {
                node_id: id.clone(),
                epoch: runtime.map_or(0, |runtime| runtime.epoch),
                sequence: 0,
                revision: None,
                digest: None,
                state: if revoked {
                    EndpointDiscoveryState::Withdrawn
                } else {
                    EndpointDiscoveryState::AwaitingReport
                },
                reported_at: None,
                current: false,
                stale: true,
                sockets: Vec::new(),
            };
            if let Some(stored) = self.discoveries.get(id) {
                let report = &stored.request;
                let configuration_matches = if revoked {
                    false
                } else {
                    let current = connections::projection(&snapshot, id)?;
                    report.revision == current.payload.revision && report.digest == current.digest
                };
                let evidence = runtime.is_some_and(|runtime| {
                    runtime.epoch == report.epoch
                        && runtime.current
                        && !runtime.stale
                        && runtime.state == ConnectionRuntimeState::Applied
                });
                view.epoch = report.epoch;
                view.sequence = report.sequence;
                view.revision = Some(report.revision);
                view.digest = Some(report.digest.clone());
                view.reported_at = Some(stored.reported_at);
                view.current = evidence && configuration_matches;
                let age = now.signed_duration_since(stored.reported_at);
                view.stale =
                    !view.current || age < Duration::zero() || age >= Duration::seconds(60);
                view.state = report.state;
                if !view.current {
                    view.state = EndpointDiscoveryState::Withdrawn;
                } else if view.stale {
                    view.current = false;
                    view.state = EndpointDiscoveryState::Expired;
                } else {
                    view.sockets = report.sockets.clone();
                    let had_observations = view
                        .sockets
                        .iter()
                        .any(|socket| !socket.observations.is_empty());
                    for socket in &mut view.sockets {
                        socket
                            .observations
                            .retain(|observation| observation.expires_at > now);
                    }
                    let live_mapping = view
                        .sockets
                        .iter()
                        .flat_map(|socket| &socket.observations)
                        .any(|observation| observation.outcome == StunDiscoveryOutcome::Mapped);
                    let any_live = view
                        .sockets
                        .iter()
                        .any(|socket| !socket.observations.is_empty());
                    if (report.state == EndpointDiscoveryState::Observed && !live_mapping)
                        || (had_observations && !any_live)
                    {
                        view.state = EndpointDiscoveryState::Expired;
                        view.current = false;
                        view.stale = true;
                        view.sockets.clear();
                    }
                }
            }
            nodes.push(view);
        }
        let network_id = self.connection.query_row(
            "SELECT network_id FROM network_identity WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        Ok(EndpointDiscoveriesSnapshot { network_id, nodes })
    }
}

fn response(stored: &StoredDiscovery) -> ReportEndpointDiscoveryResponse {
    ReportEndpointDiscoveryResponse {
        epoch: stored.request.epoch,
        sequence: stored.request.sequence,
        reported_at: stored.reported_at,
    }
}

fn address(value: &str, local: bool) -> Result<SocketAddr, Error> {
    let invalid = || Error::Unprocessable("invalid numeric UDP socket address");
    if value.len() > 64 || value.contains('%') {
        return Err(invalid());
    }
    let address: SocketAddr = value.parse().map_err(|_| invalid())?;
    let ip = address.ip();
    if address.port() == 0
        || ip.is_multicast()
        || (!local && ip.is_unspecified())
        || matches!(ip, IpAddr::V4(ip) if ip.is_broadcast())
        || matches!(address, SocketAddr::V6(ip) if ip.scope_id() != 0 || ip.flowinfo() != 0 || ip.ip().to_ipv4_mapped().is_some())
    {
        return Err(invalid());
    }
    Ok(address)
}

fn validate(
    request: &ReportEndpointDiscoveryRequest,
    configuration: &NodeConnections,
    now: DateTime<Utc>,
) -> Result<(), Error> {
    use EndpointDiscoveryState as State;
    if request.epoch == 0
        || request.sequence == 0
        || request.revision == 0
        || !local::valid_credential(&request.digest)
        || request.sockets.len() > MAX_DISCOVERY_SOCKETS
    {
        return Err(Error::Unprocessable(
            "invalid discovery counters, digest or socket limit",
        ));
    }
    match request.state {
        State::AwaitingReport | State::Expired => {
            return Err(Error::Unprocessable("discovery state is server-only"));
        }
        State::Disabled | State::Withdrawn | State::Error if !request.sockets.is_empty() => {
            return Err(Error::Unprocessable(
                "inactive discovery must have no sockets",
            ));
        }
        _ => {}
    }
    let mut sockets = BTreeSet::new();
    let generation = request.sockets.first().map(|socket| socket.generation);
    let mut mapped = false;
    for socket in &request.sockets {
        if socket.generation == 0
            || socket.socket_id == 0
            || Some(socket.generation) != generation
            || !sockets.insert(socket.socket_id)
            || socket.observations.len() > MAX_DISCOVERY_OBSERVERS
        {
            return Err(Error::Unprocessable(
                "invalid discovery generation, socket ID or observer limit",
            ));
        }
        if !configuration.valid {
            return Err(Error::Unprocessable(
                "discovery requires valid node connections",
            ));
        }
        let bound = address(&socket.bound_address, true)?;
        match socket.resource_kind {
            UdpDiscoveryResourceKind::Listener => {
                let resource = configuration
                    .listeners
                    .iter()
                    .find(|resource| {
                        resource.id == socket.resource_id
                            && resource.transport == ConnectionTransport::NativeUdp
                    })
                    .ok_or(Error::Unprocessable(
                        "discovery socket is outside current native UDP resources",
                    ))?;
                // A listener's OS local_addr preserves the configured bind,
                // including an unspecified IP. Ports are fixed and nonzero in
                // the current connection-plan contract; compare parsed values.
                let configured = address(&resource.bind, true)?;
                if bound != configured {
                    return Err(Error::Unprocessable(
                        "discovery listener socket does not match configured bind",
                    ));
                }
            }
            UdpDiscoveryResourceKind::OutgoingCandidate => {
                let resource = configuration
                    .outgoing
                    .iter()
                    .find(|resource| {
                        resource.id == socket.resource_id
                            && resource.transport == ConnectionTransport::NativeUdp
                    })
                    .ok_or(Error::Unprocessable(
                        "discovery socket is outside current native UDP resources",
                    ))?;
                // DNS endpoints can resolve to either family. Only a literal IP
                // supplies enough configuration evidence to reject a mismatch.
                if let Ok(ip) = resource.endpoint.host.parse::<IpAddr>()
                    && ip.is_ipv4() != bound.is_ipv4()
                {
                    return Err(Error::Unprocessable(
                        "discovery outgoing socket family does not match literal endpoint",
                    ));
                }
            }
        }
        let mut observers = BTreeSet::new();
        for observation in &socket.observations {
            let observer = address(&observation.observer, false)?;
            if observer.is_ipv4() != bound.is_ipv4() || !observers.insert(observer) {
                return Err(Error::Unprocessable(
                    "duplicate observer or socket family mismatch",
                ));
            }
            if observation.observed_at > now + Duration::seconds(5)
                || observation.expires_at <= now
                || observation.expires_at < observation.observed_at
                || observation.expires_at - observation.observed_at
                    > Duration::seconds(i64::from(ENDPOINT_DISCOVERY_TTL_SECONDS))
                || observation.expires_at
                    > now + Duration::seconds(i64::from(ENDPOINT_DISCOVERY_TTL_SECONDS))
            {
                return Err(Error::Unprocessable(
                    "invalid discovery observation lease times",
                ));
            }
            if observation.outcome == StunDiscoveryOutcome::Mapped {
                let endpoint = address(
                    observation
                        .mapped_endpoint
                        .as_deref()
                        .ok_or(Error::Unprocessable("mapped observation requires endpoint"))?,
                    false,
                )?;
                if endpoint.is_ipv4() != bound.is_ipv4()
                    || observation.rtt_ms.is_none_or(|rtt| rtt > 1500)
                {
                    return Err(Error::Unprocessable("invalid mapped family or RTT"));
                }
                mapped = true;
            } else if observation.mapped_endpoint.is_some() || observation.rtt_ms.is_some() {
                return Err(Error::Unprocessable(
                    "failed observation must not claim mapping or RTT",
                ));
            }
        }
    }
    if (request.state == State::Observed) != mapped {
        return Err(Error::Unprocessable(
            "observed state requires mapped observation; probing must not claim mapping",
        ));
    }
    Ok(())
}
