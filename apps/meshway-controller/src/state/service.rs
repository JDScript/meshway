//! Integration seam for management and Node identity owners. Providers execute
//! on the real SQLite transaction, never from a cached HTTP authorization result.
use super::{ReportAdmission, StateError, StateHub, VisibilityScope, stream::*};
use crate::{observations::Observations, resources::GeoIp, store::Handle};
use chrono::{DateTime, Duration, Utc};
use meshway_model::{Digest, NetworkId, NodeId};
use meshway_protocol::{
    events::{ResetReason, StreamEvent},
    state::{StateAccepted, StateReport, StateView},
};
use rusqlite::Transaction;
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

/// The Node credential encoding/verification is owned by B. This enum is never
/// serialized or logged and is checked again in each provider transaction.
#[derive(Clone)]
pub enum ReadIdentity {
    UserSession {
        digest: [u8; 32],
        network: NetworkId,
    },
    Node {
        authorization: String,
    },
}
impl ReadIdentity {
    pub(crate) fn fingerprint(&self) -> Digest {
        let mut bytes = vec![];
        match self {
            Self::UserSession { digest, network } => {
                bytes.push(0);
                bytes.extend_from_slice(digest);
                bytes.extend_from_slice(network.as_uuid().as_bytes());
            }
            Self::Node { authorization } => {
                bytes.push(1);
                bytes.extend_from_slice(authorization.as_bytes());
            }
        }
        crate::resources::content_digest(&bytes)
    }
}
pub struct ViewContext<C, E, P> {
    pub scope: VisibilityScope,
    pub configuration: C,
    pub entities: E,
    pub progress: Option<P>,
    /// Keys of the members in the typed entity snapshot, for explicit removals.
    pub member_ids: std::collections::BTreeSet<meshway_model::MembershipId>,
}
pub trait StateProvider: Send + Sync + 'static {
    type Configuration: Clone + PartialEq + Serialize + Send + Sync + 'static;
    type Entities: Clone + PartialEq + Serialize + Send + Sync + 'static;
    type Progress: Clone + PartialEq + Serialize + Send + Sync + 'static;
    fn view(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<ProviderView<Self>, StateError>;
    fn report(
        &self,
        tx: &Transaction<'_>,
        authorization: &str,
        now: DateTime<Utc>,
    ) -> Result<ReportAdmission, StateError>;
}
pub type ProviderView<P> = ViewContext<
    <P as StateProvider>::Configuration,
    <P as StateProvider>::Entities,
    <P as StateProvider>::Progress,
>;
type ProviderEvent<P> = StreamEvent<
    <P as StateProvider>::Configuration,
    <P as StateProvider>::Entities,
    <P as StateProvider>::Progress,
>;
struct Subscription<P: StateProvider> {
    identity: ReadIdentity,
    owner: Digest,
    authorization: Digest,
    last: ProviderView<P>,
    state: StateView,
    cursor: String,
    pending: VecDeque<SseFrame>,
    replay: ReplayBuffer,
    disconnected_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    observations: BTreeMap<meshway_model::ObservationId, Digest>,
    journal_sequence: u64,
}
pub struct Runtime<P: StateProvider> {
    closed: bool,
    pub hub: StateHub,
    pub geoip: GeoIp,
    pub observations: Observations,
    pub(crate) observation_identities: BTreeMap<meshway_model::SessionId, ReadIdentity>,
    pub(crate) resource_results:
        BTreeMap<(NetworkId, String), meshway_protocol::resources::RuleSetView>,
    subscriptions: BTreeMap<String, Subscription<P>>,
}
pub struct StateService<P: StateProvider> {
    pub(crate) handle: Handle,
    pub(crate) provider: Arc<P>,
    pub(crate) runtime: Arc<Mutex<Runtime<P>>>,
    sealer: Arc<crate::secrets::Sealer>,
}
impl<P: StateProvider> Clone for StateService<P> {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
            provider: Arc::clone(&self.provider),
            runtime: Arc::clone(&self.runtime),
            sealer: Arc::clone(&self.sealer),
        }
    }
}
impl<P: StateProvider> StateService<P> {
    pub fn reader(&self) -> StateReader<P> {
        StateReader {
            runtime: Arc::clone(&self.runtime),
        }
    }
    /// Stop active streams before awaiting HTTP graceful shutdown. App shutdown
    /// and drop also call this; accepted SQLite work still drains independently.
    pub fn close(&self) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.closed = true;
            runtime.subscriptions.clear();
            runtime.observations = Observations::default();
            runtime.observation_identities.clear();
        }
    }
    /// Reinstalls only immutable datasets. Volatile Node reports, cursors and
    /// observation tasks are deliberately not restored as current evidence.
    pub async fn restore_ip_datasets(&self) -> Result<(), StateError> {
        let runtime = self.runtime.clone();
        self.handle.execute(move|connection| {
            use rusqlite::OptionalExtension;
            let tx=connection.transaction()?;
            let mut runtime=runtime.lock().map_err(|_|StateError::Unavailable)?;
            for kind in [IpDatasetKind::GeoIp,IpDatasetKind::Asn] {
                let row: Option<(String,Vec<u8>,String)> = tx.query_row("SELECT id,data,digest FROM dataset_artifacts WHERE network_id IS NULL AND kind=?1 AND format='mmdb' ORDER BY created_at DESC,id DESC LIMIT 1",[match kind { IpDatasetKind::GeoIp=>"geoip",IpDatasetKind::Asn=>"asn" }],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
                if let Some((id,bytes,digest))=row {
                    if crate::resources::content_digest(&bytes).to_string()!=digest{return Err(StateError::Storage);}
                    let id=id.parse().map_err(|_|StateError::Storage)?;
                    match kind { IpDatasetKind::GeoIp=>runtime.geoip.install_city(id,bytes)?,IpDatasetKind::Asn=>runtime.geoip.install_asn(id,bytes)? }
                }
            }
            Ok(())
        }).await?
    }
    /// Deployment-only integration API; deliberately has no network HTTP route.
    /// Validate the complete MMDB before atomically saving its immutable bytes.
    pub async fn install_ip_dataset(
        &self,
        kind: IpDatasetKind,
        source: String,
        bytes: Vec<u8>,
    ) -> Result<meshway_model::ArtifactId, StateError> {
        if source.is_empty() || source.len() > 256 || source.chars().any(char::is_control) {
            return Err(StateError::Invalid);
        }
        let id = meshway_model::ArtifactId::new(
            crate::accounts::random_uuid().map_err(|_| StateError::Unavailable)?,
        )
        .map_err(|_| StateError::Unavailable)?;
        let mut checked = GeoIp::default();
        match kind {
            IpDatasetKind::GeoIp => checked.install_city(id, bytes.clone())?,
            IpDatasetKind::Asn => checked.install_asn(id, bytes.clone())?,
        }
        let digest = crate::resources::content_digest(&bytes);
        let runtime = self.runtime.clone();
        self.handle.execute(move|connection| {
            let tx=connection.transaction()?;let now=Utc::now();
            tx.execute("INSERT INTO dataset_artifacts(id,network_id,kind,source,format,data,digest,created_at) VALUES (?1,NULL,?2,?3,'mmdb',?4,?5,?6)",rusqlite::params![id.to_string(),match kind{IpDatasetKind::GeoIp=>"geoip",IpDatasetKind::Asn=>"asn"},source,bytes,digest.to_string(),now.timestamp_millis()])?;
            tx.commit()?;
            let mut runtime=runtime.lock().map_err(|_|StateError::Unavailable)?;
            match kind { IpDatasetKind::GeoIp=>runtime.geoip.install_city(id,bytes)?,IpDatasetKind::Asn=>runtime.geoip.install_asn(id,bytes)? }
            Ok(id)
        }).await?
    }
    pub fn publisher(&self) -> CommitPublisher<P> {
        CommitPublisher {
            runtime: Arc::clone(&self.runtime),
        }
    }
    pub(crate) fn new(
        handle: Handle,
        provider: P,
        sealer: Arc<crate::secrets::Sealer>,
    ) -> Result<Self, StateError> {
        Ok(Self {
            handle,
            provider: Arc::new(provider),
            sealer,
            runtime: Arc::new(Mutex::new(Runtime {
                closed: false,
                hub: StateHub::new()?,
                geoip: GeoIp::default(),
                observations: Observations::default(),
                observation_identities: BTreeMap::new(),
                resource_results: BTreeMap::new(),
                subscriptions: BTreeMap::new(),
            })),
        })
    }
    pub async fn accept_history(
        &self,
        authorization: String,
        report: meshway_protocol::observations::HistoryReport,
    ) -> Result<meshway_protocol::observations::HistoryAccepted, StateError> {
        let provider = Arc::clone(&self.provider);
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let admission = provider.report(&tx, &authorization, now)?;
                let result = crate::observations::history::accept(&tx, &admission, report, now)?;
                tx.commit()?;
                Ok(result)
            })
            .await?
    }
    pub async fn history(
        &self,
        identity: ReadIdentity,
        query: meshway_protocol::observations::HistoryQuery,
    ) -> Result<meshway_protocol::observations::HistoryPage, StateError> {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
        let provider = Arc::clone(&self.provider);
        let sealer = Arc::clone(&self.sealer);
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let view = provider.view(&tx, &identity, now)?;
                let scope = crate::secrets::CursorScope::ObservationHistory(identity.fingerprint());
                let previous = match &query.cursor {
                    None => None,
                    Some(cursor) => {
                        if cursor.len() > 4096 {
                            return Err(StateError::Cursor);
                        }
                        let bytes = URL_SAFE_NO_PAD
                            .decode(cursor)
                            .map_err(|_| StateError::Cursor)?;
                        let plain = sealer
                            .open_cursor(scope, &bytes)
                            .map_err(|_| StateError::Cursor)?;
                        Some(
                            serde_json::from_slice(plain.as_bytes())
                                .map_err(|_| StateError::Cursor)?,
                        )
                    }
                };
                let (mut page, next) =
                    crate::observations::history::query(&tx, &view.scope, &query, previous, now)?;
                if let Some(next) = next {
                    let bytes = serde_json::to_vec(&next).map_err(|_| StateError::Unavailable)?;
                    page.next_cursor = Some(
                        URL_SAFE_NO_PAD.encode(
                            sealer
                                .seal_cursor(scope, &bytes)
                                .map_err(|_| StateError::Unavailable)?
                                .as_bytes(),
                        ),
                    );
                }
                Ok(page)
            })
            .await?
    }
    pub async fn query(
        &self,
        identity: ReadIdentity,
        nodes: Option<Vec<NodeId>>,
    ) -> Result<StateView, StateError> {
        let provider = Arc::clone(&self.provider);
        let runtime = Arc::clone(&self.runtime);
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let view = provider.view(&tx, &identity, now)?;
                let mut runtime = runtime.lock().map_err(|_| StateError::Unavailable)?;
                let Runtime { hub, geoip, .. } = &mut *runtime;
                hub.query(&view.scope, nodes.as_deref(), geoip, now)
            })
            .await?
    }
    pub async fn accept_report(
        &self,
        authorization: String,
        report: StateReport,
    ) -> Result<StateAccepted, StateError> {
        let provider = Arc::clone(&self.provider);
        let runtime = Arc::clone(&self.runtime);
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let admission = provider.report(&tx, &authorization, now)?;
                let mut runtime = runtime.lock().map_err(|_| StateError::Unavailable)?;
                runtime.hub.accept_report(&admission, report, now)
            })
            .await?
    }
    pub async fn open_stream(
        &self,
        identity: ReadIdentity,
        cursor: Option<String>,
    ) -> Result<String, StateError> {
        let provider = Arc::clone(&self.provider);
        let runtime = Arc::clone(&self.runtime);
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let view = provider.view(&tx, &identity, now)?;
                let mut runtime = runtime.lock().map_err(|_| StateError::Unavailable)?;
                runtime.subscriptions.retain(|_, sub| {
                    sub.created_at + Duration::minutes(15) > now
                        && sub
                            .disconnected_at
                            .is_none_or(|at| at + Duration::minutes(1) > now)
                });
                if runtime.closed {
                    return Err(StateError::Unavailable);
                }
                if runtime.subscriptions.len() >= 256 {
                    return Err(StateError::Budget);
                }
                let owner = identity.fingerprint();
                if runtime
                    .subscriptions
                    .values()
                    .filter(|s| s.owner == owner)
                    .count()
                    >= 4
                {
                    return Err(StateError::Budget);
                }
                let id = random_id()?;
                if let Some(cursor) = &cursor {
                    let previous = runtime.subscriptions.iter().find_map(|(id, sub)| {
                        (sub.owner == owner
                            && sub.authorization == view.scope.authorization_digest
                            && sub.replay.after(cursor).is_ok())
                        .then(|| id.clone())
                    });
                    if let Some(previous) = previous {
                        let mut sub = runtime
                            .subscriptions
                            .remove(&previous)
                            .ok_or(StateError::Cursor)?;
                        let mut pending: VecDeque<_> = sub.replay.after(cursor)?.into();
                        pending.append(&mut sub.pending);
                        sub.pending = pending;
                        sub.disconnected_at = None;
                        runtime.subscriptions.insert(id.clone(), sub);
                        return Ok(id);
                    }
                }
                let Runtime { hub, geoip, .. } = &mut *runtime;
                let state = hub.query(&view.scope, None, geoip, now)?;
                let ready = random_id()?;
                let mut sub = Subscription {
                    identity,
                    owner,
                    authorization: view.scope.authorization_digest,
                    last: view,
                    state,
                    cursor: ready.clone(),
                    pending: VecDeque::new(),
                    replay: ReplayBuffer::default(),
                    disconnected_at: None,
                    created_at: now,
                    observations: BTreeMap::new(),
                    journal_sequence: hub.events.sequence(),
                };
                if cursor.is_some() {
                    sub.pending.push_back(SseFrame::encode(
                        None,
                        &ProviderEvent::<P>::Reset {
                            reason: ResetReason::CursorTooOld,
                        },
                    )?);
                }
                initial::<P>(&mut sub, ready)?;
                runtime.subscriptions.insert(id.clone(), sub);
                Ok(id)
            })
            .await?
    }
    async fn poll_frame(&self, id: String) -> Result<StreamPoll, StateError> {
        let provider = Arc::clone(&self.provider);
        let runtime = Arc::clone(&self.runtime);
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let mut runtime = runtime.lock().map_err(|_| StateError::Unavailable)?;
                let Some(mut sub) = runtime.subscriptions.remove(&id) else {
                    return Ok(StreamPoll::Closed);
                };
                // Remove-first ensures all failure paths discard pending old-scope
                // frames. There is no send queue outside this guarded poll.
                let view = match provider.view(&tx, &sub.identity, now) {
                    Ok(view) => view,
                    Err(_) => return Ok(StreamPoll::Closed),
                };
                if view.scope.expires_at <= now
                    || sub.created_at + Duration::minutes(15) <= now
                    || sub.authorization != view.scope.authorization_digest
                {
                    return Ok(StreamPoll::Closed);
                }
                let Runtime { hub, geoip, .. } = &mut *runtime;
                let state = hub.query(&view.scope, None, geoip, now)?;
                let journal = hub
                    .events
                    .after(sub.journal_sequence, view.scope.network_id);
                let journal_sequence = hub.events.sequence();
                if journal.is_err()
                    || (!sub.pending.is_empty()
                        && (view.configuration != sub.last.configuration
                            || view.entities != sub.last.entities
                            || view.scope.configuration_revision
                                != sub.last.scope.configuration_revision))
                {
                    sub.pending.clear();
                    sub.replay.clear();
                    sub.pending.push_back(SseFrame::encode(
                        None,
                        &ProviderEvent::<P>::Reset {
                            reason: if journal.is_err() {
                                ResetReason::CursorTooOld
                            } else {
                                ResetReason::ViewChanged
                            },
                        },
                    )?);
                    sub.last = view;
                    sub.state = state;
                    let ready = random_id()?;
                    sub.cursor = ready.clone();
                    sub.journal_sequence = journal_sequence;
                    initial::<P>(&mut sub, ready)?;
                } else if sub.pending.is_empty() {
                    let mut changed = false;
                    runtime.observations.expire(now);
                    let tasks = runtime.observations.visible_tasks(
                        view.scope.network_id,
                        &view.scope,
                        &runtime.observation_identities,
                        &sub.identity,
                    );
                    let visible_tasks: std::collections::BTreeSet<_> =
                        tasks.iter().map(|task| task.id).collect();
                    let mut revision = sub.last.scope.configuration_revision;
                    for entry in journal? {
                        match entry.change {
                            CommittedChange::Configuration { notice }
                                if notice.revision > revision
                                    && notice.previous_revision == revision =>
                            {
                                revision = notice.revision;
                                enqueue::<P>(
                                    &mut sub,
                                    ProviderEvent::<P>::ConfigurationChanged {
                                        network_id: notice.network_id,
                                        previous_revision: notice.previous_revision,
                                        revision: notice.revision,
                                        changed_at: notice.changed_at,
                                        cause: notice.cause,
                                        actor: notice.actor,
                                        request_id: notice.request_id,
                                        changed_pointers: notice.changed_pointers,
                                        changes_complete: notice.changes_complete,
                                        source_changed: notice.source_changed,
                                    },
                                )?;
                                changed = true;
                            }
                            CommittedChange::ObservationSample { observation, .. }
                                if visible_tasks.contains(&observation.id)
                                    && observation.path.as_ref().is_none_or(|path| {
                                        path.iter().all(|node| view.scope.visible.contains(node))
                                    }) =>
                            {
                                enqueue::<P>(
                                    &mut sub,
                                    ProviderEvent::<P>::ObservationSample { observation },
                                )?;
                                changed = true;
                            }
                            _ => {}
                        }
                    }
                    sub.journal_sequence = journal_sequence;
                    if revision != view.scope.configuration_revision {
                        sub.pending.clear();
                        sub.replay.clear();
                        sub.pending.push_back(SseFrame::encode(
                            None,
                            &ProviderEvent::<P>::Reset {
                                reason: ResetReason::ViewChanged,
                            },
                        )?);
                        sub.last = view;
                        sub.state = state;
                        let ready = random_id()?;
                        sub.cursor = ready.clone();
                        initial::<P>(&mut sub, ready)?;
                        let frame = sub.pending.pop_front().ok_or(StateError::Unavailable)?;
                        runtime.subscriptions.insert(id, sub);
                        return Ok(StreamPoll::Frame(frame));
                    }
                    for task in tasks {
                        let digest = crate::resources::content_digest(
                            &serde_json::to_vec(&task).map_err(|_| StateError::Invalid)?,
                        );
                        if sub.observations.get(&task.id) != Some(&digest) {
                            sub.observations.insert(task.id, digest);
                            enqueue::<P>(
                                &mut sub,
                                ProviderEvent::<P>::ObservationStatus {
                                    observation: Box::new(task),
                                },
                            )?;
                            changed = true;
                        }
                    }
                    if view.entities != sub.last.entities {
                        let next = random_id()?;
                        let event = ProviderEvent::<P>::EntitiesDelta {
                            base_cursor: sub.cursor.clone(),
                            cursor: next.clone(),
                            updated: view.entities.clone(),
                            removed: meshway_protocol::events::EntityRemovals {
                                nodes: sub
                                    .last
                                    .scope
                                    .visible
                                    .difference(&view.scope.visible)
                                    .copied()
                                    .collect(),
                                members: sub
                                    .last
                                    .member_ids
                                    .difference(&view.member_ids)
                                    .copied()
                                    .collect(),
                            },
                        };
                        enqueue_at::<P>(&mut sub, event, next)?;
                        changed = true;
                    }
                    if state.nodes != sub.state.nodes
                        || state.links != sub.state.links
                        || state.paths != sub.state.paths
                    {
                        let updated = state
                            .nodes
                            .iter()
                            .filter(|(node, value)| sub.state.nodes.get(node) != Some(*value))
                            .map(|(node, value)| (*node, value.clone()))
                            .collect();
                        let removed = sub
                            .state
                            .nodes
                            .keys()
                            .filter(|node| !state.nodes.contains_key(node))
                            .copied()
                            .collect();
                        let next = random_id()?;
                        let event = ProviderEvent::<P>::StateDelta {
                            state_epoch: state.state_epoch,
                            base_cursor: sub.cursor.clone(),
                            cursor: next.clone(),
                            updated,
                            removed,
                            links: state.links.clone(),
                            paths: state.paths.clone(),
                        };
                        enqueue_at::<P>(&mut sub, event, next)?;
                        changed = true;
                    }
                    if view.progress != sub.last.progress
                        && let Some(progress) = &view.progress
                    {
                        enqueue::<P>(
                            &mut sub,
                            ProviderEvent::<P>::ConfigurationProgress {
                                progress: progress.clone(),
                            },
                        )?;
                        changed = true;
                    }
                    sub.last = view;
                    sub.state = state;
                    if !changed {
                        runtime.subscriptions.insert(id, sub);
                        return Ok(StreamPoll::Pending);
                    }
                }
                let frame = sub.pending.pop_front().ok_or(StateError::Unavailable)?;
                if frame.id.is_some() {
                    sub.replay.push(frame.clone());
                }
                runtime.subscriptions.insert(id, sub);
                Ok(StreamPoll::Frame(frame))
            })
            .await?
    }
}
#[derive(Clone, Copy)]
pub enum IpDatasetKind {
    GeoIp,
    Asn,
}
/// Clone this handle into A/B's accepted SQLite closures. Publish directly
/// after COMMIT, never in the awaiting HTTP future.
pub struct CommitPublisher<P: StateProvider> {
    runtime: Arc<Mutex<Runtime<P>>>,
}
/// Synchronous execution input for B. Call after B has validated the supplied
/// admission in the same transaction; no HTTP source address participates.
pub struct StateReader<P: StateProvider> {
    runtime: Arc<Mutex<Runtime<P>>>,
}
impl<P: StateProvider> Clone for StateReader<P> {
    fn clone(&self) -> Self {
        Self {
            runtime: Arc::clone(&self.runtime),
        }
    }
}
impl<P: StateProvider> StateReader<P> {
    pub fn location(
        &self,
        admission: &ReportAdmission,
        now: DateTime<Utc>,
    ) -> Result<meshway_protocol::state::DerivedNodeState, StateError> {
        let mut runtime = self.runtime.lock().map_err(|_| StateError::Unavailable)?;
        if runtime.closed {
            return Err(StateError::Unavailable);
        }
        let Runtime { hub, geoip, .. } = &mut *runtime;
        Ok(hub.location(admission, geoip, now))
    }
}
impl<P: StateProvider> Clone for CommitPublisher<P> {
    fn clone(&self) -> Self {
        Self {
            runtime: Arc::clone(&self.runtime),
        }
    }
}
impl<P: StateProvider> CommitPublisher<P> {
    /// Preferred A/B write boundary: invoke inside the accepted store job.
    /// A failed transaction emits nothing; cancellation of the HTTP waiter
    /// cannot interrupt this synchronous commit-to-journal sequence.
    pub fn commit(
        &self,
        transaction: Transaction<'_>,
        changes: Vec<CommittedChange>,
    ) -> Result<(), StateError> {
        if changes.len() > MAX_REPLAY_EVENTS {
            return Err(StateError::Budget);
        }
        transaction.commit()?;
        for change in changes {
            self.publish_committed(change);
        }
        Ok(())
    }
    pub fn publish_committed(&self, change: CommittedChange) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.hub.publish_committed(change);
        }
    }
    pub fn revoke_session(&self, session: meshway_model::SessionId) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.observations.revoke_session(session);
            runtime.observation_identities.remove(&session);
        }
    }
}
fn random_id() -> Result<String, StateError> {
    Ok(crate::accounts::random_uuid()
        .map_err(|_| StateError::Unavailable)?
        .to_string())
}
fn initial<P: StateProvider>(sub: &mut Subscription<P>, batch: String) -> Result<(), StateError> {
    sub.pending.push_back(SseFrame::encode(
        None,
        &ProviderEvent::<P>::ConfigSnapshot {
            batch: batch.clone(),
            configuration: sub.last.configuration.clone(),
            progress: sub.last.progress.clone(),
        },
    )?);
    sub.pending.push_back(SseFrame::encode(
        None,
        &ProviderEvent::<P>::EntitiesSnapshot {
            batch: batch.clone(),
            entities: sub.last.entities.clone(),
        },
    )?);
    sub.pending.push_back(SseFrame::encode(
        None,
        &ProviderEvent::<P>::StateSnapshot {
            batch: batch.clone(),
            state: sub.state.clone(),
        },
    )?);
    sub.pending.push_back(SseFrame::encode(
        Some(batch.clone()),
        &ProviderEvent::<P>::Ready {
            batch: batch.clone(),
            cursor: batch,
            heartbeat_interval_ms: 10_000,
            disconnect_after_ms: 30_000,
        },
    )?);
    if sub
        .pending
        .iter()
        .map(|frame| frame.data.len())
        .sum::<usize>()
        > MAX_BATCH_BYTES
    {
        return Err(StateError::Budget);
    }
    Ok(())
}
fn enqueue<P: StateProvider>(
    sub: &mut Subscription<P>,
    event: ProviderEvent<P>,
) -> Result<(), StateError> {
    let next = random_id()?;
    enqueue_at::<P>(sub, event, next)
}
fn enqueue_at<P: StateProvider>(
    sub: &mut Subscription<P>,
    event: ProviderEvent<P>,
    next: String,
) -> Result<(), StateError> {
    let frame = SseFrame::encode(Some(next.clone()), &event)?;
    if sub.pending.len() >= MAX_REPLAY_EVENTS
        || sub
            .pending
            .iter()
            .map(|frame| frame.data.len())
            .sum::<usize>()
            + frame.data.len()
            > MAX_BATCH_BYTES
    {
        return Err(StateError::Budget);
    }
    sub.pending.push_back(frame);
    sub.cursor = next;
    Ok(())
}
impl<P: StateProvider> StreamSource for StateService<P> {
    fn poll<'a>(&'a self, subscription: &'a str) -> PollFuture<'a> {
        Box::pin(async move {
            match self.poll_frame(subscription.to_owned()).await {
                Err(StateError::Budget) => Ok(StreamPoll::Frame(SseFrame::encode(
                    None,
                    &ProviderEvent::<P>::Reset {
                        reason: ResetReason::Backpressure,
                    },
                )?)),
                result => result,
            }
        })
    }
    fn cancel(&self, subscription: &str) {
        if let Ok(mut runtime) = self.runtime.lock()
            && let Some(sub) = runtime.subscriptions.get_mut(subscription)
        {
            sub.disconnected_at = Some(Utc::now());
        }
    }
}
