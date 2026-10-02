mod connectivity;
mod discovery;
pub use connectivity::{UdpPunchObservation, UdpPunchSocket};
pub(crate) mod generation;
pub use discovery::{UdpDiscoveryObservation, UdpDiscoveryOutcome, UdpSocketInfo};
mod neighbor;
mod packet;
mod probe;
mod recovery;
pub mod sockets;
mod statistics;
mod stream;
#[cfg(test)]
mod tests;
mod udp;
mod wire;
mod work_budget;

use crate::{
    Batch, DuplexStream, Error, Identity, LabConfig, Result, TransportKind,
    config::{public_key, valid_id, validate_context_id},
};
use statistics::{Counters, DropReason, Failure, HighWater, StageResult, occupancy};
pub use statistics::{
    DropStatistics, QueueRejectionStatistics, QueueStatistics, RuntimeQueueStatistics, Statistics,
    TransportStatistics,
};
use std::{
    collections::BTreeMap,
    fmt,
    net::SocketAddr,
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};
use tokio::{
    sync::{Notify, Semaphore, mpsc, watch},
    task::JoinHandle,
    time::Instant,
};
use wire::{Packet, Phase};

pub const MAX_PACKET_PAYLOAD: usize = wire::MAX_PAYLOAD;
pub const MAX_NATIVE_UDP_DATAGRAM: usize = 9_000;

/// Exact UDP payload length for an established endpoint packet, excluding outer IP/UDP.
pub fn native_udp_datagram_len(path: &[String], payload_len: usize) -> Result<usize> {
    if !(2..=8).contains(&path.len()) || path.iter().any(|id| !valid_id(id)) {
        return Err(Error::UnauthorizedPath);
    }
    if payload_len > MAX_PACKET_PAYLOAD {
        return Err(Error::Oversize);
    }
    Ok(33 + 24 + 1 + 19 + path.iter().map(|id| 1 + id.len()).sum::<usize>() + 24 + payload_len)
}

/// Endpoint-authenticated payload. Only the final node receives plaintext.
pub struct ReceivedPacket {
    pub source: String,
    pub path: Vec<String>,
    pub payload: Vec<u8>,
}

impl fmt::Debug for ReceivedPacket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReceivedPacket")
            .field("source", &self.source)
            .field("path", &self.path)
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}

#[derive(Clone, Debug)]
pub enum StreamRole {
    Dialer { peer: String },
    Acceptor,
}

/// Trusted local adapter metadata and listener-scoped admission, never peer input.
#[derive(Clone, Debug, Default)]
pub struct StreamContext {
    pub listener_id: Option<String>,
    pub candidate_id: Option<String>,
    pub allowed_peers: Option<Vec<String>>,
    pub remote_addr: Option<SocketAddr>,
}

impl StreamContext {
    fn validate(
        &self,
        config: &LabConfig,
        role: &StreamRole,
        transport: TransportKind,
    ) -> Result<()> {
        validate_context_id(self.listener_id.as_deref())?;
        validate_context_id(self.candidate_id.as_deref())?;
        match role {
            StreamRole::Dialer { .. }
                if self.listener_id.is_some() || self.allowed_peers.is_some() =>
            {
                return Err(Error::InvalidConfig("dialer has listener context"));
            }
            StreamRole::Acceptor if self.candidate_id.is_some() => {
                return Err(Error::InvalidConfig("acceptor has candidate context"));
            }
            _ => (),
        }
        config.validate_listener_peers(self.allowed_peers.as_deref(), transport)
    }

    fn authorize(&self, peer: &str) -> Result<()> {
        if self
            .allowed_peers
            .as_ref()
            .is_some_and(|peers| !peers.iter().any(|id| id == peer))
        {
            return Err(Error::UnauthorizedTransport);
        }
        Ok(())
    }
}

/// A freshly key-confirmed selected adjacency; not a delivery/health guarantee.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PeerConnection {
    pub peer_node_id: String,
    pub transport: TransportKind,
    pub initiator: bool,
    pub listener_id: Option<String>,
    pub candidate_id: Option<String>,
    pub remote_addr: Option<SocketAddr>,
}

/// Point-in-time proof of a round trip on precisely this confirmed connection.
/// The connection may change after completion; no future liveness is implied.
#[derive(Clone, Debug, serde::Serialize)]
pub struct NeighborProbeObservation {
    pub connection: PeerConnection,
    pub round_trip: Duration,
}

#[derive(Clone, Debug)]
pub struct BoundListener {
    pub transport: TransportKind,
    pub address: SocketAddr,
    pub id: Option<String>,
}

pub struct ProbeReply {
    pub payload: Vec<u8>,
    pub path: Vec<String>,
    pub elapsed: Duration,
}

impl fmt::Debug for ProbeReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProbeReply")
            .field("payload_bytes", &self.payload.len())
            .field("path", &self.path)
            .field("elapsed", &self.elapsed)
            .finish()
    }
}

struct Outbound {
    token: [u8; 32],
    batch: Batch,
    generation: Option<Arc<generation::Fence>>,
}

#[derive(Clone)]
struct Channel {
    token: [u8; 32],
    priority: u16,
    transport: TransportKind,
    tx: mpsc::Sender<Outbound>,
    close: watch::Sender<bool>,
    initiator: bool,
    // Channel clones are also used by packet enqueue; keep scoped metadata shared.
    context: Arc<StreamContext>,
    probe_budget: Arc<Mutex<neighbor::ProbeBudget>>,
}

/// Immutable, locally captured association of public metadata with a private token.
/// Never serialized or included in public Debug output.
#[derive(Clone)]
struct InternalChannelSnapshot {
    token: [u8; 32],
    connection: PeerConnection,
}

impl Channel {
    fn snapshot(&self, peer: &str) -> InternalChannelSnapshot {
        InternalChannelSnapshot {
            token: self.token,
            connection: PeerConnection {
                peer_node_id: peer.into(),
                transport: self.transport,
                initiator: self.initiator,
                listener_id: self.context.listener_id.clone(),
                candidate_id: self.context.candidate_id.clone(),
                remote_addr: self.context.remote_addr,
            },
        }
    }
}

struct Incoming {
    /// Originating native carrier owner, independent of packet admission.
    socket_generation: Option<Arc<generation::Fence>>,
    peer: String,
    token: [u8; 32],
    transport: TransportKind,
    body: Vec<u8>,
    generation: Arc<generation::Fence>,
}
struct WaitingProbe {
    path: Vec<String>,
    tx: mpsc::Sender<Packet>,
}
struct EndpointSession {
    path: Vec<String>,
    cipher: crate::crypto::Cipher,
    expires: Instant,
    packets: bool,
    confirmed: bool,
    retry: Option<(Vec<u8>, Vec<u8>)>,
}
struct SendingSession {
    id: [u8; 16],
    cipher: crate::crypto::Cipher,
    expires: Instant,
}

struct Inner {
    config: LabConfig,
    identity: Identity,
    keys: BTreeMap<String, [u8; 32]>,
    peers: Mutex<BTreeMap<String, Channel>>,
    pending: Mutex<BTreeMap<[u8; 16], WaitingProbe>>,
    neighbor_pending: Mutex<BTreeMap<[u8; 16], neighbor::Waiting>>,
    udp_sockets: Mutex<BTreeMap<u32, discovery::Registration>>,
    udp_discovery_slots: Arc<Semaphore>,
    udp_punch_slots: Arc<Semaphore>,
    endpoints: Mutex<BTreeMap<(String, [u8; 16]), EndpointSession>>,
    // Immutable, configuration-bounded permits (at most 128 exact paths). Setup
    // waits never hold the shared cache/nonce lock or allocate per caller.
    packet_setup: BTreeMap<Vec<String>, tokio::sync::Mutex<()>>,
    packet_generation: Mutex<Arc<generation::Fence>>,
    packet_retries: Mutex<BTreeMap<Vec<String>, packet::Setup>>,
    recovery: Mutex<recovery::Control>,
    sending: tokio::sync::Mutex<BTreeMap<Vec<String>, SendingSession>>,
    delivery: Mutex<Option<mpsc::Sender<ReceivedPacket>>>,
    handles: Mutex<Vec<JoinHandle<()>>>,
    semaphore: Arc<Semaphore>,
    incoming: mpsc::Sender<Incoming>,
    shutdown: watch::Sender<bool>,
    changed: Notify,
    counters: [Counters; 3],
    ingress_high_water: HighWater,
    delivery_high_water: HighWater,
    #[cfg(test)]
    observed_transit: Mutex<Vec<Vec<u8>>>,
    #[cfg(test)]
    blocking_dispatch_test: std::sync::atomic::AtomicBool,
}

struct Handle {
    native: tokio::sync::Mutex<NativeRunning>,
    inner: Arc<Inner>,
    bound: Vec<BoundListener>,
    receiver: Mutex<Option<mpsc::Receiver<ReceivedPacket>>>,
}

struct NativeRunning {
    generation: sockets::SocketGeneration,
    tasks: Vec<JoinHandle<()>>,
    factory: Arc<dyn sockets::NativeSocketFactory>,
    next_socket_id: u32,
    punch_sockets: usize,
}
impl NativeRunning {
    async fn stop(&mut self) {
        self.generation.retire();
        for task in self.tasks.drain(..) {
            let _ = task.await;
        }
    }
}

type PreparedTcp = (Box<dyn sockets::NativeListener>, StreamContext);
type PreparedUdp = (
    Box<dyn sockets::NativeUdp>,
    Option<crate::OutgoingCandidate>,
    StreamContext,
);

fn prepare_native(
    config: &LabConfig,
    factory: &dyn sockets::NativeSocketFactory,
    bound: Option<&[BoundListener]>,
) -> Result<(Vec<PreparedTcp>, Vec<PreparedUdp>, Vec<BoundListener>)> {
    factory.check_active()?;
    let mut tcp = Vec::new();
    let mut udp = Vec::new();
    let mut addresses = Vec::new();
    for (index, listener) in config.listeners.iter().enumerate() {
        let bind = bound.map_or(listener.bind, |b| b[index].address);
        let purpose = sockets::NativeSocketPurpose::Listener {
            id: listener.id.as_deref(),
        };
        let context = StreamContext {
            listener_id: listener.id.clone(),
            allowed_peers: listener.allowed_peers.clone(),
            ..Default::default()
        };
        let address = match listener.transport {
            TransportKind::NativeTcp => {
                let s = factory.listen(bind, purpose)?;
                let a = s.local_addr()?;
                tcp.push((s, context));
                a
            }
            TransportKind::NativeUdp => {
                let s = factory.udp(bind, config.limits.udp_receive_buffer_bytes, purpose)?;
                let a = s.local_addr()?;
                udp.push((s, None, context));
                a
            }
            TransportKind::Vless => {
                return Err(Error::InvalidConfig("external VLESS listener required"));
            }
        };
        addresses.push(BoundListener {
            transport: listener.transport,
            address,
            id: listener.id.clone(),
        });
    }
    for candidate in &config.outgoing {
        if candidate.transport != TransportKind::NativeUdp {
            continue;
        }
        let remote = candidate
            .endpoint
            .parse::<SocketAddr>()
            .map_err(|_| Error::Protocol)?;
        let bind = if remote.is_ipv4() {
            SocketAddr::from(([0, 0, 0, 0], 0))
        } else {
            SocketAddr::from(([0_u16; 8], 0))
        };
        let socket = factory.udp(
            bind,
            config.limits.udp_receive_buffer_bytes,
            sockets::NativeSocketPurpose::Dial {
                id: candidate.id.as_deref(),
                peer: &candidate.peer,
            },
        )?;
        udp.push((
            socket,
            Some(candidate.clone()),
            StreamContext {
                candidate_id: candidate.id.clone(),
                ..Default::default()
            },
        ));
    }
    Ok((tcp, udp, addresses))
}

fn launch_native(
    inner: &Arc<Inner>,
    generation: sockets::SocketGeneration,
    factory: Arc<dyn sockets::NativeSocketFactory>,
    tcp: Vec<PreparedTcp>,
    udp: Vec<PreparedUdp>,
) -> Result<NativeRunning> {
    let mut running = NativeRunning {
        generation,
        tasks: Vec::new(),
        factory: factory.clone(),
        next_socket_id: u32::try_from(udp.len()).map_err(|_| Error::Capacity)?,
        punch_sockets: 0,
    };
    let mut spawn = |future: std::pin::Pin<Box<dyn Future<Output = ()> + Send>>| {
        let fence = running.generation.0.clone();
        running.tasks.push(tokio::spawn(async move {
            tokio::select! { biased; _ = fence.cancelled() => (), _ = future => () }
        }));
    };
    for (listener, context) in tcp {
        spawn(Box::pin(stream::listen(
            inner.clone(),
            listener,
            context,
            running.generation.0.clone(),
        )));
    }
    for (ordinal, (socket, candidate, context)) in udp.into_iter().enumerate() {
        let socket_id = u32::try_from(ordinal).map_err(|_| Error::Capacity)?;
        let info = UdpSocketInfo {
            socket_id,
            local_addr: socket.local_addr()?,
            listener_id: context.listener_id.clone(),
            candidate_id: context.candidate_id.clone(),
        };
        let (tx, rx) = mpsc::channel(1);
        lock(&inner.udp_sockets).insert(
            socket_id,
            discovery::Registration {
                info,
                tx,
                slot: Arc::new(Semaphore::new(1)),
                generation: running.generation.0.clone(),
                punch: None,
            },
        );
        let live = discovery::LiveSocket {
            inner: inner.clone(),
            id: socket_id,
            rx,
            punch_rx: None,
        };
        spawn(Box::pin(udp::run(
            inner.clone(),
            socket,
            candidate,
            context,
            live,
            running.generation.0.clone(),
            None,
        )));
    }
    for candidate in inner.config.outgoing.clone() {
        if candidate.transport == TransportKind::NativeTcp {
            spawn(Box::pin(stream::maintain(
                inner.clone(),
                candidate,
                factory.clone(),
                running.generation.0.clone(),
            )));
        }
    }
    Ok(running)
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.inner.shutdown.send_replace(true);
        self.inner.changed.notify_waiters();
    }
}

#[derive(Clone)]
pub struct Runtime {
    handle: Arc<Handle>,
}

// Poisoning means a library programming fault, not invalid network input. Recover locks
// so shutdown remains usable; fallible network/crypto code never panics while locked.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Runtime {
    pub async fn start(config: LabConfig) -> Result<Self> {
        Self::start_with_socket_factory(config, Arc::new(sockets::SystemSocketFactory)).await
    }

    pub async fn start_with_socket_factory(
        config: LabConfig,
        sockets: Arc<dyn sockets::NativeSocketFactory>,
    ) -> Result<Self> {
        config.validate()?;
        let identity = Identity::from_private_key(config.private_key.clone());
        let keys = config
            .trusted_peers
            .iter()
            .map(|peer| Ok((peer.node_id.clone(), public_key(&peer.public_key)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let (incoming, receiver) = mpsc::channel(config.limits.queue_capacity);
        let (delivery, delivery_receiver) = mpsc::channel(config.limits.queue_capacity);
        let (shutdown, _) = watch::channel(false);
        let inner = Arc::new(Inner {
            semaphore: Arc::new(Semaphore::new(config.limits.max_connections)),
            packet_setup: config
                .allowed_paths
                .iter()
                .filter(|path| path.first() == Some(&config.node_id))
                .map(|path| (path.clone(), tokio::sync::Mutex::new(())))
                .collect(),
            recovery: Mutex::new(recovery::Control::new(&config.allowed_paths)),
            config,
            identity,
            keys,
            peers: Mutex::default(),
            pending: Mutex::default(),
            neighbor_pending: Mutex::default(),
            udp_sockets: Mutex::default(),
            udp_discovery_slots: Arc::new(Semaphore::new(16)),
            udp_punch_slots: Arc::new(Semaphore::new(16)),
            endpoints: Mutex::default(),
            sending: tokio::sync::Mutex::default(),
            packet_generation: Mutex::new(generation::Fence::new()),
            packet_retries: Mutex::default(),
            delivery: Mutex::new(Some(delivery)),
            handles: Mutex::default(),
            incoming,
            shutdown,
            changed: Notify::new(),
            counters: Default::default(),
            ingress_high_water: HighWater::default(),
            delivery_high_water: HighWater::default(),
            #[cfg(test)]
            observed_transit: Mutex::default(),
            #[cfg(test)]
            blocking_dispatch_test: std::sync::atomic::AtomicBool::new(false),
        });
        // Atomic setup before any engine is spawned. A separate socket owner
        // fences native I/O even for the compatible unmarked implementation.
        let generation = sockets::SocketGeneration::new();
        let sockets = generation.factory(sockets);
        let (tcp, udp, bound) = prepare_native(&inner.config, sockets.as_ref(), None)?;
        let native = launch_native(&inner, generation, sockets, tcp, udp)?;
        let runtime = Self {
            handle: Arc::new(Handle {
                native: tokio::sync::Mutex::new(native),
                inner: inner.clone(),
                bound,
                receiver: Mutex::new(Some(delivery_receiver)),
            }),
        };
        inner.spawn(probe::dispatch(inner.clone(), receiver));
        Ok(runtime)
    }

    /// Rebind native I/O in the same Runtime, retaining E2E identity, policy and
    /// the independent packet fence. Caller prepares the new platform capability
    /// first. Failure leaves native I/O retired; never retries with system sockets.
    /// Existing listeners retain their initially assigned port (including :0).
    pub async fn replace_native_socket_factory(
        &self,
        factory: Arc<dyn sockets::NativeSocketFactory>,
    ) -> Result<()> {
        let mut native = self.handle.native.lock().await;
        native.stop().await;
        let inner = &self.handle.inner;
        {
            let mut peers = lock(&inner.peers);
            peers.retain(|_, channel| {
                if channel.transport == TransportKind::Vless {
                    return true;
                }
                channel.close.send_replace(true);
                false
            });
        }
        inner.changed.notify_waiters();
        if *inner.shutdown.borrow() {
            return Err(Error::Closed);
        }
        let generation = sockets::SocketGeneration::new();
        let factory = generation.factory(factory);
        let (tcp, udp, _) =
            prepare_native(&inner.config, factory.as_ref(), Some(&self.handle.bound))?;
        *native = launch_native(inner, generation, factory, tcp, udp)?;
        Ok(())
    }

    pub fn node_id(&self) -> &str {
        &self.handle.inner.config.node_id
    }
    pub fn bound_listeners(&self) -> &[BoundListener] {
        &self.handle.bound
    }

    pub async fn attach_stream<S: DuplexStream + 'static>(
        &self,
        stream: S,
        role: StreamRole,
        transport: TransportKind,
    ) -> Result<()> {
        self.attach_stream_with_priority(stream, role, transport, 0)
            .await
    }

    pub async fn attach_stream_with_priority<S: DuplexStream + 'static>(
        &self,
        stream: S,
        role: StreamRole,
        transport: TransportKind,
        priority: u16,
    ) -> Result<()> {
        self.attach_stream_with_context(stream, role, transport, priority, StreamContext::default())
            .await
    }

    pub async fn attach_stream_with_context<S: DuplexStream + 'static>(
        &self,
        stream: S,
        role: StreamRole,
        transport: TransportKind,
        priority: u16,
        context: StreamContext,
    ) -> Result<()> {
        if transport == TransportKind::NativeUdp {
            return Err(Error::Protocol);
        }
        let inner = &self.handle.inner;
        context.validate(&inner.config, &role, transport)?;
        if *inner.shutdown.borrow() {
            return Err(Error::Closed);
        }
        let permit = inner
            .semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)?;
        let established =
            stream::authenticate_with_context(inner, stream, role, transport, priority, context)
                .await?;
        stream::install(inner.clone(), established, transport, permit)
    }

    pub async fn wait_for_peer(&self, peer: &str, deadline: Duration) -> Result<()> {
        let inner = &self.handle.inner;
        tokio::time::timeout(deadline, async {
            loop {
                let notified = inner.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if *inner.shutdown.borrow() {
                    return Err(Error::Closed);
                }
                if lock(&inner.peers).contains_key(peer) {
                    return Ok(());
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    pub fn connected_peers(&self) -> Vec<String> {
        lock(&self.handle.inner.peers).keys().cloned().collect()
    }

    pub fn peer_transport(&self, peer: &str) -> Option<TransportKind> {
        lock(&self.handle.inner.peers)
            .get(peer)
            .map(|channel| channel.transport)
    }

    pub fn peer_connections(&self) -> Vec<PeerConnection> {
        lock(&self.handle.inner.peers)
            .iter()
            .map(|(peer, channel)| channel.snapshot(peer).connection)
            .collect()
    }

    /// Round trip on the exact confirmed neighbor channel; no path/transit grant.
    /// Timeout disconnects only that channel, never a concurrently installed replacement.
    /// At most three requests share the original deadline. The returned duration
    /// starts at the first request, including queueing and any loss/retry delay.
    pub async fn probe_neighbor(&self, peer: &str, timeout: Duration) -> Result<Duration> {
        Ok(self
            .probe_neighbor_observed(peer, timeout)
            .await?
            .round_trip)
    }

    /// Return proof and locally captured metadata for the same confirmed channel.
    /// Replacement during the call returns Unavailable. This is a point-in-time
    /// observation: the selected connection may change after completion.
    /// Uses the same bounded retries and first-request elapsed time as `probe_neighbor`;
    /// this is not a loss-excluding latency sample. Final timeout still retires the channel.
    pub async fn probe_neighbor_observed(
        &self,
        peer: &str,
        timeout: Duration,
    ) -> Result<NeighborProbeObservation> {
        neighbor::probe_observed(self.handle.inner.clone(), peer, timeout).await
    }

    pub fn statistics(&self) -> Statistics {
        let inner = &self.handle.inner;
        let capacity = inner.config.limits.queue_capacity;
        let mut outbound = [0; 3];
        for channel in lock(&inner.peers).values() {
            let current = &mut outbound[Inner::transport_index(channel.transport)];
            *current = (*current).max(occupancy(&channel.tx));
        }
        let delivery = lock(&inner.delivery).as_ref().map_or(0, occupancy);
        Statistics {
            native_udp: inner.counters[0].snapshot(capacity, outbound[0]),
            native_tcp: inner.counters[1].snapshot(capacity, outbound[1]),
            vless: inner.counters[2].snapshot(capacity, outbound[2]),
            queues: RuntimeQueueStatistics {
                ingress: inner
                    .ingress_high_water
                    .snapshot(capacity, occupancy(&inner.incoming)),
                endpoint_delivery: inner.delivery_high_water.snapshot(capacity, delivery),
            },
        }
    }

    pub async fn probe(&self, path: Vec<String>, payload: Vec<u8>) -> Result<ProbeReply> {
        probe::probe(self.handle.inner.clone(), path, payload).await
    }

    /// Take the sole bounded plaintext endpoint receiver. Probe control is never delivered here.
    pub fn take_packet_receiver(&self) -> Result<mpsc::Receiver<ReceivedPacket>> {
        lock(&self.handle.receiver).take().ok_or(Error::Unavailable)
    }

    /// The packet pump returns its unique receiver after a fenced admission
    /// generation. A stopped Runtime never revives delivery or cached sessions.
    pub(crate) fn return_packet_receiver(&self, receiver: mpsc::Receiver<ReceivedPacket>) {
        let mut slot = lock(&self.handle.receiver);
        if !*self.handle.inner.shutdown.borrow() && slot.is_none() {
            *slot = Some(receiver);
        }
    }

    // Call only after acquiring the unique receiver. The old fence stays
    // installed while its delivery queue is drained, so dispatch cannot refill
    // it with old ingress work between draining and activation.
    pub(crate) fn retire_packet_generation(&self) {
        self.handle.inner.packet_generation().retire();
    }

    pub(crate) fn begin_packet_generation(&self) -> generation::Generation {
        let fence = generation::Fence::new();
        *lock(&self.handle.inner.packet_generation) = fence.clone();
        generation::Generation(fence)
    }

    /// Encrypt and enqueue an owned endpoint packet. Success is not a remote delivery ACK.
    pub async fn send_packet(&self, path: Vec<String>, payload: Vec<u8>) -> Result<()> {
        packet::send(self.handle.inner.clone(), path, vec![payload]).await
    }

    /// Bounded owned packet batch; cached sessions and one channel queue entry per batch.
    pub async fn send_packet_batch(&self, path: Vec<String>, payloads: Vec<Vec<u8>>) -> Result<()> {
        packet::send(self.handle.inner.clone(), path, payloads).await
    }

    pub async fn shutdown(&self) {
        let inner = &self.handle.inner;
        inner.shutdown.send_replace(true);
        inner.packet_generation().retire();
        inner.changed.notify_waiters();
        self.handle.native.lock().await.stop().await;
        lock(&inner.delivery).take();
        lock(&self.handle.receiver).take();
        let handles = std::mem::take(&mut *lock(&inner.handles));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        for mut handle in handles {
            if tokio::time::timeout_at(deadline, &mut handle)
                .await
                .is_err()
            {
                handle.abort();
                let _ = handle.await;
            }
        }
        lock(&inner.peers).clear();
        lock(&inner.pending).clear();
        lock(&inner.neighbor_pending).clear();
        lock(&inner.endpoints).clear();
        lock(&inner.packet_retries).clear();
        lock(&inner.recovery).clear();
        if let Ok(mut sessions) = tokio::time::timeout_at(deadline, inner.sending.lock()).await {
            sessions.clear();
        }
    }
}

impl Inner {
    fn packet_generation(&self) -> Arc<generation::Fence> {
        lock(&self.packet_generation).clone()
    }
    fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        let mut handles = lock(&self.handles);
        if *self.shutdown.borrow() {
            return;
        }
        handles.retain(|handle| !handle.is_finished());
        handles.push(tokio::spawn(future));
    }
    fn counters(&self, transport: TransportKind) -> &Counters {
        &self.counters[Self::transport_index(transport)]
    }
    fn transport_index(transport: TransportKind) -> usize {
        match transport {
            TransportKind::NativeUdp => 0,
            TransportKind::NativeTcp => 1,
            TransportKind::Vless => 2,
        }
    }
    fn identify(&self, key: Option<&[u8]>) -> Result<String> {
        let key = key.ok_or(Error::Authentication)?;
        self.keys
            .iter()
            .find_map(|(id, expected)| (key == expected).then(|| id.clone()))
            .ok_or(Error::Authentication)
    }
    fn register(&self, peer: String, channel: Channel) -> Result<()> {
        self.authorize_transport(&peer, channel.transport)?;
        channel.context.authorize(&peer)?;
        let mut peers = lock(&self.peers);
        if *self.shutdown.borrow() {
            return Err(Error::Closed);
        }
        if let Some(previous) = peers.get(&peer) {
            if (previous.priority, previous.token) <= (channel.priority, channel.token) {
                return Err(Error::Capacity);
            }
            previous.close.send_replace(true);
        }
        peers.insert(peer, channel);
        self.changed.notify_waiters();
        Ok(())
    }
    fn authorize_transport(&self, peer: &str, transport: TransportKind) -> Result<()> {
        if !self
            .config
            .trusted_peers
            .iter()
            .any(|allowed| allowed.node_id == peer && allowed.transports.contains(&transport))
        {
            return Err(Error::UnauthorizedTransport);
        }
        Ok(())
    }
    fn unregister(&self, peer: &str, token: &[u8; 32]) {
        let mut peers = lock(&self.peers);
        if peers
            .get(peer)
            .is_some_and(|channel| &channel.token == token)
            && let Some(channel) = peers.remove(peer)
        {
            channel.close.send_replace(true);
        }
        self.changed.notify_waiters();
    }
    fn active(&self, peer: &str, token: &[u8; 32]) -> bool {
        lock(&self.peers)
            .get(peer)
            .is_some_and(|channel| &channel.token == token)
    }
    fn authorize(&self, path: &[String]) -> Result<usize> {
        let permitted = self
            .config
            .allowed_paths
            .iter()
            .any(|allowed| allowed == path);
        if !permitted {
            return Err(Error::UnauthorizedPath);
        }
        path.iter()
            .position(|id| id == &self.config.node_id)
            .ok_or(Error::UnauthorizedPath)
    }
    fn send(&self, peer: &str, packet: Packet) -> Result<()> {
        self.send_classified(peer, packet)
            .map_err(|failure| self.local_send_failure(failure))
    }
    fn local_send_failure(&self, failure: Failure) -> Error {
        if let Some(transport) = failure.outgoing {
            self.counters(transport).drop(failure.reason);
        }
        failure.error
    }
    // Dispatch owns ingress attribution; public/local send owns outgoing attribution.
    fn send_classified(&self, peer: &str, packet: Packet) -> StageResult<()> {
        let channel = lock(&self.peers)
            .get(peer)
            .cloned()
            .ok_or(Error::Unavailable)?;
        let generation = (packet.phase == Phase::Packet || packet.phase.recovery())
            .then(|| self.packet_generation());
        let batch = Batch::new(vec![packet.encode()?])?;
        self.enqueue_scoped(&channel, batch, generation)
    }
    fn enqueue(&self, channel: &Channel, batch: Batch) -> StageResult<()> {
        self.enqueue_scoped(channel, batch, None)
    }
    fn enqueue_scoped(
        &self,
        channel: &Channel,
        batch: Batch,
        generation: Option<Arc<generation::Fence>>,
    ) -> StageResult<()> {
        let counters = self.counters(channel.transport);
        match channel.tx.try_send(Outbound {
            token: channel.token,
            batch,
            generation,
        }) {
            Ok(()) => {
                counters.outbound_high_water.enqueued(&channel.tx);
                Ok(())
            }
            Err(error) => {
                let (error, reason, full) = match error {
                    mpsc::error::TrySendError::Full(_) => {
                        counters
                            .outbound_high_water
                            .observe(channel.tx.max_capacity());
                        (Error::Capacity, DropReason::OutboundQueueFull, true)
                    }
                    mpsc::error::TrySendError::Closed(_) => {
                        (Error::Unavailable, DropReason::OutboundQueueClosed, false)
                    }
                };
                counters.outbound_rejected(full);
                Err(Failure {
                    error,
                    reason,
                    outgoing: Some(channel.transport),
                })
            }
        }
    }
    fn deliver(&self, incoming: Incoming) {
        let counters = self.counters(incoming.transport);
        if incoming.body.first() != Some(&neighbor::NAMESPACE) {
            counters.received.fetch_add(1, Ordering::Relaxed);
        }
        match self.incoming.try_send(incoming) {
            Ok(()) => self.ingress_high_water.enqueued(&self.incoming),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.ingress_high_water
                    .observe(self.incoming.max_capacity());
                counters.drop(DropReason::IngressQueueFull);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                counters.drop(DropReason::IngressQueueClosed)
            }
        }
    }
}
