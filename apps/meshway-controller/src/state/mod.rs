//! Volatile source reports and read-time projections. All calls are serialized
//! with commits on the SQLite owner; no HTTP future owns committed effects.
pub mod service;
pub mod stream;
pub(crate) mod validation;

use crate::resources::GeoIp;
use chrono::{DateTime, Duration, Utc};
use meshway_model::{
    Digest, InstanceId, NetworkId, NodeId, Revision, Sequence, StateEpoch,
    state::{ConfigurationRef, ResourceRef, SocketNetwork},
};
use meshway_protocol::state::*;
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_REPORT_BYTES: usize = 1024 * 1024;
pub const MAX_NETWORK_REPORTS: usize = 10_000;
pub const MAX_STATE_BYTES: usize = 128 * 1024 * 1024;
pub const REPORT_TTL_SECONDS: i64 = 60;
pub const CLOCK_SKEW_SECONDS: i64 = 30;

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum StateError {
    #[error("identity is invalid or expired")]
    Unauthorized,
    #[error("resource is unavailable in the current scope")]
    NotFound,
    #[error("operation is not permitted")]
    Forbidden,
    #[error("invalid report or query")]
    Invalid,
    #[error("report sequence or event identifier conflicts")]
    Conflict,
    #[error("configured resource budget exceeded")]
    Budget,
    #[error("observation budget exceeded")]
    ObservationLimit,
    #[error("cursor is invalid or expired")]
    Cursor,
    #[error("state dependency is unavailable")]
    Unavailable,
    #[error("storage failed")]
    Storage,
}
impl From<rusqlite::Error> for StateError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}
impl From<crate::store::ExecutorError> for StateError {
    fn from(_: crate::store::ExecutorError) -> Self {
        Self::Unavailable
    }
}

/// Constructed by the Node/execution owner after current transactional checks.
/// It contains only already-published facts, never a client-supplied authorization map.
#[derive(Clone, Debug)]
pub struct ReportAdmission {
    pub network_id: NetworkId,
    pub node_id: NodeId,
    pub instance_id: InstanceId,
    pub lease_expires_at: DateTime<Utc>,
    /// Current execution context, distinct from saved/installation/history.
    pub execution: Option<ConfigurationRef>,
    pub configurations: Vec<ConfigurationRef>,
    /// Exact immutable contexts loaded by the execution owner. Historical
    /// contexts do not grant current execution or peer endpoint availability.
    pub contexts: Vec<ConfigurationAdmission>,
    pub mesh_addresses: BTreeSet<std::net::IpAddr>,
}
#[derive(Clone, Debug)]
pub struct ConfigurationAdmission {
    pub path_deadlines: BTreeMap<Vec<NodeId>, DateTime<Utc>>,
    pub configuration: ConfigurationRef,
    /// True only for the execution owner's currently allowed target/saved
    /// installation. Retained history alone must set this to false.
    pub installation_allowed: bool,
    pub resources: BTreeMap<ResourceRef, ResourceAdmission>,
    pub peers: BTreeSet<NodeId>,
    pub paths: Vec<Vec<NodeId>>,
    /// Authorized adjacent peers and allowed outer transports, never arbitrary
    /// subpaths of a business permit. Also used for connect probes.
    pub neighbor_transports: BTreeMap<NodeId, BTreeSet<String>>,
    pub profiles: BTreeSet<String>,
}
impl ReportAdmission {
    pub fn context(
        &self,
        reference: &ConfigurationRef,
    ) -> Result<&ConfigurationAdmission, StateError> {
        if !self.configurations.contains(reference) {
            return Err(StateError::Invalid);
        }
        self.contexts
            .iter()
            .find(|context| &context.configuration == reference)
            .ok_or(StateError::Invalid)
    }
    pub fn current(&self) -> Result<&ConfigurationAdmission, StateError> {
        self.context(self.execution.as_ref().ok_or(StateError::Invalid)?)
    }
}
#[derive(Clone, Debug)]
pub struct ResourceAdmission {
    pub network: SocketNetwork,
    pub transport: String,
    pub configured_endpoints: BTreeMap<String, String>,
}

/// Recomputed by the authorization owner for every read and stream poll.
/// `full` is only self/admin visibility; dependency nodes use the peer projection.
#[derive(Clone, Debug)]
pub struct VisibilityScope {
    pub network_id: NetworkId,
    pub configuration_revision: Revision,
    pub authorization_digest: Digest,
    pub observation_authorization_digest: Digest,
    pub visible: BTreeSet<NodeId>,
    pub full: BTreeSet<NodeId>,
    pub observation_paths: BTreeSet<Vec<NodeId>>,
    pub mesh_addresses: BTreeSet<std::net::IpAddr>,
    pub expires_at: DateTime<Utc>,
    pub current: BTreeMap<NodeId, CurrentNode>,
}
#[derive(Clone, Debug)]
pub struct CurrentNode {
    pub instance_id: Option<InstanceId>,
    /// The currently usable execution, not the set retained for receipts/history.
    pub execution: Option<ConfigurationRef>,
    pub configurations: Vec<ConfigurationRef>,
    pub revoked: bool,
    pub lease_expires_at: Option<DateTime<Utc>>,
}
#[derive(Clone)]
struct StoredReport {
    report: StateReport,
    received_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    authenticated_at: DateTime<Utc>,
    bytes: usize,
    next_expiration: Option<DateTime<Utc>>,
}

pub struct StateHub {
    pub epoch: StateEpoch,
    sequences: BTreeMap<NetworkId, Sequence>,
    exhausted: BTreeSet<NetworkId>,
    reports: BTreeMap<(NetworkId, NodeId), StoredReport>,
    pub events: stream::EventLog,
    bytes: usize,
}

#[cfg(test)]
mod observation_tests;
#[cfg(test)]
mod service_tests;
#[cfg(test)]
mod tests;
impl StateHub {
    pub fn location(
        &self,
        admission: &ReportAdmission,
        geoip: &mut GeoIp,
        now: DateTime<Utc>,
    ) -> DerivedNodeState {
        let report = self
            .reports
            .get(&(admission.network_id, admission.node_id))
            .filter(|stored| {
                stored.report.instance_id == admission.instance_id
                    && stored.report.state.configuration.applied.as_ref()
                        == admission.execution.as_ref()
                    && stored.expires_at > now
                    && admission.lease_expires_at > now
            })
            .map(|stored| VisibleReport::Full {
                report: Box::new(stored.report.state.clone()),
            });
        geoip.derive(
            report.as_ref(),
            report.is_some(),
            &admission.mesh_addresses,
            now,
        )
    }
    pub fn new() -> Result<Self, StateError> {
        let epoch =
            StateEpoch::new(crate::accounts::random_uuid().map_err(|_| StateError::Unavailable)?)
                .map_err(|_| StateError::Unavailable)?;
        Ok(Self {
            epoch,
            sequences: BTreeMap::new(),
            exhausted: BTreeSet::new(),
            reports: BTreeMap::new(),
            events: stream::EventLog::new(epoch),
            bytes: 0,
        })
    }

    pub fn accept_report(
        &mut self,
        admission: &ReportAdmission,
        report: StateReport,
        now: DateTime<Utc>,
    ) -> Result<StateAccepted, StateError> {
        validation::report(admission, &report, now)?;
        let key = (admission.network_id, admission.node_id);
        let report_bytes = serde_json::to_vec(&report)
            .map_err(|_| StateError::Invalid)?
            .len();
        if let Some(previous) = self.reports.get_mut(&key)
            && previous.report.instance_id == report.instance_id
        {
            if report.sequence < previous.report.sequence {
                return Err(StateError::Conflict);
            }
            if report.sequence == previous.report.sequence {
                if report != previous.report {
                    return Err(StateError::Conflict);
                }
                previous.authenticated_at = now;
                return Ok(StateAccepted {
                    sequence: report.sequence,
                    received_at: previous.received_at,
                    duplicate: true,
                });
            }
        }
        if !self.reports.contains_key(&key)
            && self
                .reports
                .keys()
                .filter(|(network, _)| *network == admission.network_id)
                .count()
                >= MAX_NETWORK_REPORTS
        {
            return Err(StateError::Budget);
        }
        self.sequences
            .get(&admission.network_id)
            .copied()
            .unwrap_or(Sequence::MIN)
            .checked_next()
            .map_err(|_| StateError::Budget)?;
        let previous_bytes = self.reports.get(&key).map_or(0, |stored| stored.bytes);
        let total_bytes = self
            .bytes
            .saturating_sub(previous_bytes)
            .saturating_add(report_bytes);
        if total_bytes > MAX_STATE_BYTES {
            return Err(StateError::Budget);
        }
        let sequence = report.sequence;
        let expires_at = (report.observed_at + Duration::seconds(REPORT_TTL_SECONDS))
            .min(now + Duration::seconds(REPORT_TTL_SECONDS))
            .min(admission.lease_expires_at);
        let next_expiration = next_expiration(&report, expires_at, now);
        self.reports.insert(
            key,
            StoredReport {
                report,
                received_at: now,
                expires_at,
                authenticated_at: now,
                bytes: report_bytes,
                next_expiration,
            },
        );
        self.bytes = total_bytes;
        self.publish_committed(stream::CommittedChange::State {
            network_id: admission.network_id,
            node_id: admission.node_id,
        });
        Ok(StateAccepted {
            sequence,
            received_at: now,
            duplicate: false,
        })
    }

    /// Must run on the SQLite owner immediately after COMMIT, before releasing
    /// its accepted operation. It does not await a consumer or perform fallible IO.
    pub fn publish_committed(&mut self, change: stream::CommittedChange) {
        if let stream::CommittedChange::NodeRetired {
            network_id,
            node_id,
        } = &change
        {
            // Permanent credential revocation is the durable replay barrier.
            // No old identity may obtain ReportAdmission after this commit.
            if let Some(stored) = self.reports.remove(&(*network_id, *node_id)) {
                self.bytes = self.bytes.saturating_sub(stored.bytes);
            }
        }
        let network = change.network();
        match self
            .sequences
            .get(&network)
            .copied()
            .unwrap_or(Sequence::MIN)
            .checked_next()
        {
            Ok(sequence) => {
                self.sequences.insert(network, sequence);
            }
            Err(_) => {
                self.exhausted.insert(network);
            }
        }
        self.events.append(change);
    }

    pub fn dataset_changed(&mut self) {
        let networks: BTreeSet<_> = self.reports.keys().map(|(network, _)| *network).collect();
        for network_id in networks {
            self.publish_committed(stream::CommittedChange::Resource { network_id });
        }
    }

    pub fn query(
        &mut self,
        scope: &VisibilityScope,
        nodes: Option<&[NodeId]>,
        geoip: &mut GeoIp,
        now: DateTime<Utc>,
    ) -> Result<StateView, StateError> {
        if scope.expires_at <= now {
            return Err(StateError::Unauthorized);
        }
        if self.exhausted.contains(&scope.network_id) {
            return Err(StateError::Unavailable);
        }
        let mut expired = vec![];
        for ((network, node), stored) in &mut self.reports {
            if *network == scope.network_id && stored.next_expiration.is_some_and(|at| at <= now) {
                stored.next_expiration = next_expiration(&stored.report, stored.expires_at, now);
                expired.push(*node);
            }
        }
        for node_id in expired {
            self.publish_committed(stream::CommittedChange::State {
                network_id: scope.network_id,
                node_id,
            });
        }
        if nodes.is_some_and(|nodes| {
            nodes.len() > MAX_NETWORK_REPORTS || nodes.iter().any(|id| !scope.visible.contains(id))
        }) {
            return Err(StateError::NotFound);
        }
        let selected = nodes
            .map(|ids| ids.iter().copied().collect())
            .unwrap_or_else(|| scope.visible.clone());
        let mut views = BTreeMap::new();
        for node_id in selected {
            let stored = self.reports.get(&(scope.network_id, node_id));
            let current = scope.current.get(&node_id);
            let freshness = match (stored, current) {
                (_, Some(current)) if current.revoked => Freshness::Revoked,
                (None, _) => Freshness::AwaitingReport,
                (Some(stored), Some(current))
                    if current.instance_id != Some(stored.report.instance_id) =>
                {
                    Freshness::Superseded
                }
                (Some(stored), Some(current))
                    if stored
                        .report
                        .state
                        .configuration
                        .applied
                        .as_ref()
                        .is_some_and(|reference| !current.configurations.contains(reference)) =>
                {
                    Freshness::ConfigMismatch
                }
                (Some(stored), _) if stored.expires_at <= now => Freshness::Expired,
                (_, Some(current))
                    if current
                        .lease_expires_at
                        .is_none_or(|expires| expires <= now) =>
                {
                    Freshness::Expired
                }
                (Some(_), None) => Freshness::ConfigMismatch,
                _ => Freshness::Current,
            };
            let report = stored.map(|stored| {
                if scope.full.contains(&node_id) {
                    let mut report = stored.report.state.clone();
                    report.links.retain(|peer, _| scope.visible.contains(peer));
                    report.paths.retain(|_, path| {
                        path.nodes.iter().all(|node| scope.visible.contains(node))
                    });
                    report
                        .probes
                        .retain(|probe| probe.path.iter().all(|node| scope.visible.contains(node)));
                    VisibleReport::Full {
                        report: Box::new(report),
                    }
                } else {
                    let source = &stored.report.state;
                    let mut endpoints = Vec::new();
                    if freshness == Freshness::Current
                        && source.configuration.applied.is_some()
                        && current.and_then(|node| node.execution.as_ref()) == source.configuration.applied.as_ref()
                    {
                        for (socket_id, socket) in &source.sockets {
                            if source.configuration.applied.is_none()
                                || socket.configuration.as_ref().is_some_and(|reference| Some(reference) != source.configuration.applied.as_ref()) {
                                continue;
                            }
                            if let ResourceRef::Listener { key } = &socket.resource
                                && source.listeners.get(key).is_none_or(|listener| {
                                    listener.phase != meshway_model::state::ListenerPhase::Listening
                                        || listener.configuration.as_ref().is_some_and(
                                            |reference| {
                                                Some(reference)
                                                    != source.configuration.applied.as_ref()
                                            },
                                        )
                                })
                            {
                                continue;
                            }
                            for candidate in &socket.endpoints {
                                if candidate.address.parse::<std::net::SocketAddr>().is_ok_and(|address|matches!(address.ip(),std::net::IpAddr::V6(ip) if ip.is_unicast_link_local())) { continue; }
                                if candidate.expires_at <= now {
                                    continue;
                                }
                                let candidate = PeerCandidate {
                                    address: candidate.address.clone(),
                                    source: match candidate.origin {
                                        meshway_model::state::EndpointSource::Configured {
                                            ..
                                        } => CandidateSource::Configured,
                                        meshway_model::state::EndpointSource::Interface {
                                            ..
                                        } => CandidateSource::Interface,
                                        meshway_model::state::EndpointSource::Stun { .. } => {
                                            CandidateSource::Stun
                                        }
                                    },
                                    observed_at: candidate.observed_at,
                                    expires_at: candidate.expires_at,
                                };
                                endpoints.push(PeerEndpoint {
                                    socket_id: socket_id.clone(),
                                    resource: socket.resource.clone(),
                                    generation: socket.generation,
                                    network: socket.network,
                                    candidate,
                                });
                            }
                        }
                    }
                    VisibleReport::Peer {
                        report: Box::new(PeerState {
                            capabilities: if freshness == Freshness::Current {
                                source.capabilities.clone()
                            } else {
                                BTreeMap::new()
                            },
                            configuration: source.configuration.clone(),
                            endpoints,
                        }),
                    }
                }
            });
            let derived = geoip.derive(
                report.as_ref(),
                freshness == Freshness::Current,
                &scope.mesh_addresses,
                now,
            );
            views.insert(
                node_id,
                NodeStateView {
                    instance_id: stored.map(|s| s.report.instance_id),
                    report_sequence: stored.map(|s| s.report.sequence),
                    observed_at: stored.map(|s| s.report.observed_at),
                    received_at: stored.map(|s| s.received_at),
                    report_expires_at: stored.map(|s| s.expires_at),
                    last_authenticated_at: stored.map(|s| s.authenticated_at),
                    freshness,
                    report,
                    derived,
                },
            );
        }
        let (links, paths) = aggregate(&views, now);
        Ok(StateView {
            network_id: scope.network_id,
            state_epoch: self.epoch,
            sequence: self
                .sequences
                .get(&scope.network_id)
                .copied()
                .unwrap_or(Sequence::MIN),
            configuration_revision: scope.configuration_revision,
            as_of: now,
            nodes: views,
            links,
            paths,
        })
    }
}

fn next_expiration(
    report: &StateReport,
    expires: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    std::iter::once(expires)
        .chain(
            report
                .state
                .sockets
                .values()
                .flat_map(|socket| socket.endpoints.iter().map(|endpoint| endpoint.expires_at)),
        )
        .chain(
            report
                .state
                .links
                .values()
                .flatten()
                .map(|link| link.expires_at),
        )
        .chain(report.state.paths.values().map(|path| path.expires_at))
        .filter(|expires| *expires > now)
        .min()
}

fn aggregate(
    views: &BTreeMap<NodeId, NodeStateView>,
    now: DateTime<Utc>,
) -> (Vec<AggregatedLink>, Vec<AggregatedPath>) {
    let mut links: BTreeMap<(NodeId, NodeId, String), AggregatedLink> = BTreeMap::new();
    let mut paths = vec![];
    for (node, view) in views {
        let (Some(instance_id), Some(report_sequence), Some(VisibleReport::Full { report })) =
            (view.instance_id, view.report_sequence, &view.report)
        else {
            continue;
        };
        let reference = ReportRef {
            node_id: *node,
            instance_id,
            report_sequence,
        };
        for (peer, records) in &report.links {
            for link in records {
                let a = (*node).min(*peer);
                let b = (*node).max(*peer);
                let current = view.freshness == Freshness::Current && link.expires_at > now;
                let key = (a, b, link.link_id.clone());
                if let Some(existing) = links.get_mut(&key) {
                    let matches = existing.transport == link.transport
                        && existing.phase == link.phase
                        && existing.initiator_node_id == link.initiator_node_id
                        && existing.evidence == LinkEvidence::OneSided
                        && current;
                    existing.evidence = if matches {
                        LinkEvidence::Matched
                    } else if !current || existing.evidence == LinkEvidence::Expired {
                        LinkEvidence::Expired
                    } else {
                        LinkEvidence::Conflicting
                    };
                    existing.reports.push(reference.clone());
                    existing.in_use = matches.then_some(
                        existing.in_use.unwrap_or(false) || link.business_dependencies > 0,
                    );
                } else {
                    links.insert(
                        key,
                        AggregatedLink {
                            a,
                            b,
                            link_id: link.link_id.clone(),
                            transport: link.transport.clone(),
                            initiator_node_id: link.initiator_node_id,
                            phase: link.phase,
                            evidence: if current {
                                LinkEvidence::OneSided
                            } else {
                                LinkEvidence::Expired
                            },
                            reports: vec![reference.clone()],
                            in_use: current.then_some(link.business_dependencies > 0),
                            since: link.since,
                        },
                    );
                }
            }
        }
        for (path_id, state) in &report.paths {
            paths.push(AggregatedPath {
                path_id: path_id.clone(),
                reporter: reference.clone(),
                state: state.clone(),
            });
        }
    }
    (links.into_values().collect(), paths)
}
pub(crate) mod progress;
pub mod provider;
