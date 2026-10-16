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
mod mtu;
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

    #[allow(clippy::too_many_arguments)]
    fn promote(
        &mut self,
        plan: &Arc<Plan>,
        peer: NodeId,
        carrier: Carrier,
        remote: SocketAddr,
        transcript: Transcript,
        capacity: OwnedSemaphorePermit,
        ready_until: Instant,
        supersedes: Replacement,
        acquisition: LinkAcquisition,
    ) -> Result<bool, Error> {
        self.promote_scoped(
            plan,
            peer,
            carrier,
            remote,
            transcript,
            capacity,
            ready_until,
            supersedes,
            acquisition,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn promote_scoped(
        &mut self,
        plan: &Arc<Plan>,
        peer: NodeId,
        carrier: Carrier,
        remote: SocketAddr,
        transcript: Transcript,
        capacity: OwnedSemaphorePermit,
        ready_until: Instant,
        supersedes: Replacement,
        acquisition: LinkAcquisition,
        attempt: Option<meshway_control::connectivity::AttemptScope>,
    ) -> Result<bool, Error> {
        if self
            .channels
            .get(&remote)
            .is_some_and(|channel| channel.fact.peer != peer)
        {
            return Ok(false);
        }
        let health = Arc::new(Mutex::new(Instant::now() + HEALTH_WINDOW));
        let mut scope = self.shared.scope(Some((peer, carrier.clone())), false);
        scope.health = Some(health.clone());
        if scope.check().is_err() {
            return Ok(false);
        }
        if let Some(attempt) = &attempt {
            attempt.ensure_current().map_err(|_| Error::Authority)?;
            self.authorization
                .as_ref()
                .ok_or(Error::Underlay)?
                .authorize_channel(attempt, Arc::new(scope.clone()), remote)?;
        }
        let fact = LinkFact {
            id: transcript.id(),
            socket_id: self.id,
            peer,
            peer_boot: transcript.peer_boot(carrier.dialer == plan.node),
            carrier,
            local: self.local,
            remote,
            acquisition,
        };
        let selected = install_ready(
            &self.shared,
            ReadyPeer {
                fact: fact.clone(),
                sender: self.sender.clone(),
                scope: scope.clone(),
                plan: plan.clone(),
                supersedes,
                attempt,
            },
        );
        match selected {
            Ok(true) => {}
            Ok(false) => {
                scope.cancellation.cancel();
                return Ok(false);
            }
            Err(error) => {
                scope.cancellation.cancel();
                return Err(error);
            }
        }
        if let Some(old) = self.channels.remove(&remote) {
            old.scope.cancellation.cancel();
        }
        self.channels.insert(
            remote,
            Channel {
                fact,
                transcript,
                scope,
                health,
                probe: None,
                next_probe: Instant::now() + PROBE_INTERVAL,
                last_response: Instant::now() - RETRY,
                last_probe_response: Instant::now() - RETRY,
                confirmed: false,
                ready_until,
                budget: ControlBudget::with_limit(16),
                _capacity: capacity,
                recovery: None,
                mtu: mtu::Search::new(),
                roam: 0,
                roam_until: None,
                unanswered: 0,
                peer_pings: 0,
                answered_at: Instant::now(),
                source_grant: None,
            },
        );
        self.measure_mtu();
        self.changed();
        Ok(true)
    }

    /// The underlay path changed: every channel proves itself at once, so a
    /// peer sees this Node's new source within a round trip. The path is a
    /// new one: its size is searched again and the old measurement withdrawn
    /// meanwhile, as when a peer's channel follows it ([`Self::roam_answer`]).
    fn roamed(&mut self) {
        for channel in self.channels.values_mut() {
            channel.roam = ROAM_PROBES;
            channel.next_probe = Instant::now();
            // An inbound channel is the peer's to dial again; ending it here
            // would only stop the Pings that let the peer follow.
            if channel.fact.acquisition != LinkAcquisition::AcceptedInbound {
                channel.roam_until = Some(Instant::now() + ROAM_ANSWER);
            }
            channel.mtu = mtu::Search::new();
            if let Ok(mut known) = self.shared.path_mtu.lock() {
                known.remove(&channel.fact.id);
            }
        }
        self.measure_mtu();
    }

    /// A channel whose roaming probes went unanswered past [`ROAM_ANSWER`]
    /// ends its health window now: the peer could not follow this Node's new
    /// source, and the channel is acquired again (for a peer behind NAT, by
    /// a new coordinated traversal this Node starts).
    fn roam_unanswered(&mut self) -> Result<(), Error> {
        let now = Instant::now();
        for channel in self.channels.values_mut() {
            if channel.roam_until.is_none_or(|until| now < until) {
                continue;
            }
            channel.roam_until = None;
            diagnostic_link(
                &self.shared,
                &channel.fact,
                &channel.scope,
                "roam",
                "unanswered",
                None,
            );
            *channel.health.lock().map_err(|_| Error::Worker)? = now;
        }
        Ok(())
    }

    /// A Ping from an unknown source that one of this socket's selected
    /// channels signed: its peer roamed. Challenge the new source with a
    /// fresh signed Ping of that channel; nothing changes until its Pong.
    /// Returns whether a channel claimed the message.
    async fn roam_challenge(&mut self, remote: SocketAddr, body: &[u8]) -> Result<bool, Error> {
        if self.roaming.contains_key(&remote) {
            return Ok(true);
        }
        let plan = self.shared.plan();
        let Some(current) = self
            .channels
            .iter()
            .find(|(_, channel)| {
                self.selected(channel)
                    && channel
                        .transcript
                        .verify_health(&plan, channel.fact.peer, Kind::Ping, body)
                        .is_ok()
            })
            .map(|(current, _)| *current)
        else {
            return Ok(false);
        };
        if self.roaming.len() >= MAX_ROAM_CHALLENGES
            || self
                .roaming
                .values()
                .filter(|challenge| challenge.channel == current)
                .count()
                >= MAX_ROAM_SOURCES
            || remote.is_ipv4() != self.local.is_ipv4()
        {
            return Ok(true);
        }
        let channel = &self.channels[&current];
        let scope = Scope {
            cancellation: channel.scope.cancellation.child_token(),
            deadline: Some(Instant::now() + ROAM_CHALLENGE),
            edge: channel.scope.edge.clone(),
            health: None,
        };
        // A dial or traversal socket sends only to its own recipients: the
        // challenge alone may reach this source, for its own lifetime.
        if self
            .path
            .authorize_challenge(remote, Arc::new(scope.clone()))
            .is_err()
        {
            scope.cancellation.cancel();
            return Ok(true);
        }
        let nonce = adjacency::boot_nonce(&self.shared.boot)?;
        let output = adjacency::encode(
            Kind::Ping,
            &channel
                .transcript
                .health(&self.shared.keys, &plan, Kind::Ping, &nonce)?,
        )?;
        diagnostic_link(
            &self.shared,
            &channel.fact,
            &channel.scope,
            "roam",
            "source_challenged",
            Some((remote, nonce)),
        );
        self.roaming.insert(
            remote,
            RoamChallenge {
                channel: current,
                nonce,
                output,
                scope,
                retry: Retry::due(),
            },
        );
        self.roam_tick().await;
        Ok(true)
    }

    /// Send due challenges; drop ended ones.
    async fn roam_tick(&mut self) {
        self.roaming.retain(|_, challenge| {
            let live = challenge.scope.check().is_ok();
            if !live {
                challenge.scope.cancellation.cancel();
            }
            live
        });
        for (remote, challenge) in &mut self.roaming {
            if challenge.retry.take(RETRY, ROAM_PROBES) {
                let _ = send_control(
                    &self.sender,
                    &challenge.scope,
                    *remote,
                    challenge.output.clone(),
                )
                .await;
            }
        }
    }

    /// A Pong from an unknown source: if it answers that source's challenge,
    /// the channel moves there. Its identity, transcript and boot stay; only
    /// the recipient changes, and the path size is searched again.
    fn roam_answer(&mut self, remote: SocketAddr, body: &[u8]) -> Result<(), Error> {
        let Some(challenge) = self.roaming.get(&remote) else {
            return Ok(());
        };
        let Some(channel) = self.channels.get(&challenge.channel) else {
            return Ok(());
        };
        let plan = self.shared.plan();
        if challenge.scope.check().is_err()
            || channel
                .transcript
                .verify_health(&plan, channel.fact.peer, Kind::Pong, body)
                .ok()
                != Some(challenge.nonce)
        {
            return Ok(());
        }
        let challenge = self.roaming.remove(&remote).ok_or(Error::Worker)?;
        challenge.scope.cancellation.cancel();
        let old = challenge.channel;
        self.roaming.retain(|_, other| {
            let keep = other.channel != old;
            if !keep {
                other.scope.cancellation.cancel();
            }
            keep
        });
        if self.channels.contains_key(&remote) {
            return Ok(());
        }
        let mut channel = self.channels.remove(&old).ok_or(Error::Worker)?;
        // One grant per channel, for its current remote: the next move
        // revokes it, so a channel follows any number of moves.
        let grant = Scope {
            cancellation: channel.scope.cancellation.child_token(),
            ..channel.scope.clone()
        };
        let moved = self
            .path
            .authorize_source(remote, Arc::new(grant.clone()))
            .is_ok()
            && channel.scope.check().is_ok()
            && {
                let mut links = self.shared.links.lock().map_err(|_| Error::Worker)?;
                match links.get_mut(&channel.fact.peer) {
                    Some(link) if link.fact.id == channel.fact.id => {
                        link.fact.remote = remote;
                        true
                    }
                    _ => false,
                }
            };
        if !moved {
            grant.cancellation.cancel();
            self.channels.insert(old, channel);
            return Ok(());
        }
        if let Some(previous) = channel.source_grant.replace(grant.cancellation) {
            previous.cancel();
        }
        channel.fact.remote = remote;
        channel.mtu = mtu::Search::new();
        channel.probe = None;
        channel.roam = 0;
        channel.roam_until = None;
        channel.confirmed = true;
        channel.next_probe = Instant::now() + PROBE_INTERVAL;
        *channel.health.lock().map_err(|_| Error::Worker)? = Instant::now() + HEALTH_WINDOW;
        if let Ok(mut known) = self.shared.path_mtu.lock() {
            known.remove(&channel.fact.id);
        }
        diagnostic_link(
            &self.shared,
            &channel.fact,
            &channel.scope,
            "roam",
            "source_moved",
            Some((old, channel.fact.id)),
        );
        self.channels.insert(remote, channel);
        self.measure_mtu();
        self.changed();
        Ok(())
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

    async fn tick(&mut self) -> Result<(), Error> {
        if self
            .channels
            .values()
            .any(|channel| Instant::now() >= channel.next_probe)
        {
            self.measure_mtu();
        }
        self.stun_tick().await?;
        self.traversal_tick().await?;
        self.roam_tick().await;
        self.lifetime.check()?;
        self.pending
            .retain(|_, attempt| attempt.scope.check().is_ok());
        self.roam_unanswered()?;
        let stale: Vec<_> = self
            .channels
            .iter()
            .filter(|(_, channel)| !self.selected(channel))
            .map(|(remote, _)| *remote)
            .collect();
        for remote in stale {
            self.lifetime.check()?;
            if let Some(channel) = self.channels.remove(&remote) {
                diagnostic_link(
                    &self.shared,
                    &channel.fact,
                    &channel.scope,
                    "channel_cleanup",
                    "scope_invalid_or_deselected",
                    None,
                );
                channel.health_ended();
                channel.scope.cancellation.cancel();
                retire(&self.shared, &channel.fact)?;
                self.changed();
            }
        }
        if let Some(outbound) = &mut self.outbound {
            if outbound.established {
                if self.channels.is_empty() {
                    return Err(Error::Unavailable);
                }
            } else {
                outbound.scope.check()?;
                if self
                    .shared
                    .links
                    .lock()
                    .map_err(|_| Error::Worker)?
                    .get(&outbound.peer)
                    .is_some_and(|link| {
                        link.scope.check().is_ok()
                            && Some(link.fact.id) != outbound.supersedes
                            && (link.fact.carrier == outbound.carrier
                                || link.fact.carrier.protocol != outbound.carrier.protocol)
                    })
                {
                    return Err(Error::Unavailable);
                }
                for (remote, attempt) in &mut outbound.attempts {
                    if !attempt.retry.take(RETRY, MAX_SENDS) {
                        continue;
                    }
                    let _ = send_control(
                        &self.sender,
                        &outbound.scope,
                        *remote,
                        attempt.output().to_vec(),
                    )
                    .await;
                }
            }
        }
        for (remote, channel) in &mut self.channels {
            if channel
                .recovery
                .as_ref()
                .is_some_and(|recovery| recovery.scope.check().is_err())
                && let Some(recovery) = channel.recovery.take()
            {
                recovery.scope.cancellation.cancel();
            }
            if let Some(recovery) = &mut channel.recovery
                && recovery.retry.take(RETRY, MAX_SENDS)
            {
                let _ = send_control(
                    &self.sender,
                    &recovery.scope,
                    *remote,
                    recovery.output.clone(),
                )
                .await;
            }
            if channel.scope.check().is_ok()
                && let Some(size) = channel
                    .mtu
                    .due(probe_route(self.path.as_ref(), *remote), Instant::now())
            {
                // A DF size probe signed for its exact length. One that cannot
                // be built ends this measurement only, never the owner.
                let nonce = adjacency::boot_nonce(&self.shared.boot)?;
                match channel.transcript.probe(
                    &self.shared.keys,
                    &self.shared.plan(),
                    &nonce,
                    usize::from(size.saturating_sub(header(*remote))),
                ) {
                    Ok(bytes) => {
                        channel.mtu.sent(nonce);
                        let _ = send_control(&self.sender, &channel.scope, *remote, bytes).await;
                    }
                    Err(error) => {
                        tracing::debug!(?error, peer = %channel.fact.peer, channel = ?channel.fact.id,
                            size, "UDP size probe not built; measurement ended");
                        channel.mtu.abandon(Instant::now());
                    }
                }
            }
            if channel.scope.check().is_err() || Instant::now() < channel.next_probe {
                continue;
            }
            // Only a response to this node's fresh challenge renews health.
            // Received business packets and remote Ping messages do not.
            let nonce = match channel.probe {
                Some(nonce) => nonce,
                None => adjacency::boot_nonce(&self.shared.boot)?,
            };
            channel.probe = Some(nonce);
            channel.next_probe = Instant::now()
                + if channel.roam > 0 {
                    channel.roam -= 1;
                    RETRY
                } else {
                    PROBE_INTERVAL
                };
            let bytes = adjacency::encode(
                Kind::Ping,
                &channel.transcript.health(
                    &self.shared.keys,
                    &self.shared.plan(),
                    Kind::Ping,
                    &nonce,
                )?,
            )?;
            channel.unanswered = channel.unanswered.saturating_add(1);
            if let Err(error) = send_control(&self.sender, &channel.scope, *remote, bytes).await {
                tracing::debug!(?error, peer = %channel.fact.peer, channel = ?channel.fact.id,
                    "UDP health probe send failed");
                meshway_logging::profile::event("manager.udp.health", "send_failed");
            }
        }
        Ok(())
    }

    async fn receive(
        &mut self,
        frame: Frame,
        received_at: std::time::Instant,
    ) -> Result<(), Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("manager.udp.dispatch.poll");
        PROFILE.poll(self.receive_inner(frame, received_at)).await
    }

    async fn receive_batch(
        &mut self,
        frames: Vec<Frame>,
        received_at: std::time::Instant,
    ) -> Result<(), Error> {
        // Demultiplex before the business event queue. Its saturation must not
        // delay a health/handshake/STUN response already received by this socket.
        // Signature work retains the independent per-peer/global control budget.
        let (control, data): (Vec<_>, Vec<_>) = frames.into_iter().partition(|frame| {
            frame.payload.starts_with(adjacency::MAGIC)
                || frame.payload.starts_with(traversal::MAGIC)
                || meshway_stun::classify(&frame.payload).is_some()
        });
        for frame in control {
            self.receive(frame, received_at).await?;
        }
        // Under backlog, give the notified consumer a turn before another
        // burst, rather than waiting until all 1,024 slots are occupied. Small
        // ready prefixes are published immediately; never await capacity.
        if self.shared.events.consumer_turn(data.len()) {
            meshway_logging::profile::event("manager.udp.dispatch", "consumer_turn");
            tokio::task::yield_now().await;
        }
        let mut started = std::time::Instant::now();
        let mut checked = BTreeMap::new();
        let mut events = Vec::new();
        for frame in data {
            if started.elapsed() >= Duration::from_micros(200) {
                // Descheduling is not packet congestion. Retain the bounded
                // already-read suffix across a fair turn, then recheck its
                // original channel when dispatch resumes.
                meshway_logging::profile::event("manager.udp.dispatch", "quantum_yield");
                self.dispatch_batch(std::mem::take(&mut events));
                tokio::task::yield_now().await;
                started = std::time::Instant::now();
                checked.clear();
            }
            let Some(remote) = frame.peer else {
                continue;
            };
            let Some(channel) = self.channels.get(&remote) else {
                continue;
            };
            if *checked
                .entry(remote)
                .or_insert_with(|| self.selected(channel))
            {
                let event = Event::Frame {
                    link: Box::new(channel.fact.clone()),
                    frame,
                    received_at,
                };
                if Shared::business_event(&event) {
                    meshway_logging::profile::event("manager.udp", "mesh_frame");
                    events.push(event);
                } else {
                    self.dispatch_event(event);
                }
            }
        }
        self.dispatch_batch(events);
        // One fair scheduling turn per bounded batch, rather than one per
        // rejected business packet while control waits in the socket backlog.
        tokio::task::yield_now().await;
        Ok(())
    }

    async fn receive_inner(
        &mut self,
        frame: Frame,
        received_at: std::time::Instant,
    ) -> Result<(), Error> {
        let Some(remote) = frame.peer else {
            return Ok(());
        };
        if meshway_stun::classify(&frame.payload).is_some() {
            self.stun_receive(remote, &frame.payload)?;
            return Ok(());
        }
        if frame.payload.starts_with(traversal::MAGIC) {
            self.traversal_receive(remote, &frame.payload).await?;
            return Ok(());
        }
        if frame.payload.starts_with(adjacency::MAGIC) {
            // A single bounded rate budget covers all listeners/candidates and
            // is checked before signature work or attacker-selected allocation.
            let admitted = match self.channels.get_mut(&remote) {
                Some(channel) => channel.budget.take(),
                None => self
                    .shared
                    .control_budget
                    .lock()
                    .map_err(|_| Error::Worker)?
                    .take(),
            };
            if !admitted {
                return Ok(());
            }
            let Ok((kind, body)) = adjacency::decode(&frame.payload) else {
                return Ok(());
            };
            match kind {
                Kind::Init => self.init(remote, body).await?,
                Kind::Challenge | Kind::Ready => self.client_message(remote, kind, body).await?,
                Kind::Ack => self.ack(remote, body).await?,
                Kind::Ping | Kind::Pong => self.health(remote, kind, body).await?,
                Kind::Probe => {
                    self.answer_size_probe(remote, frame.payload.len(), body)
                        .await?
                }
            }
        } else if let Some(channel) = self.channels.get(&remote)
            && self.selected(channel)
        {
            // Source is a demultiplex key only after signed challenge completion.
            // SecureNode remains the sole packet decoder and proof verifier.
            self.dispatch(channel, frame, received_at);
        }
        Ok(())
    }

    fn dispatch(&self, channel: &Channel, frame: Frame, received_at: std::time::Instant) {
        let event = Event::Frame {
            link: Box::new(channel.fact.clone()),
            frame,
            received_at,
        };
        self.dispatch_event(event);
    }

    fn dispatch_event(&self, event: Event) {
        meshway_logging::profile::event("manager.udp", "mesh_frame");
        match self.shared.event_sender(&event).try_send(event) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                meshway_logging::profile::event("manager.udp.drop", "event_full");
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                meshway_logging::profile::event("manager.udp.drop", "event_closed");
            }
        }
    }

    fn dispatch_batch(&self, mut events: Vec<Event>) {
        if events.is_empty() {
            return;
        }
        let count = events.len();
        let accepted = count.min(self.shared.events.capacity());
        events.truncate(accepted);
        let mut dropped = count - accepted;
        let mut reason = "event_full";
        if accepted != 0 {
            match self.shared.events.try_send_batch(events) {
                Ok(()) => meshway_logging::profile::event("manager.event_queue", "publish_batch"),
                Err(error) => {
                    if matches!(error, mpsc::error::TrySendError::Closed(_)) {
                        reason = "event_closed";
                    }
                    dropped += accepted;
                }
            }
        }
        for _ in 0..dropped {
            meshway_logging::profile::event("manager.udp.drop", reason);
        }
    }

    async fn repeat_intro(&mut self, remote: SocketAddr, kind: Kind, body: &[u8]) -> bool {
        if let Some(attempt) = self.pending.get_mut(&remote) {
            if attempt.scope.check().is_ok()
                && attempt.intro_kind == kind
                && attempt.intro == body
                && attempt.responses.take(RETRY, MAX_SENDS)
            {
                let _ = send_control(
                    &self.sender,
                    &attempt.scope,
                    remote,
                    attempt.challenge.clone(),
                )
                .await;
            }
            return true;
        }
        false
    }

    fn incoming_carriers(&self) -> Result<Vec<(NodeId, Carrier)>, Error> {
        let Some(listener) = &self.listener else {
            return Ok(Vec::new());
        };
        Ok(self
            .shared
            .plan()
            .incoming(&listener.key, TransportProtocol::NativeUdp))
    }

    async fn init(&mut self, remote: SocketAddr, body: &[u8]) -> Result<(), Error> {
        if self.listener.is_none() || self.repeat_intro(remote, Kind::Init, body).await {
            return Ok(());
        }
        // A fresh source challenge may coexist with the old channel even on
        // the same tuple. Nothing in the channel map changes until valid ACK.
        if self.pending.len() + self.channels.len() >= self.shared.limits.connections {
            return Ok(());
        }
        let plan = self.shared.plan();
        let cfg = &*plan;
        // Only the claimed dialer's carrier is checked; its exact context and
        // signature are still verified below.
        let listener = self.listener.as_ref().ok_or(Error::Worker)?;
        let carriers: Vec<_> = adjacency::claimed_dialer(body)
            .and_then(|peer| {
                cfg.accepting(peer, &listener.key, TransportProtocol::NativeUdp)
                    .map(|carrier| (peer, carrier))
            })
            .into_iter()
            .collect();
        let Ok((peer, carrier, initiation)) = adjacency::incoming(cfg, &carriers, body) else {
            self.unverified_init_rejections = self.unverified_init_rejections.saturating_add(1);
            if self.unverified_init_rejections.is_power_of_two() && diagnostic_event() {
                tracing::debug!(operation = "init_rejected", cause = "signature_or_context",
                    remote = %remote, count = self.unverified_init_rejections,
                    "Connectivity unauthenticated initiation rejected");
            }
            return Ok(());
        };
        if self
            .channels
            .get(&remote)
            .is_some_and(|channel| channel.fact.peer != peer)
        {
            return Ok(());
        }
        if self.replay.contains(&(peer, initiation.client))
            || self.pending.values().any(|attempt| attempt.peer == peer)
        {
            return Ok(());
        }
        let supersedes = {
            let links = self.shared.links.lock().map_err(|_| Error::Worker)?;
            match links.get(&peer).filter(|link| link.scope.check().is_ok()) {
                Some(link) if link.fact.carrier.protocol == TransportProtocol::NativeUdp => {
                    diagnostic_link(
                        &self.shared,
                        &link.fact,
                        &link.scope,
                        "signed_init_eligible",
                        "udp_predecessor_captured",
                        Some((remote, initiation.client)),
                    );
                    Replacement::Channel(link.fact.id)
                }
                Some(link) => {
                    if self.diagnosed.len() < MAX_REPLAY
                        && self
                            .diagnosed
                            .insert((peer, initiation.client, "healthy_stream"))
                    {
                        diagnostic_link(
                            &self.shared,
                            &link.fact,
                            &link.scope,
                            "signed_init_rejected",
                            "existing_healthy_stream",
                            Some((remote, initiation.client)),
                        );
                    }
                    return Ok(());
                }
                None => Replacement::Ranked,
            }
        };
        self.challenge_source(
            peer,
            carrier,
            initiation,
            remote,
            Kind::Init,
            body,
            supersedes,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn challenge_source(
        &mut self,
        peer: NodeId,
        carrier: Carrier,
        initiation: Initiation,
        remote: SocketAddr,
        intro_kind: Kind,
        body: &[u8],
        supersedes: Replacement,
    ) -> Result<(), Error> {
        let Ok(capacity) = self.shared.capacity.clone().try_acquire_owned() else {
            return Ok(());
        };
        let scope = self.shared.scope(Some((peer, carrier.clone())), true);
        if scope.check().is_err() {
            return Ok(());
        }
        let rebuild = self.replay.len() >= MAX_REPLAY;
        if !rebuild {
            self.replay.insert((peer, initiation.client));
        }
        // At capacity no historical nonce is forgotten and no extra history is
        // allocated. Only this bounded pending slot can admit its fresh ACK.
        // Expiring/retrying the slot generates another random server nonce, so
        // its old ACK is unusable. A copied INIT alone cannot force rebuilding:
        // the current source must finish this challenge and win normal CAS.
        let transcript = initiation.respond(&self.shared.boot)?;
        let challenge = adjacency::encode(
            Kind::Challenge,
            &transcript.message(&self.shared.keys, &self.shared.plan(), Kind::Challenge)?,
        )?;
        // One input produces at most one smaller response, with no autonomous
        // server retry timer before the actual source has completed a challenge.
        if challenge.len() > body.len() + 11 {
            return Ok(());
        }
        let attempt = ServerAttempt {
            peer,
            carrier,
            transcript,
            intro: body.to_vec(),
            intro_kind,
            challenge,
            scope,
            responses: Retry::sent(RETRY),
            capacity,
            supersedes,
            rebuild,
        };
        let _ = send_control(
            &self.sender,
            &attempt.scope,
            remote,
            attempt.challenge.clone(),
        )
        .await;
        self.pending.insert(remote, attempt);
        Ok(())
    }

    async fn ack(&mut self, remote: SocketAddr, body: &[u8]) -> Result<(), Error> {
        let plan = self.shared.plan();
        let cfg = &*plan;
        // Match the pending fresh transcript first. An old channel on the same
        // tuple must neither swallow a fresh ACK nor authorize it by address.
        let pending_matches = self.pending.get(&remote).is_some_and(|attempt| {
            attempt.scope.check().is_ok()
                && attempt
                    .transcript
                    .verify(cfg, attempt.peer, Kind::Ack, body)
                    .is_ok()
        });
        if !pending_matches {
            if let Some(channel) = self.channels.get_mut(&remote)
                && self.listener.is_some()
                && !channel.confirmed
                && Instant::now() < channel.ready_until
                && channel.scope.check().is_ok()
                && Instant::now() >= channel.last_response + RETRY
                && channel
                    .transcript
                    .verify(cfg, channel.fact.peer, Kind::Ack, body)
                    .is_ok()
            {
                channel.last_response = Instant::now();
                let bytes = adjacency::encode(
                    Kind::Ready,
                    &channel.transcript.message(
                        &self.shared.keys,
                        &self.shared.plan(),
                        Kind::Ready,
                    )?,
                )?;
                let _ = send_control(&self.sender, &channel.scope, remote, bytes).await;
            }
            return Ok(());
        }
        let attempt = self.pending.get(&remote).ok_or(Error::Worker)?;
        let bytes = adjacency::encode(
            Kind::Ready,
            &attempt
                .transcript
                .message(&self.shared.keys, &self.shared.plan(), Kind::Ready)?,
        )?;
        let attempt = self.pending.remove(&remote).ok_or(Error::Worker)?;
        let scope = attempt.scope;
        if self.promote(
            &plan,
            attempt.peer,
            attempt.carrier,
            remote,
            attempt.transcript,
            attempt.capacity,
            scope.deadline_or(self.shared.limits.handshake_timeout),
            attempt.supersedes,
            LinkAcquisition::AcceptedInbound,
        )? {
            if attempt.rebuild {
                // Authentication and normal selection succeeded. Close/join
                // this generation and surface Event::Closed, allowing the
                // host's saved-attempt retry to reopen it. Never leave a live
                // listener permanently unable to accept a new channel, or clear
                // its replay/ABA state underneath other pending handshakes.
                tracing::warn!("UDP replay capacity exhausted; connectivity rebuild required");
                self.shared.stop.cancel();
                return Err(Error::Capacity);
            }
            // READY attests completed authentication on the selected owner, not
            // merely a valid ACK for a candidate rejected by selection.
            let _ = send_control(&self.sender, &scope, remote, bytes).await;
        }
        Ok(())
    }

    async fn client_message(
        &mut self,
        remote: SocketAddr,
        kind: Kind,
        body: &[u8],
    ) -> Result<(), Error> {
        if self
            .outbound
            .as_ref()
            .is_some_and(|outbound| outbound.established)
        {
            return self.recovery_message(remote, kind, body).await;
        }
        let Some(outbound) = &mut self.outbound else {
            return Ok(());
        };
        if outbound.established || outbound.scope.check().is_err() {
            return Ok(());
        }
        let Some(attempt) = outbound.attempts.get_mut(&remote) else {
            return Ok(());
        };
        let plan = self.shared.plan();
        let cfg = &*plan;
        if kind == Kind::Challenge {
            let Ok(transcript) = attempt.initiation.challenge(cfg, outbound.peer, body) else {
                return Ok(());
            };
            let candidate = LinkFact {
                id: transcript.id(),
                socket_id: self.id,
                peer: outbound.peer,
                peer_boot: transcript.peer_boot(true),
                carrier: outbound.carrier.clone(),
                local: self.local,
                remote,
                acquisition: outbound.acquisition.clone(),
            };
            if self
                .shared
                .links
                .lock()
                .map_err(|_| Error::Worker)?
                .get(&outbound.peer)
                .is_some_and(|link| {
                    link.scope.check().is_ok()
                        && Some(link.fact.id) != outbound.supersedes
                        && preferred_existing(&link.fact, &candidate)
                })
            {
                // Do not sign a superseding ACK for a stale initial candidate
                // already beaten by this initiator's healthy ranked selection.
                return Err(Error::Unavailable);
            }
            match &attempt.phase {
                ClientPhase::Acked {
                    transcript: old, ..
                } => {
                    if old.id() != transcript.id() {
                        return Ok(());
                    }
                }
                ClientPhase::Init { .. } => {
                    let output = adjacency::encode(
                        Kind::Ack,
                        &transcript.message(&self.shared.keys, &self.shared.plan(), Kind::Ack)?,
                    )?;
                    attempt.phase = ClientPhase::Acked { transcript, output };
                    attempt.retry = Retry::due();
                }
            }
            // Retransmission is timer-paced, including duplicate challenges.
            return Ok(());
        }
        let ClientPhase::Acked { transcript, .. } = &attempt.phase else {
            return Ok(());
        };
        if transcript
            .verify(cfg, outbound.peer, Kind::Ready, body)
            .is_err()
        {
            return Ok(());
        }
        let Some(ClientAttempt {
            phase: ClientPhase::Acked { transcript, .. },
            ..
        }) = outbound.attempts.remove(&remote)
        else {
            return Err(Error::Worker);
        };
        let peer = outbound.peer;
        let carrier = outbound.carrier.clone();
        let capacity = outbound.capacity.take().ok_or(Error::Worker)?;
        outbound.established = true;
        outbound.attempts.clear();
        let deadline = outbound
            .scope
            .deadline_or(self.shared.limits.handshake_timeout);
        let acquisition = outbound.acquisition.clone();
        let supersedes = outbound
            .supersedes
            .map_or(Replacement::Ranked, Replacement::Channel);
        if !self.promote(
            &plan,
            peer,
            carrier,
            remote,
            transcript,
            capacity,
            deadline,
            supersedes,
            acquisition,
        )? {
            return Err(Error::Unavailable);
        }
        Ok(())
    }

    async fn health(&mut self, remote: SocketAddr, kind: Kind, body: &[u8]) -> Result<(), Error> {
        let Some(channel) = self.channels.get_mut(&remote) else {
            // A known channel's peer at a new source roamed; without one
            // (a restarted listener) the source is challenged as recovery.
            return match kind {
                Kind::Ping if self.roam_challenge(remote, body).await? => Ok(()),
                Kind::Ping => self.probe_recovery(remote, body).await,
                Kind::Pong => self.roam_answer(remote, body),
                _ => Ok(()),
            };
        };
        if channel.scope.check().is_err() {
            return Ok(());
        }
        let plan = self.shared.plan();
        let cfg = &*plan;
        let Ok(nonce) = channel
            .transcript
            .verify_health(cfg, channel.fact.peer, kind, body)
        else {
            return Ok(());
        };
        if kind == Kind::Pong {
            if channel.probe != Some(nonce) {
                if channel.mtu.answered(&nonce, Instant::now()) {
                    self.measure_mtu();
                }
                return Ok(());
            }
            if channel.scope.check().is_err() {
                return Ok(());
            }
            channel.probe = None;
            channel.confirmed = true;
            channel.roam = 0;
            channel.roam_until = None;
            channel.unanswered = 0;
            channel.peer_pings = 0;
            channel.answered_at = Instant::now();
            *channel.health.lock().map_err(|_| Error::Worker)? = Instant::now() + HEALTH_WINDOW;
        } else {
            channel.peer_pings = channel.peer_pings.saturating_add(1);
            if Instant::now() < channel.last_response + RETRY {
                return Ok(());
            }
            channel.last_response = Instant::now();
            let bytes = adjacency::encode(
                Kind::Pong,
                &channel.transcript.health(
                    &self.shared.keys,
                    &self.shared.plan(),
                    Kind::Pong,
                    &nonce,
                )?,
            )?;
            let _ = send_control(&self.sender, &channel.scope, remote, bytes).await;
        }
        Ok(())
    }

    /// Answer a peer's size probe with a Pong over its nonce, only if it was
    /// signed for the length actually received.
    async fn answer_size_probe(
        &mut self,
        remote: SocketAddr,
        datagram: usize,
        body: &[u8],
    ) -> Result<(), Error> {
        let Some(channel) = self.channels.get_mut(&remote) else {
            return Ok(());
        };
        if channel.scope.check().is_err() || Instant::now() < channel.last_probe_response + RETRY {
            return Ok(());
        }
        let plan = self.shared.plan();
        let Ok(nonce) = channel
            .transcript
            .verify_probe(&plan, channel.fact.peer, datagram, body)
        else {
            return Ok(());
        };
        channel.last_probe_response = Instant::now();
        let bytes = adjacency::encode(
            Kind::Pong,
            &channel
                .transcript
                .health(&self.shared.keys, &plan, Kind::Pong, &nonce)?,
        )?;
        let _ = send_control(&self.sender, &channel.scope, remote, bytes).await;
        Ok(())
    }

    /// A passive restart cannot dial a dial-only peer. Verify that peer's signed
    /// outstanding Ping, then require a NEW source challenge/ACK. Neither an
    /// unknown datagram nor a copied old probe retires any channel.
    async fn probe_recovery(&mut self, remote: SocketAddr, body: &[u8]) -> Result<(), Error> {
        if self.listener.is_none()
            || self.repeat_intro(remote, Kind::Ping, body).await
            || self.pending.len() + self.channels.len() >= self.shared.limits.connections
        {
            return Ok(());
        }
        let plan = self.shared.plan();
        let cfg = &*plan;
        for (peer, carrier) in self.incoming_carriers()? {
            let Ok((old_id, initiation)) = Initiation::from_probe(cfg, peer, &carrier, body) else {
                continue;
            };
            if self.replay.contains(&(peer, initiation.client))
                || self.pending.values().any(|attempt| attempt.peer == peer)
            {
                return Ok(());
            }
            let supersedes = {
                let links = self.shared.links.lock().map_err(|_| Error::Worker)?;
                match links.get(&peer) {
                    None => Replacement::Vacant(links.last.get(&peer).copied()),
                    Some(link)
                        if link.fact.id == old_id
                            && link.fact.carrier == carrier
                            && link.scope.check().is_ok() =>
                    {
                        Replacement::Channel(old_id)
                    }
                    Some(_) => return Ok(()),
                }
            };
            if diagnostic_event() {
                tracing::debug!(operation = "probe_recovery", peer = %peer, channel = ?old_id, remote = %remote,
                    "Connectivity signed probe challenged");
            }
            return self
                .challenge_source(
                    peer,
                    carrier,
                    initiation,
                    remote,
                    Kind::Ping,
                    body,
                    supersedes,
                )
                .await;
        }
        Ok(())
    }

    async fn recovery_message(
        &mut self,
        remote: SocketAddr,
        kind: Kind,
        body: &[u8],
    ) -> Result<(), Error> {
        let Some(channel) = self.channels.get(&remote) else {
            return Ok(());
        };
        if !self.selected(channel) {
            return Ok(());
        }
        let plan = self.shared.plan();
        let cfg = &*plan;
        if channel.fact.carrier.dialer != cfg.node {
            return Ok(());
        }
        if kind == Kind::Challenge {
            if channel.recovery.is_some() {
                return Ok(());
            }
            let Some(probe) = channel.probe else {
                return Ok(());
            };
            let Ok(transcript) =
                channel
                    .transcript
                    .probe_challenge(cfg, channel.fact.peer, probe, body)
            else {
                return Ok(());
            };
            let Ok(capacity) = self.shared.capacity.clone().try_acquire_owned() else {
                return Ok(());
            };
            let mut scope = self.shared.scope(
                Some((channel.fact.peer, channel.fact.carrier.clone())),
                true,
            );
            // Pending ACK output also dies with the captured predecessor. The
            // replacement receives its own scope only after verified READY/CAS.
            scope.cancellation = channel.scope.cancellation.child_token();
            scope.health = Some(channel.health.clone());
            scope.check()?;
            let output = adjacency::encode(
                Kind::Ack,
                &transcript.message(&self.shared.keys, &self.shared.plan(), Kind::Ack)?,
            )?;
            self.channels
                .get_mut(&remote)
                .ok_or(Error::Worker)?
                .recovery = Some(Recovery {
                transcript,
                scope,
                output,
                retry: Retry::due(),
                capacity,
            });
            return Ok(());
        }
        let Some(recovery) = &channel.recovery else {
            return Ok(());
        };
        if kind != Kind::Ready
            || recovery.scope.check().is_err()
            || recovery
                .transcript
                .verify(cfg, channel.fact.peer, Kind::Ready, body)
                .is_err()
        {
            return Ok(());
        }
        let peer = channel.fact.peer;
        let carrier = channel.fact.carrier.clone();
        let old_id = channel.fact.id;
        let acquisition = channel.fact.acquisition.clone();
        let recovery = self
            .channels
            .get_mut(&remote)
            .ok_or(Error::Worker)?
            .recovery
            .take()
            .ok_or(Error::Worker)?;
        let _ = self.promote(
            &plan,
            peer,
            carrier,
            remote,
            recovery.transcript,
            recovery.capacity,
            recovery
                .scope
                .deadline_or(self.shared.limits.handshake_timeout),
            Replacement::Channel(old_id),
            acquisition,
        )?;
        recovery.scope.cancellation.cancel();
        Ok(())
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
pub(in crate::manager) mod tests {
    use super::super::tests::{netmap::support::Netmaps, support::carriers};
    use super::*;

    const DIALER_BOOT: crate::Boot = [0xd1; 16];
    use meshway_transport::udp::UdpTransport;
    use tokio::net::UdpSocket;

    pub(in crate::manager) fn shared(
        desired: &Desired,
        keys: &DataPlaneKeys,
    ) -> (Arc<Shared>, super::super::events::Receiver) {
        let plan = Plan::new(desired).unwrap();
        let generation = meshway_model::Sequence::MIN.checked_next().unwrap();
        let (events, receiver) = super::super::events::channel(64, 8 * 1024 * 1024);
        let interests = Arc::new(super::super::interest::Interests::new());
        interests.allow(&plan);
        let shared = Arc::new(Shared {
            keys: keys.clone(),
            plans: tokio::sync::watch::channel(plan).0,
            stop: CancellationToken::new(),
            links: Arc::new(Mutex::new(LinkTable::default())),
            events: events.clone(),
            control_events: events,
            capacity: Arc::new(Semaphore::new(64)),
            limits: Limits::default(),
            control_budget: Arc::new(Mutex::new(ControlBudget::new())),
            sockets: Arc::new(Mutex::new(BTreeMap::new())),
            stun_confirmed: Arc::default(),
            stun_samples: Arc::default(),
            path_mtu: Arc::default(),
            size_probes: Arc::default(),
            observation_request: Arc::default(),
            generation,
            traversal_endpoints: Arc::new(Mutex::new(BTreeMap::new())),
            resource_changes: tokio::sync::watch::channel(()).0,
            interests,
            facts: Default::default(),
            instance: Default::default(),
            boot: crate::adjacency::boot().unwrap(),
            unbound: Arc::default(),
            roams: Arc::new(tokio::sync::watch::channel(0).0),
            roam_observation: Arc::default(),
        });
        (shared, receiver)
    }

    /// Bound on every control datagram these tests wait for.
    const CONTROL_WAIT: Duration = Duration::from_secs(5);

    /// An owner endpoint whose write readiness is already observed, so each
    /// control send completes inline within the owner call that makes it. A
    /// socket bound just now has none yet: its first send would wait for the
    /// Session worker, and a runtime stalled beyond [`OUTPUT_WAIT`] drops that
    /// best-effort datagram (as the network may). A real dialer retransmits;
    /// these tests drive the owner directly and send each message once.
    async fn endpoint(socket: UdpSocket) -> UdpTransport {
        socket.writable().await.unwrap();
        UdpTransport::adopt(socket, &Default::default()).unwrap()
    }

    /// Node 1 (`shared`) accepts node 0's real Native UDP handshake: a
    /// selected channel toward node 0 is in its link table while the
    /// returned owner lives.
    pub(in crate::manager) async fn accepted_link(
        fixture: &mut Netmaps,
        shared: &Arc<Shared>,
    ) -> Box<dyn std::any::Any + Send> {
        let ids = fixture.ids();
        let socket = endpoint(fixture.take_udp(1)).await;
        let local = socket.local_addr().unwrap();
        let id = adjacency::nonce().unwrap();
        let mut owner = Owner::new(shared.clone(), Box::new(socket), id).unwrap();
        owner.listener = Some(ListenerFact {
            socket_id: id,
            key: "udp".into(),
            protocol: TransportProtocol::NativeUdp,
            local,
            buffers: None,
        });
        let dialer = Plan::new(&fixture.desired[0]).unwrap();
        let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let (initiation, intro) =
            Initiation::new(&fixture.keys[0], &dialer, &carrier, &DIALER_BOOT).unwrap();
        owner
            .init(peer.local_addr().unwrap(), &intro)
            .await
            .unwrap();
        let mut bytes = [0; adjacency::MAX_MESSAGE];
        let (len, _) = control(&peer, &mut bytes).await;
        let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
        let transcript = initiation.challenge(&dialer, ids[1], challenge).unwrap();
        let ack = transcript
            .message(&fixture.keys[0], &dialer, Kind::Ack)
            .unwrap();
        owner.ack(peer.local_addr().unwrap(), &ack).await.unwrap();
        assert!(shared.links.lock().unwrap().contains_key(&ids[0]));
        Box::new(owner)
    }

    /// The next datagram `client` receives, within [`CONTROL_WAIT`].
    async fn control(client: &UdpSocket, bytes: &mut [u8]) -> (usize, SocketAddr) {
        tokio::time::timeout(CONTROL_WAIT, client.recv_from(bytes))
            .await
            .expect("a control datagram within CONTROL_WAIT")
            .unwrap()
    }

    #[tokio::test]
    async fn replay_exhaustion_rebuilds_instead_of_forgetting_nonces_and_old_ack_cannot_install() {
        let mut fixture = Netmaps::start(false, true).await;
        let ids = fixture.ids();
        let (shared, _events) = shared(&fixture.desired[1], &fixture.keys[1]);
        let socket = endpoint(fixture.take_udp(1)).await;
        let local = socket.local_addr().unwrap();
        let id = adjacency::nonce().unwrap();
        let listener = ListenerFact {
            socket_id: id,
            key: "udp".into(),
            protocol: TransportProtocol::NativeUdp,
            local,
            buffers: None,
        };
        shared
            .register_socket(
                id,
                meshway_model::state::ResourceRef::Listener { key: "udp".into() },
                TransportProtocol::NativeUdp,
                local,
            )
            .unwrap();
        let mut owner = Owner::new(shared.clone(), Box::new(socket), id).unwrap();
        owner.listener = Some(listener.clone());
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let source = client.local_addr().unwrap();
        let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
        let (initiation, intro) = Initiation::new(
            &fixture.keys[0],
            &Plan::new(&fixture.desired[0]).unwrap(),
            &carrier,
            &DIALER_BOOT,
        )
        .unwrap();
        owner.init(source, &intro).await.unwrap();
        let mut bytes = [0; adjacency::MAX_MESSAGE];
        let (len, from) = control(&client, &mut bytes).await;
        assert_eq!(from, local);
        let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
        let old = initiation
            .challenge(&Plan::new(&fixture.desired[0]).unwrap(), ids[1], challenge)
            .unwrap();
        let old_ack = old
            .message(
                &fixture.keys[0],
                &Plan::new(&fixture.desired[0]).unwrap(),
                Kind::Ack,
            )
            .unwrap();
        owner.ack(source, &old_ack).await.unwrap();
        let (len, _) = control(&client, &mut bytes).await;
        assert_eq!(adjacency::decode(&bytes[..len]).unwrap().0, Kind::Ready);
        assert_eq!(owner.channels[&source].fact.id, old.id());

        // Seed only already-retired nonce metadata to exercise the production
        // capacity boundary without thousands of redundant crypto exchanges.
        // The triggering packets below still require real registered signatures.
        while owner.replay.len() < MAX_REPLAY {
            owner.replay.insert((ids[0], adjacency::nonce().unwrap()));
        }
        let history = owner.replay.clone();
        // New content keeping every identity and carrier: the Manager's apply
        // keeps this channel; the owner only drops in-flight acquisition.
        let next = Plan::new(&fixture.reserial(1).await).unwrap();
        let previous = shared.plan();
        shared.plans.send_replace(next.clone());
        shared.retain_table(&previous, &next).unwrap();
        owner.reconfigured().unwrap();
        let live = owner.channels[&source].scope.clone();
        assert_eq!(
            owner.replay, history,
            "renewal must not forget replay state"
        );
        let mut invalid = intro.clone();
        *invalid.last_mut().unwrap() ^= 1;
        owner.init(source, &invalid).await.unwrap();
        owner.init(source, &intro).await.unwrap();
        live.check().unwrap();
        assert!(
            !shared.stop.is_cancelled(),
            "invalid/replayed traffic cannot force rebuilding"
        );
        let (rebuild_init, fresh_intro) = Initiation::new(
            &fixture.keys[0],
            &Plan::new(&fixture.desired[0]).unwrap(),
            &carrier,
            &DIALER_BOOT,
        )
        .unwrap();
        owner.init(source, &fresh_intro).await.unwrap();
        live.check().unwrap();
        assert!(
            !shared.stop.is_cancelled(),
            "an INIT alone cannot retire channels"
        );
        let (len, _) = control(&client, &mut bytes).await;
        let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
        let rebuild = rebuild_init
            .challenge(&Plan::new(&fixture.desired[0]).unwrap(), ids[1], challenge)
            .unwrap();
        let expired_ack = rebuild
            .message(
                &fixture.keys[0],
                &Plan::new(&fixture.desired[0]).unwrap(),
                Kind::Ack,
            )
            .unwrap();
        owner
            .pending
            .remove(&source)
            .unwrap()
            .scope
            .cancellation
            .cancel();
        owner.init(source, &fresh_intro).await.unwrap();
        let (len, _) = control(&client, &mut bytes).await;
        let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
        let retried = rebuild_init
            .challenge(&Plan::new(&fixture.desired[0]).unwrap(), ids[1], challenge)
            .unwrap();
        assert_ne!(
            retried.id(),
            rebuild.id(),
            "retrying an unrecorded capacity slot must use a fresh server nonce"
        );
        owner.ack(source, &expired_ack).await.unwrap();
        assert!(!shared.stop.is_cancelled());
        owner.ack(source, &old_ack).await.unwrap();
        assert!(!shared.stop.is_cancelled());
        let ack = retried
            .message(
                &fixture.keys[0],
                &Plan::new(&fixture.desired[0]).unwrap(),
                Kind::Ack,
            )
            .unwrap();
        let impostor = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        owner
            .ack(impostor.local_addr().unwrap(), &ack)
            .await
            .unwrap();
        assert!(!shared.stop.is_cancelled());
        live.check().unwrap();
        assert!(matches!(
            owner.ack(source, &ack).await,
            Err(Error::Capacity)
        ));
        assert!(live.check().is_err());
        assert!(
            shared.stop.is_cancelled(),
            "the Manager recv boundary must surface Closed"
        );
        assert_eq!(owner.replay, history);
        owner.shutdown().await.unwrap();
        drop(owner);

        // This is a new physical owner, the same path as a host reinstall. An
        // old INIT is harmless only because a fresh source challenge is required.
        let (restarted, _events) = self::shared(&fixture.desired[1], &fixture.keys[1]);
        let socket = endpoint(UdpSocket::bind(local).await.unwrap()).await;
        let socket_id = adjacency::nonce().unwrap();
        let mut owner = Owner::new(restarted.clone(), Box::new(socket), socket_id).unwrap();
        owner.listener = Some(ListenerFact {
            socket_id,
            ..listener
        });
        owner.init(source, &intro).await.unwrap();
        let (len, from) = control(&client, &mut bytes).await;
        assert_eq!(from, local);
        let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
        let fresh = initiation
            .challenge(&Plan::new(&fixture.desired[0]).unwrap(), ids[1], challenge)
            .unwrap();
        assert_ne!(fresh.id(), old.id());
        owner.ack(source, &old_ack).await.unwrap();
        assert!(owner.channels.is_empty());
        let fresh_ack = fresh
            .message(
                &fixture.keys[0],
                &Plan::new(&fixture.desired[0]).unwrap(),
                Kind::Ack,
            )
            .unwrap();
        let impostor = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        owner
            .ack(impostor.local_addr().unwrap(), &fresh_ack)
            .await
            .unwrap();
        assert!(owner.channels.is_empty());
        owner.ack(source, &fresh_ack).await.unwrap();
        assert_eq!(restarted.links.lock().unwrap()[&ids[0]].fact.id, fresh.id());
        // The dialer's state survived; the listener restarted. Each side
        // reads the other's signed boot from the fresh transcript.
        assert_eq!(
            restarted.links.lock().unwrap()[&ids[0]].fact.peer_boot,
            DIALER_BOOT
        );
        assert_eq!(fresh.peer_boot(true), restarted.boot);
        assert_ne!(fresh.peer_boot(true), old.peer_boot(true));
        let fresh_ready = fresh
            .message(&restarted.keys, &restarted.plan(), Kind::Ready)
            .unwrap();
        assert!(
            old.verify(
                &Plan::new(&fixture.desired[0]).unwrap(),
                ids[1],
                Kind::Ready,
                &fresh_ready
            )
            .is_err()
        );
        owner.shutdown().await.unwrap();
        fixture.close().await;
    }

    #[tokio::test]
    async fn server_source_challenge_replay_and_confirmation_use_real_registered_keys() {
        let mut fixture = Netmaps::start(false, true).await;
        let ids = fixture.ids();
        let (shared, _receiver) = shared(&fixture.desired[1], &fixture.keys[1]);
        let socket = endpoint(fixture.take_udp(1)).await;
        let local = socket.local_addr().unwrap();
        let id = adjacency::nonce().unwrap();
        let mut owner = Owner::new(shared.clone(), Box::new(socket), id).unwrap();
        owner.listener = Some(ListenerFact {
            socket_id: id,
            key: "udp".into(),
            protocol: TransportProtocol::NativeUdp,
            local,
            buffers: None,
        });
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let source = client.local_addr().unwrap();
        let impostor = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let wrong_source = impostor.local_addr().unwrap();
        let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
        let (initiation, intro) = Initiation::new(
            &fixture.keys[0],
            &Plan::new(&fixture.desired[0]).unwrap(),
            &carrier,
            &DIALER_BOOT,
        )
        .unwrap();
        let mut invalid = intro.clone();
        *invalid.last_mut().unwrap() ^= 1;
        owner.init(wrong_source, &invalid).await.unwrap();
        assert!(owner.pending.is_empty());
        owner.init(source, &intro).await.unwrap();
        let expected_id = owner.pending[&source].transcript.id();
        owner.pending.get_mut(&source).unwrap().responses = Retry::due();
        owner.init(source, &intro).await.unwrap();
        assert_eq!(owner.pending[&source].transcript.id(), expected_id);
        owner.init(wrong_source, &intro).await.unwrap();
        assert_eq!(owner.pending.len(), 1);
        let mut bytes = [0; adjacency::MAX_MESSAGE];
        let (len, actual) = control(&client, &mut bytes).await;
        assert_eq!(actual, local);
        let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
        assert_eq!(kind, Kind::Challenge);
        let transcript = initiation
            .challenge(&Plan::new(&fixture.desired[0]).unwrap(), ids[1], body)
            .unwrap();
        let ack = transcript
            .message(
                &fixture.keys[0],
                &Plan::new(&fixture.desired[0]).unwrap(),
                Kind::Ack,
            )
            .unwrap();
        owner.ack(wrong_source, &ack).await.unwrap();
        assert!(owner.channels.is_empty());
        let wrong_peer = transcript
            .message(
                &fixture.keys[2],
                &Plan::new(&fixture.desired[2]).unwrap(),
                Kind::Ack,
            )
            .unwrap();
        owner.ack(source, &wrong_peer).await.unwrap();
        assert!(owner.channels.is_empty());
        owner.ack(source, &ack).await.unwrap();
        assert_eq!(owner.channels[&source].fact.id, transcript.id());
        assert_eq!(shared.links.lock().unwrap().len(), 1);
        let ready = transcript
            .message(
                &fixture.keys[1],
                &Plan::new(&fixture.desired[1]).unwrap(),
                Kind::Ready,
            )
            .unwrap();
        let plan = Plan::new(&fixture.desired[0]).unwrap();
        let cfg = &*plan;
        transcript.verify(cfg, ids[1], Kind::Ready, &ready).unwrap();
        assert!(
            transcript
                .verify(cfg, ids[1], Kind::Challenge, &ready)
                .is_err()
        );
        let nonce = adjacency::nonce().unwrap();
        let pong = transcript
            .health(
                &fixture.keys[1],
                &Plan::new(&fixture.desired[1]).unwrap(),
                Kind::Pong,
                &nonce,
            )
            .unwrap();
        assert_eq!(
            transcript
                .verify_health(cfg, ids[1], Kind::Pong, &pong)
                .unwrap(),
            nonce
        );
        assert!(
            transcript
                .verify_health(cfg, ids[1], Kind::Ping, &pong)
                .is_err()
        );
        // A fresh challenge changes channel identity; old ACK/health proofs fail.
        let (_, _, fresh) =
            adjacency::incoming(&shared.plan(), &[(ids[0], carrier)], &intro).unwrap();
        let fresh = fresh.respond(&DIALER_BOOT).unwrap();
        assert!(
            fresh
                .verify(&shared.plan(), ids[0], Kind::Ack, &ack)
                .is_err()
        );
        assert!(fresh.verify_health(cfg, ids[1], Kind::Pong, &pong).is_err());
        let peer_pong = transcript
            .health(
                &fixture.keys[0],
                &Plan::new(&fixture.desired[0]).unwrap(),
                Kind::Pong,
                &nonce,
            )
            .unwrap();
        let before = *owner.channels[&source].health.lock().unwrap();
        owner.health(source, Kind::Pong, &peer_pong).await.unwrap();
        assert_eq!(*owner.channels[&source].health.lock().unwrap(), before);
        owner.channels.get_mut(&source).unwrap().probe = Some(nonce);
        owner
            .health(wrong_source, Kind::Pong, &peer_pong)
            .await
            .unwrap();
        assert_eq!(*owner.channels[&source].health.lock().unwrap(), before);
        owner.health(source, Kind::Pong, &peer_pong).await.unwrap();
        let renewed = *owner.channels[&source].health.lock().unwrap();
        assert!(renewed >= before);
        assert!(owner.channels[&source].probe.is_none());
        owner.health(source, Kind::Pong, &peer_pong).await.unwrap();
        assert_eq!(*owner.channels[&source].health.lock().unwrap(), renewed);
        // Hold the consumer idle until its business event queue is saturated.
        // An authenticated Pong at the tail of a hot batch must still renew the
        // exact channel, rather than being dropped or waiting on that consumer.
        for _ in 0..8 {
            let nonce = adjacency::nonce().unwrap();
            owner.channels.get_mut(&source).unwrap().probe = Some(nonce);
            let body = transcript
                .health(
                    &fixture.keys[0],
                    &Plan::new(&fixture.desired[0]).unwrap(),
                    Kind::Pong,
                    &nonce,
                )
                .unwrap();
            let mut frames = (0..31)
                .map(|_| Frame::new(vec![7; 1200], Some(source)))
                .collect::<Vec<_>>();
            frames.push(Frame::new(
                adjacency::encode(Kind::Pong, &body).unwrap(),
                Some(source),
            ));
            owner
                .receive_batch(frames, std::time::Instant::now())
                .await
                .unwrap();
            assert!(owner.channels[&source].probe.is_none());
            assert_eq!(owner.channels[&source].fact.id, transcript.id());
            owner.channels[&source].scope.check().unwrap();
        }
        assert_eq!(
            shared.events.capacity(),
            0,
            "business queue really saturated"
        );
        owner.shutdown().await.unwrap();
        fixture.close().await;
    }

    /// A peer that roamed: its signed Ping of the existing channel arrives
    /// from a new source. The owner challenges that source with a fresh Ping
    /// of the same channel and moves the channel there only on the peer's
    /// Pong over that nonce from it: same channel identity and boot, new
    /// remote. An unsolicited Pong or an unanswered challenge moves nothing.
    #[tokio::test]
    async fn a_roaming_peer_is_followed_only_after_its_new_source_answers_a_challenge() {
        let mut fixture = Netmaps::start(false, true).await;
        let ids = fixture.ids();
        let (shared, _receiver) = shared(&fixture.desired[1], &fixture.keys[1]);
        let socket = endpoint(fixture.take_udp(1)).await;
        let local = socket.local_addr().unwrap();
        let id = adjacency::nonce().unwrap();
        let mut owner = Owner::new(shared.clone(), Box::new(socket), id).unwrap();
        owner.listener = Some(ListenerFact {
            socket_id: id,
            key: "udp".into(),
            protocol: TransportProtocol::NativeUdp,
            local,
            buffers: None,
        });
        let dialer = Plan::new(&fixture.desired[0]).unwrap();
        let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
        let old = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let (initiation, intro) =
            Initiation::new(&fixture.keys[0], &dialer, &carrier, &DIALER_BOOT).unwrap();
        owner.init(old.local_addr().unwrap(), &intro).await.unwrap();
        let mut bytes = [0; adjacency::MAX_MESSAGE];
        let (len, _) = control(&old, &mut bytes).await;
        let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
        let transcript = initiation.challenge(&dialer, ids[1], challenge).unwrap();
        let ack = transcript
            .message(&fixture.keys[0], &dialer, Kind::Ack)
            .unwrap();
        owner.ack(old.local_addr().unwrap(), &ack).await.unwrap();
        let channel = owner.channels[&old.local_addr().unwrap()].fact.clone();
        let ping = |nonce: &[u8; 32]| {
            transcript
                .health(&fixture.keys[0], &dialer, Kind::Ping, nonce)
                .unwrap()
        };
        // An unsolicited Pong from anywhere changes nothing.
        let moved = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let source = moved.local_addr().unwrap();
        let pong = transcript
            .health(
                &fixture.keys[0],
                &dialer,
                Kind::Pong,
                &adjacency::nonce().unwrap(),
            )
            .unwrap();
        owner.health(source, Kind::Pong, &pong).await.unwrap();
        assert!(owner.channels.contains_key(&old.local_addr().unwrap()));
        // A Ping from the new source: the source is challenged, not trusted.
        owner
            .health(source, Kind::Ping, &ping(&adjacency::nonce().unwrap()))
            .await
            .unwrap();
        assert!(owner.channels.contains_key(&old.local_addr().unwrap()));
        assert!(!owner.channels.contains_key(&source));
        let nonce = loop {
            let (len, from) = control(&moved, &mut bytes).await;
            assert_eq!(from, local);
            let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
            if kind == Kind::Ping {
                break transcript
                    .verify_health(&dialer, ids[1], Kind::Ping, body)
                    .unwrap();
            }
        };
        // A Pong over another nonce, or from another source, moves nothing.
        let wrong = transcript
            .health(
                &fixture.keys[0],
                &dialer,
                Kind::Pong,
                &adjacency::nonce().unwrap(),
            )
            .unwrap();
        owner.health(source, Kind::Pong, &wrong).await.unwrap();
        let answer = transcript
            .health(&fixture.keys[0], &dialer, Kind::Pong, &nonce)
            .unwrap();
        let elsewhere = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        owner
            .health(elsewhere.local_addr().unwrap(), Kind::Pong, &answer)
            .await
            .unwrap();
        assert!(owner.channels.contains_key(&old.local_addr().unwrap()));
        // The answer from the challenged source moves the same channel.
        owner.health(source, Kind::Pong, &answer).await.unwrap();
        assert!(!owner.channels.contains_key(&old.local_addr().unwrap()));
        let followed = &owner.channels[&source].fact;
        assert_eq!(followed.id, channel.id);
        assert_eq!(followed.peer_boot, channel.peer_boot);
        assert_eq!(followed.remote, source);
        let link = shared.links.lock().unwrap()[&ids[0]].fact.clone();
        assert_eq!((link.id, link.remote), (channel.id, source));
        owner.channels[&source].scope.check().unwrap();
        owner.shutdown().await.unwrap();
        fixture.close().await;
    }

    /// After a path change a channel this Node can acquire again must answer
    /// its roaming probes within `ROAM_ANSWER`, or it ends at once rather than
    /// after the health window; an answered one stays. An inbound channel is
    /// the peer's to dial again and only keeps probing.
    #[tokio::test]
    async fn an_unanswered_roam_ends_an_acquirable_channel_and_keeps_an_inbound_one() {
        let mut fixture = Netmaps::start(false, true).await;
        let ids = fixture.ids();
        let (shared, _receiver) = shared(&fixture.desired[1], &fixture.keys[1]);
        let socket = endpoint(fixture.take_udp(1)).await;
        let local = socket.local_addr().unwrap();
        let id = adjacency::nonce().unwrap();
        let mut owner = Owner::new(shared.clone(), Box::new(socket), id).unwrap();
        owner.listener = Some(ListenerFact {
            socket_id: id,
            key: "udp".into(),
            protocol: TransportProtocol::NativeUdp,
            local,
            buffers: None,
        });
        let dialer = Plan::new(&fixture.desired[0]).unwrap();
        let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let remote = peer.local_addr().unwrap();
        let (initiation, intro) =
            Initiation::new(&fixture.keys[0], &dialer, &carrier, &DIALER_BOOT).unwrap();
        owner.init(remote, &intro).await.unwrap();
        let mut bytes = [0; adjacency::MAX_MESSAGE];
        let (len, _) = control(&peer, &mut bytes).await;
        let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
        let transcript = initiation.challenge(&dialer, ids[1], challenge).unwrap();
        let ack = transcript
            .message(&fixture.keys[0], &dialer, Kind::Ack)
            .unwrap();
        owner.ack(remote, &ack).await.unwrap();
        assert_eq!(
            owner.channels[&remote].fact.acquisition,
            LinkAcquisition::AcceptedInbound
        );
        owner.roamed();
        assert!(owner.channels[&remote].roam_until.is_none(), "inbound");
        // The same channel as one this Node dialed: answered probes keep it.
        owner.channels.get_mut(&remote).unwrap().fact.acquisition =
            LinkAcquisition::PublishedEndpoint;
        owner.roamed();
        assert!(owner.channels[&remote].roam_until.is_some());
        owner.tick().await.unwrap();
        let nonce = loop {
            let (len, _) = control(&peer, &mut bytes).await;
            let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
            if kind == Kind::Ping {
                break transcript
                    .verify_health(&dialer, ids[1], Kind::Ping, body)
                    .unwrap();
            }
        };
        let pong = transcript
            .health(&fixture.keys[0], &dialer, Kind::Pong, &nonce)
            .unwrap();
        owner.health(remote, Kind::Pong, &pong).await.unwrap();
        assert!(owner.channels[&remote].roam_until.is_none(), "answered");
        // Unanswered past the bound: the channel and its link end now.
        owner.roamed();
        owner.channels.get_mut(&remote).unwrap().roam_until = Some(Instant::now());
        owner.channels[&remote].scope.check().unwrap();
        owner.tick().await.unwrap();
        assert!(!owner.channels.contains_key(&remote));
        assert!(!shared.links.lock().unwrap().contains_key(&ids[0]));
        owner.shutdown().await.unwrap();
        fixture.close().await;
    }

    /// A real UDP endpoint whose route claims jumbo frames.
    struct Jumbo<T>(T);

    impl<T: Datagram> Datagram for Jumbo<T> {
        fn local_addr(&self) -> std::io::Result<SocketAddr> {
            self.0.local_addr()
        }
        fn poll_send_to(
            &self,
            cx: &mut std::task::Context<'_>,
            payload: &[u8],
            peer: SocketAddr,
        ) -> std::task::Poll<std::io::Result<usize>> {
            self.0.poll_send_to(cx, payload, peer)
        }
        fn poll_recv_from(
            &self,
            cx: &mut std::task::Context<'_>,
            buffer: &mut [u8],
        ) -> std::task::Poll<std::io::Result<meshway_transport::raw::DatagramRead>> {
            self.0.poll_recv_from(cx, buffer)
        }
        fn path_mtu(&self, _peer: SocketAddr) -> Option<u16> {
            Some(9000)
        }
    }

    /// Route MTU 9000: the owner's size probe stays within the codec (the
    /// search is capped at the largest encodable probe) and the owner keeps
    /// running. A peer's probe is answered only at the length it was signed
    /// for: a truncated copy gets no Pong.
    #[tokio::test]
    async fn size_probes_stay_encodable_on_a_jumbo_route_and_bind_their_length() {
        let mut fixture = Netmaps::start(false, true).await;
        let ids = fixture.ids();
        let (shared, _receiver) = shared(&fixture.desired[1], &fixture.keys[1]);
        let socket = endpoint(fixture.take_udp(1)).await;
        let local = socket.local_addr().unwrap();
        let id = adjacency::nonce().unwrap();
        let mut owner = Owner::new(shared.clone(), Box::new(Jumbo(socket)), id).unwrap();
        owner.listener = Some(ListenerFact {
            socket_id: id,
            key: "udp".into(),
            protocol: TransportProtocol::NativeUdp,
            local,
            buffers: None,
        });
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let source = client.local_addr().unwrap();
        let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
        let dialer = Plan::new(&fixture.desired[0]).unwrap();
        let (initiation, intro) =
            Initiation::new(&fixture.keys[0], &dialer, &carrier, &DIALER_BOOT).unwrap();
        owner.init(source, &intro).await.unwrap();
        let mut bytes = vec![0; 65_536];
        let (len, _) = control(&client, &mut bytes).await;
        let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
        assert_eq!(kind, Kind::Challenge);
        let transcript = initiation.challenge(&dialer, ids[1], body).unwrap();
        let ack = transcript
            .message(&fixture.keys[0], &dialer, Kind::Ack)
            .unwrap();
        owner.ack(source, &ack).await.unwrap();
        assert!(owner.channels.contains_key(&source));
        owner.tick().await.unwrap();
        let probe = loop {
            let (len, _) = control(&client, &mut bytes).await;
            if adjacency::decode(&bytes[..len]).unwrap().0 == Kind::Probe {
                break bytes[..len].to_vec();
            }
        };
        assert_eq!(probe.len(), adjacency::MAX_PROBE);
        let (_, body) = adjacency::decode(&probe).unwrap();
        let owner_plan = Plan::new(&fixture.desired[1]).unwrap();
        assert!(
            transcript
                .verify_probe(&dialer, ids[1], probe.len(), body)
                .is_ok()
        );
        assert!(
            transcript
                .verify_probe(&dialer, ids[1], 1300, body)
                .is_err()
        );
        // The peer's probe: truncated, then whole.
        let nonce = adjacency::nonce().unwrap();
        let signed = transcript
            .probe(&fixture.keys[0], &dialer, &nonce, 1300)
            .unwrap();
        assert!(
            transcript
                .probe(&fixture.keys[0], &dialer, &nonce, 9000 - 28)
                .is_err()
        );
        let (_, body) = adjacency::decode(&signed[..1200]).unwrap();
        let before = owner.channels[&source].last_probe_response;
        owner.answer_size_probe(source, 1200, body).await.unwrap();
        assert_eq!(
            owner.channels[&source].last_probe_response, before,
            "a truncated probe is not answered"
        );
        let (_, body) = adjacency::decode(&signed).unwrap();
        owner
            .answer_size_probe(source, signed.len(), body)
            .await
            .unwrap();
        let pong = loop {
            let (len, _) = control(&client, &mut bytes).await;
            let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
            if kind == Kind::Pong {
                break transcript
                    .verify_health(&owner_plan, ids[1], Kind::Pong, body)
                    .unwrap();
            }
        };
        assert_eq!(pong, nonce, "only the whole probe is answered");
        owner.shutdown().await.unwrap();
        fixture.close().await;
    }

    /// A real UDP endpoint whose sends stay pending while `stalled`.
    struct Stalled<T> {
        inner: T,
        stalled: Arc<std::sync::atomic::AtomicBool>,
    }

    impl<T: Datagram> Datagram for Stalled<T> {
        fn local_addr(&self) -> std::io::Result<SocketAddr> {
            self.inner.local_addr()
        }
        fn poll_send_to(
            &self,
            cx: &mut std::task::Context<'_>,
            payload: &[u8],
            peer: SocketAddr,
        ) -> std::task::Poll<std::io::Result<usize>> {
            if self.stalled.load(std::sync::atomic::Ordering::SeqCst) {
                return std::task::Poll::Pending;
            }
            self.inner.poll_send_to(cx, payload, peer)
        }
        fn poll_recv_from(
            &self,
            cx: &mut std::task::Context<'_>,
            buffer: &mut [u8],
        ) -> std::task::Poll<std::io::Result<meshway_transport::raw::DatagramRead>> {
            self.inner.poll_recv_from(cx, buffer)
        }
    }

    /// A control send not completed within [`OUTPUT_WAIT`] is dropped like a
    /// lost datagram, and the owner never waits longer: the dialer's repeated
    /// INIT is answered with the same stored challenge.
    #[tokio::test]
    async fn a_challenge_dropped_at_the_output_wait_is_resent_on_the_repeated_init() {
        let mut fixture = Netmaps::start(false, true).await;
        let ids = fixture.ids();
        let (shared, _receiver) = shared(&fixture.desired[1], &fixture.keys[1]);
        let socket = endpoint(fixture.take_udp(1)).await;
        let local = socket.local_addr().unwrap();
        let stalled = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let id = adjacency::nonce().unwrap();
        let endpoint = Stalled {
            inner: socket,
            stalled: stalled.clone(),
        };
        let mut owner = Owner::new(shared.clone(), Box::new(endpoint), id).unwrap();
        owner.listener = Some(ListenerFact {
            socket_id: id,
            key: "udp".into(),
            protocol: TransportProtocol::NativeUdp,
            local,
            buffers: None,
        });
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let source = client.local_addr().unwrap();
        let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
        let dialer = Plan::new(&fixture.desired[0]).unwrap();
        let (initiation, intro) =
            Initiation::new(&fixture.keys[0], &dialer, &carrier, &DIALER_BOOT).unwrap();
        let started = Instant::now();
        owner.init(source, &intro).await.unwrap();
        assert!(
            started.elapsed() >= OUTPUT_WAIT,
            "the send waited out its bound"
        );
        let challenge = owner.pending[&source].challenge.clone();
        let mut bytes = [0; adjacency::MAX_MESSAGE];
        assert!(
            tokio::time::timeout(RETRY, client.recv_from(&mut bytes))
                .await
                .is_err(),
            "the dropped challenge never leaves"
        );
        stalled.store(false, std::sync::atomic::Ordering::SeqCst);
        owner.init(source, &intro).await.unwrap();
        let (len, from) = control(&client, &mut bytes).await;
        assert_eq!(from, local);
        assert_eq!(&bytes[..len], challenge.as_slice());
        let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
        assert_eq!(kind, Kind::Challenge);
        initiation.challenge(&dialer, ids[1], body).unwrap();
        owner.shutdown().await.unwrap();
        fixture.close().await;
    }

    /// A real UDP endpoint with a fixed 1500-byte route that records the
    /// send grants of roaming peers, at most four live of each kind (the
    /// Underlay's bound).
    struct Granting<T> {
        inner: T,
        challenges: Grants,
        sources: Grants,
    }

    type Grants = Arc<Mutex<Vec<(SocketAddr, Arc<dyn meshway_transport::SendLifetime>)>>>;

    fn grant(
        grants: &Grants,
        peer: SocketAddr,
        lifetime: Arc<dyn meshway_transport::SendLifetime>,
    ) -> std::io::Result<()> {
        let mut grants = grants.lock().unwrap();
        grants.retain(|(address, lifetime)| lifetime.is_current() && *address != peer);
        if grants.len() >= 4 {
            return Err(std::io::ErrorKind::QuotaExceeded.into());
        }
        grants.push((peer, lifetime));
        Ok(())
    }

    fn live(grants: &Grants) -> Vec<SocketAddr> {
        grants
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, lifetime)| lifetime.is_current())
            .map(|(address, _)| *address)
            .collect()
    }

    impl<T: Datagram> Datagram for Granting<T> {
        fn local_addr(&self) -> std::io::Result<SocketAddr> {
            self.inner.local_addr()
        }
        fn poll_send_to(
            &self,
            cx: &mut std::task::Context<'_>,
            payload: &[u8],
            peer: SocketAddr,
        ) -> std::task::Poll<std::io::Result<usize>> {
            self.inner.poll_send_to(cx, payload, peer)
        }
        fn poll_recv_from(
            &self,
            cx: &mut std::task::Context<'_>,
            buffer: &mut [u8],
        ) -> std::task::Poll<std::io::Result<meshway_transport::raw::DatagramRead>> {
            self.inner.poll_recv_from(cx, buffer)
        }
        fn path_mtu(&self, _peer: SocketAddr) -> Option<u16> {
            Some(1500)
        }
        fn authorize_challenge(
            &self,
            peer: SocketAddr,
            lifetime: Arc<dyn meshway_transport::SendLifetime>,
        ) -> std::io::Result<()> {
            grant(&self.challenges, peer, lifetime)
        }
        fn authorize_source(
            &self,
            peer: SocketAddr,
            lifetime: Arc<dyn meshway_transport::SendLifetime>,
        ) -> std::io::Result<()> {
            grant(&self.sources, peer, lifetime)
        }
    }

    /// One accepted channel from node 0 over a [`Granting`] endpoint.
    struct Roaming {
        fixture: Netmaps,
        shared: Arc<Shared>,
        owner: Owner,
        transcript: Transcript,
        dialer: Arc<Plan>,
        sources: Grants,
        peer: UdpSocket,
    }

    async fn roaming() -> Roaming {
        let mut fixture = Netmaps::start(false, true).await;
        let ids = fixture.ids();
        let (shared, _receiver) = shared(&fixture.desired[1], &fixture.keys[1]);
        let socket = endpoint(fixture.take_udp(1)).await;
        let local = socket.local_addr().unwrap();
        let sources = Grants::default();
        let endpoint = Granting {
            inner: socket,
            challenges: Grants::default(),
            sources: sources.clone(),
        };
        let id = adjacency::nonce().unwrap();
        let mut owner = Owner::new(shared.clone(), Box::new(endpoint), id).unwrap();
        owner.listener = Some(ListenerFact {
            socket_id: id,
            key: "udp".into(),
            protocol: TransportProtocol::NativeUdp,
            local,
            buffers: None,
        });
        let dialer = Plan::new(&fixture.desired[0]).unwrap();
        let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let (initiation, intro) =
            Initiation::new(&fixture.keys[0], &dialer, &carrier, &DIALER_BOOT).unwrap();
        owner
            .init(peer.local_addr().unwrap(), &intro)
            .await
            .unwrap();
        let mut bytes = [0; adjacency::MAX_MESSAGE];
        let (len, _) = control(&peer, &mut bytes).await;
        let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
        let transcript = initiation.challenge(&dialer, ids[1], challenge).unwrap();
        let ack = transcript
            .message(&fixture.keys[0], &dialer, Kind::Ack)
            .unwrap();
        owner.ack(peer.local_addr().unwrap(), &ack).await.unwrap();
        Roaming {
            fixture,
            shared,
            owner,
            transcript,
            dialer,
            sources,
            peer,
        }
    }

    /// F6a B3: a channel follows its peer through six moves. Each move
    /// grants its new source and revokes the previous one, so one live grant
    /// remains (the endpoint's bound of four is never reached).
    #[tokio::test]
    async fn a_channel_follows_six_moves_holding_one_source_grant() {
        let Roaming {
            fixture,
            mut owner,
            transcript,
            dialer,
            sources,
            peer,
            ..
        } = roaming().await;
        let ids = fixture.ids();
        let id = owner.channels[&peer.local_addr().unwrap()].fact.id;
        let mut bytes = [0; adjacency::MAX_MESSAGE];
        let mut sockets = vec![peer];
        for step in 1..=6 {
            let moved = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let source = moved.local_addr().unwrap();
            let ping = transcript
                .health(
                    &fixture.keys[0],
                    &dialer,
                    Kind::Ping,
                    &adjacency::nonce().unwrap(),
                )
                .unwrap();
            owner.health(source, Kind::Ping, &ping).await.unwrap();
            let nonce = loop {
                let (len, _) = control(&moved, &mut bytes).await;
                let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
                if kind == Kind::Ping {
                    break transcript
                        .verify_health(&dialer, ids[1], Kind::Ping, body)
                        .unwrap();
                }
            };
            let answer = transcript
                .health(&fixture.keys[0], &dialer, Kind::Pong, &nonce)
                .unwrap();
            owner.health(source, Kind::Pong, &answer).await.unwrap();
            assert_eq!(owner.channels[&source].fact.id, id, "move {step} followed");
            assert_eq!(live(&sources), vec![source], "move {step}: one grant");
            sockets.push(moved);
        }
        owner.shutdown().await.unwrap();
        assert!(
            live(&sources).is_empty(),
            "the channel's end revokes its grant"
        );
        fixture.close().await;
    }

    /// F6a B4: this Node's own path change (a roam, or a STUN remap) starts
    /// every channel's size search again and withdraws the old measurement
    /// at once: the new path may be smaller while the route MTU stays.
    #[tokio::test]
    async fn an_own_path_change_searches_the_path_size_again() {
        let Roaming {
            fixture,
            shared,
            mut owner,
            transcript,
            dialer,
            peer,
            ..
        } = roaming().await;
        let ids = fixture.ids();
        let remote = peer.local_addr().unwrap();
        let id = owner.channels[&remote].fact.id;
        let mut bytes = vec![0; 65_536];
        let probe = async |owner: &mut Owner, bytes: &mut Vec<u8>| {
            owner.tick().await.unwrap();
            loop {
                let (len, _) = control(&peer, bytes).await;
                let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
                if kind == Kind::Probe {
                    break (
                        len,
                        transcript.verify_probe(&dialer, ids[1], len, body).unwrap(),
                    );
                }
            }
        };
        let (len, nonce) = probe(&mut owner, &mut bytes).await;
        assert_eq!(len, 1500 - 28, "the route MTU first");
        let pong = transcript
            .health(&fixture.keys[0], &dialer, Kind::Pong, &nonce)
            .unwrap();
        owner.health(remote, Kind::Pong, &pong).await.unwrap();
        assert_eq!(
            shared
                .path_mtu
                .lock()
                .unwrap()
                .get(&id)
                .map(|known| known.1),
            Some(1500)
        );
        owner.roamed();
        assert!(
            !shared.path_mtu.lock().unwrap().contains_key(&id),
            "the old path's size is withdrawn"
        );
        let (len, _) = probe(&mut owner, &mut bytes).await;
        assert_eq!(
            len,
            1500 - 28,
            "searched again now, not after the raise timer"
        );
        owner.shutdown().await.unwrap();
        fixture.close().await;
    }
}
