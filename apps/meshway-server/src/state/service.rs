//! Integration seam for management and Node identity owners. Providers execute
//! on the real SQLite transaction, never from a cached HTTP authorization result.
use super::{ReportAdmission, StateHub, VisibilityScope, stream::*};
use crate::effect::Effect;
use crate::error::Error;
use crate::{observations::Observations, resources::GeoIp, store::Handle};
use chrono::{DateTime, Duration, Utc};
use meshway_model::{Digest, NetworkId, NodeId, RequestId};
use meshway_protocol::{
    events::{ClosedReason, ResetReason, StreamEvent},
    state::{StateAccepted, StateReport, StateView},
};
use rusqlite::{Transaction, TransactionBehavior};
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

/// Node credential encoding/verification lives in `execution`. This enum is never
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
    /// Parse a session credential without granting authorization. Providers
    /// still recheck the actual session and membership on every transaction.
    pub fn user_session(token: &str, network: NetworkId) -> Result<Self, Error> {
        let digest = crate::accounts::token_digest(token)?;
        Ok(Self::UserSession { digest, network })
    }
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
    /// Scope of data that may remain in replay, separate from the evolving
    /// execution/observation authorization digest used inside the State core.
    fn replay_scope(&self, view: &ProviderView<Self>) -> Result<Digest, Error> {
        Ok(view.scope.authorization_digest)
    }
    /// Managers may replay prior non-secret network views in order before the
    /// next notice. Restricted readers must recheck target-dependent queues.
    fn can_drain_configuration_changes(&self, _view: &ProviderView<Self>) -> bool {
        false
    }
    /// A restricted settings view has no management GET refresh. Replace its
    /// complete batch when the authorized settings themselves change.
    fn configuration_refresh_required(
        &self,
        _previous: &ProviderView<Self>,
        _current: &ProviderView<Self>,
    ) -> bool {
        false
    }
    /// Identity/role loss closes the connection. A still-authorized view change
    /// clears replay/pending and synchronizes again instead.
    fn same_authority(&self, previous: &ProviderView<Self>, current: &ProviderView<Self>) -> bool {
        previous.scope.authorization_digest == current.scope.authorization_digest
    }
    fn filter_notice(
        &self,
        _view: &ProviderView<Self>,
        notice: meshway_protocol::events::ConfigurationNotice,
    ) -> meshway_protocol::events::ConfigurationNotice {
        notice
    }
    fn view(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<ProviderView<Self>, Error>;
    fn report(
        &self,
        tx: &Transaction<'_>,
        authorization: &str,
        now: DateTime<Utc>,
    ) -> Result<ReportAdmission, Error>;
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
    request_id: RequestId,
    /// Wake-up channels and the earliest self-computed deadline for the next
    /// projection; see `wake`.
    listener: super::wake::Listener,
    deadline: Option<DateTime<Utc>>,
    owner: Digest,
    authorization: Digest,
    last: ProviderView<P>,
    state: StateView,
    cursor: String,
    pending: VecDeque<SseFrame>,
    replay: ReplayBuffer,
    disconnected_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    observations: BTreeMap<meshway_model::ObservationSubscriptionId, Digest>,
    active_observations: BTreeMap<meshway_model::ObservationSubscriptionId, Digest>,
    journal_sequence: u64,
    initial_batch: Option<String>,
}
pub(crate) struct Runtime {
    closed: bool,
    pub hub: StateHub,
    pub geoip: GeoIp,
    pub observations: Observations,
    pub traversal: crate::connectivity::Attempts,
    /// Peer facts disclosed through Node `sync`; never a compilation input.
    pub(crate) facts: super::facts::FactLedger,
    /// Netmap store owner's reader; scopes fact recording and disclosure.
    pub(crate) netmaps: Arc<dyn super::facts::NetmapReader>,
}
/// The executor applies committed effects here, on its own thread, after COMMIT.
impl crate::effect::EffectSink for Mutex<Runtime> {
    fn apply(&self, effect: crate::effect::Effect) {
        let Ok(mut runtime) = self.lock() else {
            return;
        };
        match effect {
            Effect::AcceptedState {
                admission,
                report,
                observed_at,
            } => {
                // The same serialized owner prechecked this exact body and time.
                if runtime
                    .hub
                    .accept_report(&admission, *report, observed_at)
                    .is_err()
                {
                    tracing::error!("Prechecked State report could not be committed");
                    runtime.close();
                }
            }
            crate::effect::Effect::SessionRevoked { session_id } => {
                runtime.observations.revoke_session(session_id);
                runtime.hub.wakeups.bump(None);
            }
            Effect::PeerFacts {
                network_id,
                node_id,
                facts,
            } => {
                let recorded = match facts {
                    Some(facts) => runtime.facts.record(network_id, *facts, Utc::now()),
                    None => runtime.facts.remove(network_id, node_id),
                };
                if recorded.is_err() {
                    tracing::error!("Peer facts could not be recorded");
                    runtime.close();
                }
            }
            journaled => {
                if let Effect::NodeRetired {
                    network_id,
                    node_id,
                } = &journaled
                    && runtime.facts.remove(*network_id, *node_id).is_err()
                {
                    tracing::error!("Retired Node facts could not be withdrawn");
                    runtime.close();
                }
                runtime.hub.publish_committed(journaled)
            }
        }
    }
}
impl Runtime {
    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }
    pub(crate) fn close(&mut self) {
        self.closed = true;
        self.observations = Observations::default();
        self.traversal = crate::connectivity::Attempts::default();
        self.facts = super::facts::FactLedger::new(self.hub.epoch);
        self.hub.wakeups.bump(None);
    }
    pub(crate) fn new(netmaps: Arc<dyn super::facts::NetmapReader>) -> Result<Self, Error> {
        let hub = StateHub::new()?;
        Ok(Self {
            closed: false,
            facts: super::facts::FactLedger::new(hub.epoch),
            hub,
            geoip: GeoIp::default(),
            observations: Observations::default(),
            traversal: crate::connectivity::Attempts::default(),
            netmaps,
        })
    }
}
pub struct StateService<P: StateProvider> {
    pub(crate) handle: Handle,
    pub(crate) provider: Arc<P>,
    pub(crate) runtime: Arc<Mutex<Runtime>>,
    subscriptions: Arc<Mutex<BTreeMap<String, Subscription<P>>>>,
    sealer: Arc<crate::secrets::Sealer>,
}
impl<P: StateProvider> Clone for StateService<P> {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
            provider: Arc::clone(&self.provider),
            runtime: Arc::clone(&self.runtime),
            subscriptions: Arc::clone(&self.subscriptions),
            sealer: Arc::clone(&self.sealer),
        }
    }
}
impl<P: StateProvider> StateService<P> {
    pub fn reader(&self) -> StateReader {
        StateReader::new(Arc::clone(&self.runtime))
    }
    /// Stop active streams before awaiting HTTP graceful shutdown. App shutdown
    /// and drop also call this; accepted SQLite work still drains independently.
    pub fn close(&self) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.close();
        }
    }
    pub(crate) fn new(
        handle: Handle,
        provider: P,
        sealer: Arc<crate::secrets::Sealer>,
        runtime: Arc<Mutex<Runtime>>,
    ) -> Result<Self, Error> {
        Ok(Self {
            handle,
            provider: Arc::new(provider),
            sealer,
            runtime,
            subscriptions: Arc::default(),
        })
    }
    pub async fn accept_history(
        &self,
        authorization: String,
        report: meshway_protocol::observations::HistoryReport,
    ) -> Result<meshway_protocol::observations::HistoryAccepted, Error> {
        let provider = Arc::clone(&self.provider);
        self.handle
            .transact(TransactionBehavior::Deferred, move |tx| {
                let now = Utc::now();
                let admission = provider.report(tx, &authorization, now)?;
                let result = crate::observations::history::accept(tx, &admission, report, now)?;
                Ok((result, Vec::new()))
            })
            .await
    }
    pub async fn history(
        &self,
        identity: ReadIdentity,
        query: meshway_protocol::observations::HistoryQuery,
    ) -> Result<meshway_protocol::observations::HistoryPage, Error> {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
        let provider = Arc::clone(&self.provider);
        let sealer = Arc::clone(&self.sealer);
        self.handle
            .read(move |tx| {
                let now = Utc::now();
                let view = provider.view(tx, &identity, now)?;
                let scope = crate::secrets::CursorScope::ObservationHistory(identity.fingerprint());
                let previous = match &query.cursor {
                    None => None,
                    Some(cursor) => {
                        if cursor.len() > 4096 {
                            return Err(Error::InvalidCursor);
                        }
                        let bytes = URL_SAFE_NO_PAD
                            .decode(cursor)
                            .map_err(|_| Error::InvalidCursor)?;
                        let plain = sealer
                            .open_cursor(scope, &bytes)
                            .map_err(|_| Error::InvalidCursor)?;
                        Some(
                            serde_json::from_slice(plain.as_bytes())
                                .map_err(|_| Error::InvalidCursor)?,
                        )
                    }
                };
                let (mut page, next) =
                    crate::observations::history::query(tx, &view.scope, &query, previous, now)?;
                if let Some(next) = next {
                    let bytes = serde_json::to_vec(&next)
                        .map_err(|_| Error::encode("observation history cursor"))?;
                    page.next_cursor =
                        Some(URL_SAFE_NO_PAD.encode(sealer.seal_cursor(scope, &bytes)?.as_bytes()));
                }
                Ok(page)
            })
            .await
    }
    pub async fn query(
        &self,
        identity: ReadIdentity,
        nodes: Option<Vec<NodeId>>,
    ) -> Result<StateView, Error> {
        let provider = Arc::clone(&self.provider);
        let runtime = Arc::clone(&self.runtime);
        self.handle
            .read(move |tx| {
                let now = Utc::now();
                let view = provider.view(tx, &identity, now)?;
                let mut runtime = runtime.lock().map_err(|_| Error::RUNTIME)?;
                let Runtime { hub, geoip, .. } = &mut *runtime;
                hub.query(&view.scope, nodes.as_deref(), geoip, now)
            })
            .await
    }
    pub async fn accept_report(
        &self,
        authorization: String,
        report: StateReport,
    ) -> Result<StateAccepted, Error> {
        let provider = Arc::clone(&self.provider);
        let runtime = Arc::clone(&self.runtime);
        let sealer = Arc::clone(&self.sealer);
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                let now = Utc::now();
                let admission = provider.report(tx, &authorization, now)?;
                let runtime = runtime.lock().map_err(|_| Error::RUNTIME)?;
                let accepted = runtime.hub.check_report(&admission, &report, now)?;
                let netmaps = Arc::clone(&runtime.netmaps);
                drop(runtime);
                let facts = if accepted.duplicate {
                    None
                } else {
                    // Exit readiness feeds configuration evaluation only.
                    super::exit::replace(tx, &sealer, &admission, &report, now)?;
                    let reporter = netmaps.current(tx, admission.network_id, admission.node_id)?;
                    Some(Effect::PeerFacts {
                        network_id: admission.network_id,
                        node_id: admission.node_id,
                        facts: super::facts::peer_facts(
                            &admission,
                            &report,
                            reporter.as_deref().map(|compiled| &compiled.netmap),
                            now,
                        )
                        .map(Box::new),
                    })
                };
                let mut effects = vec![Effect::AcceptedState {
                    admission: Box::new(admission),
                    report: Box::new(report),
                    observed_at: now,
                }];
                effects.extend(facts);
                Ok((accepted, effects))
            })
            .await
    }
    pub async fn open_stream(
        &self,
        identity: ReadIdentity,
        cursor: Option<String>,
        request_id: RequestId,
    ) -> Result<String, Error> {
        let provider = Arc::clone(&self.provider);
        let runtime = Arc::clone(&self.runtime);
        let subscriptions = Arc::clone(&self.subscriptions);
        self.handle
            .read(move |tx| {
                let now = Utc::now();
                let view = provider.view(tx, &identity, now)?;
                let mut runtime = runtime.lock().map_err(|_| Error::RUNTIME)?;
                let mut subscriptions = subscriptions.lock().map_err(|_| Error::RUNTIME)?;
                let authorization = provider.replay_scope(&view)?;
                subscriptions.retain(|_, sub| {
                    sub.created_at + Duration::minutes(15) > now
                        && sub
                            .disconnected_at
                            .is_none_or(|at| at + Duration::minutes(1) > now)
                });
                if runtime.closed {
                    return Err(Error::CLOSED);
                }
                let owner = identity.fingerprint();
                let id = random_id()?;
                if let Some(cursor) = &cursor {
                    let previous = subscriptions.iter().find_map(|(id, sub)| {
                        (sub.owner == owner
                            && sub.authorization == authorization
                            && sub.replay.after(cursor).is_ok())
                        .then(|| id.clone())
                    });
                    if let Some(previous) = previous {
                        if subscriptions
                            .values()
                            .filter(|sub| sub.owner == owner && sub.disconnected_at.is_none())
                            .count()
                            >= 4
                            && subscriptions[&previous].disconnected_at.is_some()
                        {
                            return Err(Error::StateLimit);
                        }
                        let mut sub = subscriptions
                            .remove(&previous)
                            .ok_or(Error::InvalidCursor)?;
                        let mut pending: VecDeque<_> = sub.replay.after(cursor)?.into();
                        let replayed: std::collections::BTreeSet<_> = pending
                            .iter()
                            .filter_map(|frame| frame.id.clone())
                            .collect();
                        pending.extend(sub.pending.drain(..).filter(|frame| {
                            frame.id.as_ref().is_none_or(|id| !replayed.contains(id))
                        }));
                        sub.pending = pending;
                        sub.disconnected_at = None;
                        sub.request_id = request_id;
                        if sub.pending.len() <= MAX_REPLAY_EVENTS
                            && sub
                                .pending
                                .iter()
                                .map(|frame| frame.data.len())
                                .sum::<usize>()
                                <= MAX_BATCH_BYTES
                        {
                            subscriptions.insert(id.clone(), sub);
                            return Ok(id);
                        }
                        // Resume backlog is bounded too. Fall through to a full
                        // current snapshot rather than dropping a delta silently.
                    }
                }
                // Disconnected replay is bounded storage, not an active stream
                // quota. Evict it before denying a new live connection.
                if subscriptions.len() >= 256
                    && let Some(oldest) = subscriptions
                        .iter()
                        .filter_map(|(id, sub)| sub.disconnected_at.map(|at| (id.clone(), at)))
                        .min_by_key(|(_, at)| *at)
                        .map(|(id, _)| id)
                {
                    subscriptions.remove(&oldest);
                }
                if subscriptions.len() >= 256
                    || subscriptions
                        .values()
                        .filter(|sub| sub.owner == owner && sub.disconnected_at.is_none())
                        .count()
                        >= 4
                {
                    return Err(Error::StateLimit);
                }
                let Runtime { hub, geoip, .. } = &mut *runtime;
                let listener = hub.wakeups.listen(view.scope.network_id);
                let state = hub.query(&view.scope, None, geoip, now)?;
                let ready = random_id()?;
                let mut sub = Subscription {
                    identity,
                    request_id,
                    listener,
                    deadline: None,
                    owner,
                    authorization,
                    last: view,
                    state,
                    cursor: ready.clone(),
                    pending: VecDeque::new(),
                    replay: ReplayBuffer::default(),
                    disconnected_at: None,
                    created_at: now,
                    observations: BTreeMap::new(),
                    active_observations: Default::default(),
                    journal_sequence: hub.events.sequence(),
                    initial_batch: None,
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
                subscriptions.insert(id.clone(), sub);
                Ok(id)
            })
            .await
    }
    async fn poll_frame(&self, id: String) -> Result<StreamPoll, Error> {
        let provider = Arc::clone(&self.provider);
        let runtime = Arc::clone(&self.runtime);
        let subscriptions = Arc::clone(&self.subscriptions);
        self.handle
            .read(move |tx| {
                let now = Utc::now();
                let mut runtime = runtime.lock().map_err(|_| Error::RUNTIME)?;
                let mut subscriptions = subscriptions.lock().map_err(|_| Error::RUNTIME)?;
                let Some(mut sub) = subscriptions.remove(&id) else {
                    return Ok(StreamPoll::Closed);
                };
                // Remove-first ensures all failure paths discard pending old-scope
                // frames. There is no send queue outside this guarded poll.
                // Anything committed from here on resolves the next wait at once.
                sub.listener.mark_seen();
                if runtime.closed {
                    return closed::<P>(ClosedReason::ServerShutdown);
                }
                let view = match provider.view(tx, &sub.identity, now) {
                    Ok(view) => view,
                    Err(Error::InvalidSession | Error::InvalidCredentials) => {
                        return closed::<P>(ClosedReason::SessionExpired);
                    }
                    Err(Error::NotFound | Error::PermissionDenied)
                        if matches!(sub.identity, ReadIdentity::UserSession { .. }) =>
                    {
                        return closed::<P>(ClosedReason::MembershipRevoked);
                    }
                    Err(Error::StateConflict | Error::BindingConflict)
                        if matches!(sub.identity, ReadIdentity::Node { .. }) =>
                    {
                        return closed::<P>(ClosedReason::NodeLeaseExpired);
                    }
                    Err(_) => return Ok(StreamPoll::Closed),
                };
                let authorization = provider.replay_scope(&view)?;
                if view.scope.expires_at <= now {
                    return closed::<P>(match sub.identity {
                        ReadIdentity::UserSession { .. } => ClosedReason::SessionExpired,
                        ReadIdentity::Node { .. } => ClosedReason::NodeLeaseExpired,
                    });
                }
                if sub.created_at + Duration::minutes(15) <= now
                    || !provider.same_authority(&sub.last, &view)
                {
                    return Ok(StreamPoll::Closed);
                }
                runtime.observations.expire(now);
                let tasks = runtime.observations.visible_tasks(
                    view.scope.network_id,
                    &view.scope,
                    &sub.identity,
                );
                let active_observations = runtime.observations.active_subscriptions(&tasks);
                let observation_scope_changed = sub.active_observations != active_observations;
                sub.active_observations = active_observations;
                let Runtime { hub, geoip, .. } = &mut *runtime;
                let state = hub.query(&view.scope, None, geoip, now)?;
                let journal = hub
                    .events
                    .after(sub.journal_sequence, view.scope.network_id);
                let journal_sequence = hub.events.sequence();
                if sub.authorization != authorization
                    || provider.configuration_refresh_required(&sub.last, &view)
                    || journal.is_err()
                    || observation_scope_changed
                        && (!sub.pending.is_empty() || !sub.replay.is_empty())
                    || (!sub.pending.is_empty()
                        && (sub.initial_batch.is_some()
                            || !provider.can_drain_configuration_changes(&view))
                        && (view.configuration != sub.last.configuration
                            || view.entities != sub.last.entities
                            || view.scope.configuration_revision
                                != sub.last.scope.configuration_revision))
                {
                    sub.pending.clear();
                    sub.replay.clear();
                    sub.observations.clear();
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
                    sub.authorization = authorization;
                    sub.state = state;
                    let ready = random_id()?;
                    sub.cursor = ready.clone();
                    sub.journal_sequence = journal_sequence;
                    initial::<P>(&mut sub, ready)?;
                } else if sub.pending.is_empty() {
                    let mut changed = false;
                    let visible_tasks: std::collections::BTreeSet<_> = tasks
                        .iter()
                        .filter(|task| crate::observations::active(task.phase))
                        .map(|task| task.id)
                        .collect();
                    let mut revision = sub.last.scope.configuration_revision;
                    for entry in journal? {
                        match entry.change {
                            Effect::Resource { network_id }
                                if provider.can_drain_configuration_changes(&view) =>
                            {
                                enqueue::<P>(
                                    &mut sub,
                                    ProviderEvent::<P>::ResourcesChanged {
                                        network_id,
                                        revision: view.scope.configuration_revision,
                                    },
                                )?;
                                changed = true;
                            }
                            Effect::Configuration { notice }
                                if notice.revision > revision
                                    && notice.previous_revision == revision =>
                            {
                                revision = notice.revision;
                                let notice = provider.filter_notice(&view, notice);
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
                            Effect::ObservationSample { observation, .. }
                                if visible_tasks.contains(&observation.id)
                                    && observation.path.as_ref().is_none_or(|path| {
                                        path.iter().all(|node| view.scope.visible.contains(node))
                                    }) =>
                            {
                                for task in tasks.iter().filter(|task| {
                                    task.id == observation.id
                                        && task.netmap == observation.netmap
                                        && crate::observations::active(task.phase)
                                }) {
                                    let mut observation = observation.clone();
                                    observation.subscription_id = task.subscription_id;
                                    observation.subscriber_expires_at = task.subscriber_expires_at;
                                    enqueue::<P>(
                                        &mut sub,
                                        ProviderEvent::<P>::ObservationSample { observation },
                                    )?;
                                }
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
                        let frame = sub
                            .pending
                            .pop_front()
                            .ok_or(Error::Internal("pending sse frame missing"))?;
                        subscriptions.insert(id, sub);
                        return Ok(StreamPoll::Frame(frame));
                    }
                    let retained: std::collections::BTreeSet<_> = tasks
                        .iter()
                        .filter_map(|task| task.subscription_id)
                        .collect();
                    let removed: Vec<_> = sub
                        .observations
                        .keys()
                        .filter(|id| !retained.contains(id))
                        .copied()
                        .collect();
                    for subscription_id in removed {
                        sub.observations.remove(&subscription_id);
                        enqueue::<P>(
                            &mut sub,
                            ProviderEvent::<P>::ObservationRemoved { subscription_id },
                        )?;
                        changed = true;
                    }
                    for task in tasks {
                        let subscription = task
                            .subscription_id
                            .ok_or(Error::Internal("observation subscription missing"))?;
                        let digest = crate::resources::content_digest(
                            &serde_json::to_vec(&task)
                                .map_err(|_| Error::encode("observation task"))?,
                        );
                        if sub.observations.get(&subscription) != Some(&digest) {
                            sub.observations.insert(subscription, digest);
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
                        || state.sequence != sub.state.sequence
                        || state.configuration_revision != sub.state.configuration_revision
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
                            as_of: Some(state.as_of),
                            sequence: Some(state.sequence),
                            configuration_revision: Some(state.configuration_revision),
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
                        sub.deadline = next_deadline(
                            &sub,
                            runtime.hub.next_expiration(sub.last.scope.network_id),
                            runtime.observations.next_expiration(),
                            now,
                        );
                        subscriptions.insert(id, sub);
                        return Ok(StreamPoll::Pending);
                    }
                }
                let frame = sub
                    .pending
                    .pop_front()
                    .ok_or(Error::Internal("pending sse frame missing"))?;
                if frame.id.is_some() && frame.id == sub.initial_batch {
                    sub.initial_batch = None;
                }
                if frame.id.is_some() {
                    sub.replay.push(frame.clone());
                }
                subscriptions.insert(id, sub);
                Ok(StreamPoll::Frame(frame))
            })
            .await
    }
}
pub struct StateReader {
    runtime: Arc<Mutex<Runtime>>,
}
impl Clone for StateReader {
    fn clone(&self) -> Self {
        Self {
            runtime: Arc::clone(&self.runtime),
        }
    }
}
impl StateReader {
    pub(crate) fn new(runtime: Arc<Mutex<Runtime>>) -> Self {
        Self { runtime }
    }
    pub fn location(
        &self,
        admission: &ReportAdmission,
        now: DateTime<Utc>,
    ) -> Result<meshway_protocol::state::DerivedNodeState, Error> {
        let mut runtime = self.runtime.lock().map_err(|_| Error::RUNTIME)?;
        if runtime.closed {
            return Err(Error::CLOSED);
        }
        let Runtime { hub, geoip, .. } = &mut *runtime;
        Ok(hub.location(admission, geoip, now))
    }
}
fn random_id() -> Result<String, Error> {
    Ok(crate::accounts::random_uuid()?.to_string())
}
fn closed<P: StateProvider>(reason: ClosedReason) -> Result<StreamPoll, Error> {
    Ok(StreamPoll::Frame(SseFrame::encode(
        None,
        &ProviderEvent::<P>::Closed { reason },
    )?))
}
fn initial<P: StateProvider>(sub: &mut Subscription<P>, batch: String) -> Result<(), Error> {
    sub.initial_batch = Some(batch.clone());
    sub.pending.push_back(SseFrame::encode(
        None,
        &ProviderEvent::<P>::ConfigSnapshot {
            batch: batch.clone(),
            request_id: sub.request_id,
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
        return Err(Error::StateLimit);
    }
    Ok(())
}
fn enqueue<P: StateProvider>(
    sub: &mut Subscription<P>,
    event: ProviderEvent<P>,
) -> Result<(), Error> {
    let next = random_id()?;
    enqueue_at::<P>(sub, event, next)
}
fn enqueue_at<P: StateProvider>(
    sub: &mut Subscription<P>,
    event: ProviderEvent<P>,
    next: String,
) -> Result<(), Error> {
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
        return Err(Error::StateLimit);
    }
    sub.pending.push_back(frame);
    sub.cursor = next;
    Ok(())
}
/// When this subscription's view changes without any commit: a report, lease,
/// endpoint or observation reaching its expiry, the scope or the subscription
/// itself running out. Capped so authorization is rechecked at least every
/// minute even on a silent network.
fn next_deadline<P: StateProvider>(
    sub: &Subscription<P>,
    reports: Option<DateTime<Utc>>,
    observations: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let scope = &sub.last.scope;
    let cap = now + Duration::seconds(60);
    [
        Some(scope.expires_at),
        Some(sub.created_at + Duration::minutes(15)),
        reports,
        observations,
        Some(cap),
    ]
    .into_iter()
    .flatten()
    .chain(
        scope
            .current
            .values()
            .filter_map(|node| node.lease_expires_at),
    )
    .chain(
        sub.state
            .nodes
            .values()
            .filter_map(|node| node.report_expires_at),
    )
    .filter(|at| *at > now)
    .min()
}

impl<P: StateProvider> StreamSource for StateService<P> {
    fn wait<'a>(&'a self, subscription: &'a str) -> WaitFuture<'a> {
        let pending = self.subscriptions.lock().ok().and_then(|subscriptions| {
            subscriptions
                .get(subscription)
                .map(|sub| (sub.listener.clone(), sub.deadline))
        });
        Box::pin(async move {
            let Some((mut listener, deadline)) = pending else {
                return;
            };
            let sleep = async {
                match deadline {
                    Some(at) => {
                        let remaining = (at - Utc::now()).to_std().unwrap_or_default();
                        tokio::time::sleep(remaining).await;
                    }
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                _ = listener.changed() => {}
                _ = sleep => {}
            }
        })
    }
    fn poll<'a>(&'a self, subscription: &'a str) -> PollFuture<'a> {
        Box::pin(async move {
            match self.poll_frame(subscription.to_owned()).await {
                Err(Error::StateLimit) => Ok(StreamPoll::Frame(SseFrame::encode(
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
        if let Ok(mut subscriptions) = self.subscriptions.lock() {
            let Some(sub) = subscriptions.get_mut(subscription) else {
                return;
            };
            sub.disconnected_at = Some(Utc::now());
            let owner = sub.owner;
            let mut retained: Vec<_> = subscriptions
                .iter()
                .filter(|(_, sub)| sub.owner == owner)
                .filter_map(|(id, sub)| sub.disconnected_at.map(|at| (id.clone(), at)))
                .collect();
            retained.sort_by_key(|(_, at)| *at);
            let excess = retained.len().saturating_sub(4);
            for (id, _) in retained.into_iter().take(excess) {
                subscriptions.remove(&id);
            }
        }
    }
}
