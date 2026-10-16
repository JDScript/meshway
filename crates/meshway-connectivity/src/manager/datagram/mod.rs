//! One receive/demultiplex owner and one formal Session sender per actual UDP
//! endpoint. Peer associations never own or close the shared descriptor.
use super::*;
use crate::{
    DatagramSocket,
    adjacency::{Initiation, Kind, Transcript},
};
use meshway_transport::{ReceiveError, Receiver, SendContext, SendOutcome, raw::Datagram};
use std::{
    collections::{BTreeSet, VecDeque},
    time::Duration,
};
use tokio::{sync::OwnedSemaphorePermit, time::Instant};
mod health;
mod mtu;
mod receive;
mod roam;
mod session;
mod stun;
mod traversal;

const RETRY: Duration = Duration::from_millis(250);
const MAX_SENDS: u8 = 8;
const PROBE_INTERVAL: Duration = Duration::from_secs(2);
const HEALTH_WINDOW: Duration = Duration::from_secs(6);
const OUTPUT_WAIT: Duration = Duration::from_millis(100);
const MAX_REPLAY: usize = 1024;
/// After a path change every channel sends this many health Pings,
/// [`RETRY`] apart, before returning to [`PROBE_INTERVAL`].
const ROAM_PROBES: u8 = 4;
/// How long a roaming peer's new source has to answer its challenge.
const ROAM_CHALLENGE: Duration = Duration::from_millis(1500);
/// How long a channel this Node can acquire again has, after a path change
/// (or a new mapping of its socket), to answer the roaming probes: the
/// peer's challenge of the new source fits in it. Unanswered, the peer could
/// not follow (its NAT filters the new source): the channel ends at once and
/// is acquired again, instead of waiting out [`HEALTH_WINDOW`].
const ROAM_ANSWER: Duration = Duration::from_millis(1500);
/// New sources challenged at once: per channel, and per socket.
const MAX_ROAM_SOURCES: usize = 2;
const MAX_ROAM_CHALLENGES: usize = 8;

/// Bounded retransmission of one short control exchange: at most `max`
/// sends, `interval` apart. Each protocol keeps its own deadline, signature
/// and budget; only this counting/pacing rule is shared.
#[derive(Clone, Copy)]
pub(super) struct Retry {
    sends: u8,
    next: Instant,
}
impl Retry {
    /// Nothing sent yet; the first send is due now.
    pub(super) fn due() -> Self {
        Self {
            sends: 0,
            next: Instant::now(),
        }
    }
    /// One send already made now (e.g. an immediate response).
    pub(super) fn sent(interval: Duration) -> Self {
        Self {
            sends: 1,
            next: Instant::now() + interval,
        }
    }
    /// Take one send slot if it is due and under budget.
    pub(super) fn take(&mut self, interval: Duration, max: u8) -> bool {
        let now = Instant::now();
        if self.sends >= max || now < self.next {
            return false;
        }
        self.sends += 1;
        self.next = now + interval;
        true
    }
}

struct ServerAttempt {
    peer: NodeId,
    carrier: Carrier,
    transcript: Transcript,
    intro: Vec<u8>,
    intro_kind: Kind,
    challenge: Vec<u8>,
    scope: Scope,
    /// The first challenge is sent on INIT; repeats answer repeated INITs.
    responses: Retry,
    capacity: OwnedSemaphorePermit,
    supersedes: Replacement,
    rebuild: bool,
}

struct Recovery {
    transcript: Transcript,
    scope: Scope,
    output: Vec<u8>,
    retry: Retry,
    capacity: OwnedSemaphorePermit,
}

/// Initiator side of one four-message handshake: each phase carries exactly
/// the material its retransmission needs, with one retry ledger per phase.
enum ClientPhase {
    /// INIT sent; waiting for the listener's signed challenge.
    Init { output: Vec<u8> },
    /// Challenge verified, ACK signed; waiting for READY.
    Acked {
        transcript: Transcript,
        output: Vec<u8>,
    },
}

struct ClientAttempt {
    initiation: Initiation,
    phase: ClientPhase,
    retry: Retry,
}
impl ClientAttempt {
    fn output(&self) -> &[u8] {
        match &self.phase {
            ClientPhase::Init { output } | ClientPhase::Acked { output, .. } => output,
        }
    }
}

struct Outbound {
    peer: NodeId,
    carrier: Carrier,
    acquisition: LinkAcquisition,
    attempts: BTreeMap<SocketAddr, ClientAttempt>,
    scope: Scope,
    capacity: Option<OwnedSemaphorePermit>,
    established: bool,
    /// The selected channel this acquisition replaces, if any: its peer's
    /// listener moved away from that channel's remote.
    supersedes: Option<[u8; 32]>,
}

struct Channel {
    fact: LinkFact,
    transcript: Transcript,
    scope: Scope,
    health: Arc<Mutex<Instant>>,
    probe: Option<[u8; 32]>,
    next_probe: Instant,
    last_response: Instant,
    /// Size probes are answered on their own budget, apart from health Pings.
    last_probe_response: Instant,
    confirmed: bool,
    ready_until: Instant,
    budget: ControlBudget,
    _capacity: OwnedSemaphorePermit,
    recovery: Option<Recovery>,
    mtu: mtu::Search,
    /// Health Pings still due at the roaming pace after a path change.
    roam: u8,
    /// Until when the roaming probes may stay unanswered (see [`ROAM_ANSWER`]).
    roam_until: Option<Instant>,
    /// Since the last Pong to this Node's own nonce: health Pings sent, the
    /// peer's authenticated Pings received, and when it was. Why a health
    /// window ended: the path died both ways, or only one direction did.
    unanswered: u16,
    peer_pings: u16,
    answered_at: Instant,
    /// The send grant of the source this channel followed last
    /// ([`Owner::roam_answer`]); revoked by the next move.
    source_grant: Option<CancellationToken>,
}

/// A return-routability challenge of one roaming peer's new source: a
/// fresh signed Ping of the existing channel, sent only there. Its Pong,
/// signed by the peer and received from that source, moves the channel.
struct RoamChallenge {
    /// The channel's current remote (its key in `channels`).
    channel: SocketAddr,
    nonce: [u8; 32],
    output: Vec<u8>,
    /// Ends with the channel, at the deadline or once answered.
    scope: Scope,
    retry: Retry,
}

struct Owner {
    shared: Arc<Shared>,
    id: [u8; 32],
    local: SocketAddr,
    /// The same endpoint the Session owns, read only for its route MTU.
    path: Arc<dyn Datagram>,
    lifetime: Scope,
    sender: Arc<AsyncMutex<Sender>>,
    receiver: Option<Receiver>,
    listener: Option<ListenerFact>,
    outbound: Option<Outbound>,
    pending: BTreeMap<SocketAddr, ServerAttempt>,
    channels: BTreeMap<SocketAddr, Channel>,
    replay: BTreeSet<(NodeId, [u8; 32])>,
    // Independent diagnostic deduplication; never an authentication/replay gate.
    diagnosed: BTreeSet<(NodeId, [u8; 32], &'static str)>,
    unverified_init_rejections: u64,
    discovery: BTreeMap<[u8; 12], stun::Pending>,
    traversal: BTreeMap<meshway_model::UdpTraversalId, traversal::Attempt>,
    commands: Option<mpsc::Receiver<super::traversal::Command>>,
    authorization: Option<Arc<dyn crate::DatagramAuthorization>>,
    roams: tokio::sync::watch::Receiver<u64>,
    /// Challenges of new sources, keyed by the source.
    roaming: BTreeMap<SocketAddr, RoamChallenge>,
}

impl Channel {
    /// Why this channel's health window ended, from its own counters: the
    /// peer's Pings still arriving while none of this Node's was answered is
    /// one direction lost, none arriving the whole path.
    fn health_ended(&self) {
        let expired = self
            .health
            .lock()
            .is_ok_and(|deadline| Instant::now() >= *deadline);
        if expired && diagnostic_event() {
            tracing::debug!(operation = "health", peer = %self.fact.peer, channel = ?self.fact.id,
                remote = %self.fact.remote, unanswered = self.unanswered, peer_pings = self.peer_pings,
                answered_ms = self.answered_at.elapsed().as_millis() as u64,
                cause = if self.peer_pings == 0 { "no_peer_traffic" } else { "own_probes_unanswered" },
                "Connectivity channel health ended");
        }
    }
}

impl Owner {
    /// New desired content. In-flight acquisition was authenticated or
    /// admitted on the earlier content: drop it. Manager's apply has already
    /// kept or retired every selected channel; ones it retired leave here.
    fn reconfigured(&mut self) -> Result<(), Error> {
        for attempt in self.pending.values() {
            attempt.scope.cancellation.cancel();
        }
        self.pending.clear();
        for attempt in self.traversal.values() {
            attempt.cancel();
        }
        self.traversal.clear();
        self.discovery.clear();
        if let Some(outbound) = &mut self.outbound
            && !outbound.established
        {
            outbound.scope.cancellation.cancel();
            outbound.attempts.clear();
            return Err(Error::Unavailable);
        }
        let remotes: Vec<_> = self.channels.keys().copied().collect();
        for remote in remotes {
            let channel = self.channels.get_mut(&remote).ok_or(Error::Worker)?;
            if let Some(recovery) = channel.recovery.take() {
                recovery.scope.cancellation.cancel();
            }
            // Health is proven again under the new content at once; READY
            // answers belong to the acquisition that was dropped above.
            channel.probe = None;
            channel.next_probe = Instant::now();
            channel.ready_until = Instant::now();
            if !self.selected(&self.channels[&remote]) {
                let channel = self.channels.remove(&remote).ok_or(Error::Worker)?;
                channel.scope.cancellation.cancel();
                retire(&self.shared, &channel.fact)?;
            }
        }
        self.changed();
        Ok(())
    }

    fn new(shared: Arc<Shared>, endpoint: Box<dyn Datagram>, id: [u8; 32]) -> Result<Self, Error> {
        let local = endpoint.local_addr()?;
        let path: Arc<dyn Datagram> = Arc::from(endpoint);
        let lifetime = shared.scope(None, false);
        let limits = shared.limits.session;
        if limits.max_frame_bytes() < adjacency::MAX_MESSAGE {
            return Err(Error::Limits);
        }
        let frames = limits.max_frames().min(64);
        let limits = meshway_transport::Limits::new(
            limits.max_frame_bytes().min(65_507),
            frames,
            limits.max_bytes().min(1024 * 1024),
            limits.max_operations().min(frames),
            limits
                .max_batch_frames()
                .min(frames)
                .min(shared.limits.events),
        )
        .map_err(|_| Error::Limits)?;
        let (mut sender, receiver) = Session::datagram(
            scope::Datagram {
                inner: Box::new(path.clone()),
                scope: shared.physical()?,
            },
            limits,
        )
        .into_split();
        sender
            .reserve_control(meshway_transport::BudgetAvailable {
                frames: 2,
                bytes: 2 * adjacency::MAX_MESSAGE,
                operations: 2,
            })
            .map_err(|_| Error::Limits)?;
        let mut roams = shared.roams.subscribe();
        roams.borrow_and_update();
        Ok(Self {
            roams,
            roaming: BTreeMap::new(),
            shared,
            id,
            local,
            path,
            lifetime,
            sender: Arc::new(AsyncMutex::new(sender)),
            receiver: Some(receiver),
            listener: None,
            outbound: None,
            pending: BTreeMap::new(),
            channels: BTreeMap::new(),
            replay: BTreeSet::new(),
            diagnosed: BTreeSet::new(),
            unverified_init_rejections: 0,
            discovery: BTreeMap::new(),
            traversal: BTreeMap::new(),
            commands: None,
            authorization: None,
        })
    }

    fn changed(&self) {
        let _ = self.shared.control_events.try_send(Event::Changed);
    }

    /// Read each channel's route MTU and drop entries of channels this owner
    /// no longer has. A system call per channel, no packet.
    /// A finished search that confirmed nothing (the path shrank below the
    /// floor) withdraws the channel's value; one still searching keeps it.
    fn measure_mtu(&self) {
        let measured: Vec<_> = self
            .channels
            .iter()
            .filter_map(|(remote, channel)| {
                Some((
                    channel.fact.id,
                    channel
                        .mtu
                        .measured(probe_route(self.path.as_ref(), *remote))?,
                ))
            })
            .collect();
        if let Ok(mut known) = self.shared.path_mtu.lock() {
            known.retain(|id, (socket, _)| {
                *socket != self.id || self.channels.values().any(|channel| channel.fact.id == *id)
            });
            for (id, mtu) in measured {
                match mtu {
                    Some(mtu) => known.insert(id, (self.id, mtu)),
                    None => known.remove(&id),
                };
            }
        }
    }

    fn selected(&self, channel: &Channel) -> bool {
        channel.scope.check().is_ok()
            && self.shared.links.lock().is_ok_and(|links| {
                links
                    .get(&channel.fact.peer)
                    .is_some_and(|link| link.fact.id == channel.fact.id)
            })
    }

    async fn run(&mut self) -> Result<(), Error> {
        let mut plans = self.shared.plans.subscribe();
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let lifetime = self.lifetime.clone();
            enum Work {
                Reconfigured,
                Roamed,
                Tick,
                Frame(Result<Vec<Frame>, ReceiveError>, std::time::Instant),
                Command(Option<super::traversal::Command>),
            }
            let work = lifetime.run(async {
                tokio::select! { biased;
                    changed = plans.changed() => changed.map(|_| Work::Reconfigured).map_err(|_| Error::Cancelled),
                    changed = self.roams.changed() => changed.map(|_| Work::Roamed).map_err(|_| Error::Cancelled),
                    _ = tick.tick() => Ok(Work::Tick),
                    command = async { match &mut self.commands { Some(commands) => commands.recv().await, None => std::future::pending().await } } => Ok(Work::Command(command)),
                    frame = self.receiver.as_mut().ok_or(Error::Unavailable)?.recv_batch() => Ok(Work::Frame(frame, std::time::Instant::now())),
                }
            }).await;
            let operation = async {
                match work? {
                    Work::Reconfigured => self.reconfigured()?,
                    Work::Roamed => {
                        self.roams.borrow_and_update();
                        self.roamed();
                        self.tick().await?;
                    }
                    Work::Command(Some(command)) => self.traversal_command(command)?,
                    Work::Command(None) => self.commands = None,
                    Work::Tick => self.tick().await?,
                    Work::Frame(Ok(frames), received_at) => {
                        self.receive_batch(frames, received_at).await?;
                    }
                    Work::Frame(
                        Err(
                            ReceiveError::Empty
                            | ReceiveError::Truncated(_)
                            | ReceiveError::Oversized
                            | ReceiveError::InvalidMetadata,
                        ),
                        _,
                    ) => {}
                    Work::Frame(Err(ReceiveError::Io(error)), _)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::ConnectionRefused
                                | std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::HostUnreachable
                                | std::io::ErrorKind::NetworkUnreachable
                                | std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::Interrupted
                        ) =>
                    {
                        // Unconnected UDP may surface an asynchronous error from a
                        // single destination. Its health scope expires independently.
                        tokio::task::yield_now().await;
                    }
                    Work::Frame(Err(_), _) => return Err(Error::Unavailable),
                }
                Ok(())
            };
            lifetime.run(operation).await?;
        }
    }

    async fn shutdown(&mut self) -> Result<(), Error> {
        self.lifetime.cancellation.cancel();
        for channel in self.channels.values() {
            channel.scope.cancellation.cancel();
            retire(&self.shared, &channel.fact)?;
        }
        for attempt in self.pending.values() {
            attempt.scope.cancellation.cancel();
        }
        if let Some(outbound) = &self.outbound {
            outbound.scope.cancellation.cancel();
        }
        self.receiver.take(); // synchronously closes the formal session's raw I/O
        self.sender
            .lock()
            .await
            .shutdown()
            .await
            .map_err(|_| Error::Worker)?;
        self.channels.clear();
        self.pending.clear();
        self.changed();
        Ok(())
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.lifetime.cancellation.cancel();
        if let Ok(mut known) = self.shared.path_mtu.lock() {
            known.retain(|_, (socket, _)| *socket != self.id);
        }
        for attempt in self.traversal.values() {
            attempt.cancel();
        }
        if let Ok(mut sockets) = self.shared.sockets.lock() {
            sockets.remove(&self.id);
        }
        for channel in self.channels.values() {
            channel.scope.cancellation.cancel();
            let _ = retire(&self.shared, &channel.fact);
        }
        for attempt in self.pending.values() {
            attempt.scope.cancellation.cancel();
        }
        if let Some(outbound) = &self.outbound {
            outbound.scope.cancellation.cancel();
        }
        self.receiver.take();
    }
}

/// IP and UDP header bytes of a datagram toward `remote`.
fn header(remote: SocketAddr) -> u16 {
    if remote.is_ipv4() { 28 } else { 48 }
}

/// The route MTU toward `remote` as far as a size probe can show it: never
/// above the largest probe the codec encodes. A larger route (jumbo frames)
/// is reported at that bound, which every inner MTU fits.
fn probe_route(path: &dyn Datagram, remote: SocketAddr) -> Option<u16> {
    let ceiling = u16::try_from(adjacency::MAX_PROBE)
        .unwrap_or(u16::MAX)
        .saturating_add(header(remote));
    path.path_mtu(remote).map(|route| route.min(ceiling))
}

async fn send_control(
    sender: &AsyncMutex<Sender>,
    scope: &Scope,
    remote: SocketAddr,
    bytes: Vec<u8>,
) -> Result<(), Error> {
    scope.check()?;
    let cancellation = scope.cancellation.child_token();
    let _cancel = cancellation.clone().drop_guard();
    let deadline = scope
        .deadline_or(OUTPUT_WAIT)
        .min(Instant::now() + OUTPUT_WAIT);
    let context = SendContext {
        cancellation,
        deadline: Some(deadline),
        lifetime: Some(Arc::new(scope.clone())),
    };
    let mut frames = VecDeque::from([Frame::new(bytes, Some(remote))]);
    let operation = async {
        let admission = sender.lock().await.try_send(&mut frames, context);
        if admission.accepted != 1 {
            return Err(Error::Capacity);
        }
        let completion = admission
            .completion
            .ok_or(Error::Worker)?
            .wait()
            .await
            .map_err(|_| Error::Worker)?;
        if completion.frames.len() != 1
            || !matches!(completion.frames[0].outcome, SendOutcome::LocalComplete)
        {
            return Err(Error::Unavailable);
        }
        Ok(())
    };
    scope
        .run(async {
            tokio::time::timeout_at(deadline, operation)
                .await
                .map_err(|_| Error::Cancelled)?
        })
        .await
}

pub(super) async fn listen(shared: Arc<Shared>, fact: ListenerFact, endpoint: Box<dyn Datagram>) {
    let result = async {
        let mut owner = Owner::new(shared.clone(), endpoint, fact.socket_id)?;
        owner.listener = Some(fact);
        let result = owner.run().await;
        owner.shutdown().await?;
        result
    }
    .await;
    if let Err(error) = &result {
        tracing::debug!(%error, "Datagram socket owner ended");
        shared.stop.cancel();
    }
}

pub(super) async fn listen_traversal(
    shared: Arc<Shared>,
    id: [u8; 32],
    listener: Option<ListenerFact>,
    socket: crate::TraversalSocket,
    commands: mpsc::Receiver<super::traversal::Command>,
) {
    let result = async {
        let mut owner = Owner::new(shared.clone(), socket.endpoint, id)?;
        owner.listener = listener;
        owner.commands = Some(commands);
        owner.authorization = Some(socket.authorization);
        let result = owner.run().await;
        owner.shutdown().await?;
        result
    }
    .await;
    if let Err(error) = &result {
        tracing::debug!(%error, "Datagram socket owner ended");
        shared.stop.cancel();
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn dial(
    shared: Arc<Shared>,
    factory: Arc<dyn SocketFactory>,
    plan: Arc<Plan>,
    peer: NodeId,
    key: &str,
    carrier: &Carrier,
    target: &workers::Target,
    timeout: Duration,
    supersedes: Option<[u8; 32]>,
) -> Result<(), Error> {
    let mut scope = shared.scope(Some((peer, carrier.clone())), true);
    scope.deadline = Some(scope.deadline_or(timeout).min(Instant::now() + timeout));
    let endpoints = scope
        .run(factory.datagram_endpoint(&plan.desired, key, &target.endpoint))
        .await?;
    if endpoints.len() > 2 {
        return Err(Error::Capacity);
    }
    // Both address families use the SAME original resolution/handshake budget.
    // A blackholed v4 address must not prevent trying a configured v6 address.
    let mut attempts = JoinSet::new();
    let acquisition = target.acquisition.clone();
    for socket in endpoints {
        let scope = Scope {
            cancellation: scope.cancellation.child_token(),
            ..scope.clone()
        };
        attempts.spawn(dial_endpoint(
            shared.clone(),
            plan.clone(),
            socket,
            peer,
            carrier.clone(),
            scope,
            acquisition.clone(),
            supersedes,
        ));
    }
    let mut failed = false;
    while let Some(result) = attempts.join_next().await {
        if result.is_err() {
            failed = true;
            shared.stop.cancel();
        }
    }
    if failed {
        return Err(Error::Worker);
    }
    Err(Error::Unavailable)
}

#[allow(clippy::too_many_arguments)]
async fn dial_endpoint(
    shared: Arc<Shared>,
    plan: Arc<Plan>,
    socket: DatagramSocket,
    peer: NodeId,
    carrier: Carrier,
    scope: Scope,
    acquisition: LinkAcquisition,
    supersedes: Option<[u8; 32]>,
) -> Result<(), Error> {
    let DatagramSocket { endpoint, targets } = socket;
    if targets.is_empty() || targets.len() > 16 {
        return Err(Error::Capacity);
    }
    let local = endpoint.local_addr()?;
    let capacity = shared
        .capacity
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::Capacity)?;
    let mut attempts = BTreeMap::new();
    for remote in targets {
        if remote.port() == 0
            || remote.ip().is_unspecified()
            || remote.ip().is_multicast()
            || remote.is_ipv4() != local.is_ipv4()
        {
            return Err(Error::Configuration);
        }
        let (initiation, intro) = Initiation::new(&shared.keys, &plan, &carrier, &shared.boot)?;
        attempts.insert(
            remote,
            ClientAttempt {
                initiation,
                phase: ClientPhase::Init {
                    output: adjacency::encode(Kind::Init, &intro)?,
                },
                retry: Retry::due(),
            },
        );
    }
    scope.check()?;
    let mut owner = Owner::new(shared, endpoint, adjacency::nonce()?)?;
    owner.shared.register_socket(
        owner.id,
        meshway_model::state::ResourceRef::Dial {
            key: format!("{}/{}", carrier.listener, carrier.listener_key),
        },
        TransportProtocol::NativeUdp,
        local,
    )?;
    owner.outbound = Some(Outbound {
        peer,
        carrier,
        acquisition,
        attempts,
        scope,
        capacity: Some(capacity),
        established: false,
        supersedes,
    });
    let result = owner.run().await;
    owner.shutdown().await?;
    result
}

pub(super) struct ControlBudget {
    window: Instant,
    remaining: u16,
    limit: u16,
}
impl ControlBudget {
    pub fn new() -> Self {
        Self::with_limit(256)
    }
    fn with_limit(limit: u16) -> Self {
        Self {
            window: Instant::now(),
            remaining: limit,
            limit,
        }
    }
    fn take(&mut self) -> bool {
        if Instant::now() >= self.window + Duration::from_secs(1) {
            self.window = Instant::now();
            self.remaining = self.limit;
        }
        if self.remaining == 0 {
            false
        } else {
            self.remaining -= 1;
            true
        }
    }
}

#[cfg(test)]
pub(in crate::manager) mod tests;
