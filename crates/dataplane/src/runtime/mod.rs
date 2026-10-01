mod discovery;
pub use discovery::{UdpDiscoveryObservation, UdpDiscoveryOutcome, UdpSocketInfo};
mod neighbor;
mod packet;
mod probe;
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
    net::TcpListener,
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
    peer: String,
    token: [u8; 32],
    transport: TransportKind,
    body: Vec<u8>,
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
    endpoints: Mutex<BTreeMap<(String, [u8; 16]), EndpointSession>>,
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
}

struct Handle {
    inner: Arc<Inner>,
    bound: Vec<BoundListener>,
    receiver: Mutex<Option<mpsc::Receiver<ReceivedPacket>>>,
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
            config,
            identity,
            keys,
            peers: Mutex::default(),
            pending: Mutex::default(),
            neighbor_pending: Mutex::default(),
            udp_sockets: Mutex::default(),
            udp_discovery_slots: Arc::new(Semaphore::new(16)),
            endpoints: Mutex::default(),
            sending: tokio::sync::Mutex::default(),
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
        });
        // Prepare native listeners and UDP candidates before spawning any tasks, so
        // socket-option/bind failure is an explicit, atomic startup error.
        let mut tcp = Vec::new();
        let mut udp = Vec::new();
        let mut bound = Vec::new();
        for listener in &inner.config.listeners {
            let context = StreamContext {
                listener_id: listener.id.clone(),
                allowed_peers: listener.allowed_peers.clone(),
                ..Default::default()
            };
            let address = match listener.transport {
                TransportKind::NativeTcp => {
                    let socket = TcpListener::bind(listener.bind).await?;
                    let address = socket.local_addr()?;
                    tcp.push((socket, context));
                    address
                }
                TransportKind::NativeUdp => {
                    let socket =
                        udp::bind(listener.bind, inner.config.limits.udp_receive_buffer_bytes)?;
                    let address = socket.local_addr()?;
                    udp.push((socket, None, context));
                    address
                }
                TransportKind::Vless => {
                    return Err(Error::InvalidConfig("external VLESS listener required"));
                }
            };
            bound.push(BoundListener {
                transport: listener.transport,
                address,
                id: listener.id.clone(),
            });
        }
        for candidate in &inner.config.outgoing {
            if candidate.transport == TransportKind::NativeUdp {
                let socket =
                    udp::bind_outgoing(candidate, inner.config.limits.udp_receive_buffer_bytes)?;
                udp.push((
                    socket,
                    Some(candidate.clone()),
                    StreamContext {
                        candidate_id: candidate.id.clone(),
                        ..Default::default()
                    },
                ));
            }
        }
        let runtime = Self {
            handle: Arc::new(Handle {
                inner: inner.clone(),
                bound,
                receiver: Mutex::new(Some(delivery_receiver)),
            }),
        };
        inner.spawn(probe::dispatch(inner.clone(), receiver));
        for (listener, context) in tcp {
            inner.spawn(stream::listen(inner.clone(), listener, context));
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
                },
            );
            let live = discovery::LiveSocket {
                inner: inner.clone(),
                id: socket_id,
                rx,
            };
            inner.spawn(udp::run(inner.clone(), socket, candidate, context, live));
        }
        for candidate in inner.config.outgoing.clone() {
            match candidate.transport {
                TransportKind::NativeTcp => inner.spawn(stream::maintain(inner.clone(), candidate)),
                TransportKind::NativeUdp => (), // Prepared above; reconnect reuses this socket.
                TransportKind::Vless => (),     // Established by CLI adapter, then attach_stream.
            }
        }
        Ok(runtime)
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
    pub async fn probe_neighbor(&self, peer: &str, timeout: Duration) -> Result<Duration> {
        Ok(self
            .probe_neighbor_observed(peer, timeout)
            .await?
            .round_trip)
    }

    /// Return proof and locally captured metadata for the same confirmed channel.
    /// Replacement during the call returns Unavailable. This is a point-in-time
    /// observation: the selected connection may change after completion.
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
        inner.changed.notify_waiters();
        lock(&inner.delivery).take();
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
        if let Ok(mut sessions) = tokio::time::timeout_at(deadline, inner.sending.lock()).await {
            sessions.clear();
        }
    }
}

impl Inner {
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
        let batch = Batch::new(vec![packet.encode()?])?;
        self.enqueue(&channel, batch)
    }
    fn enqueue(&self, channel: &Channel, batch: Batch) -> StageResult<()> {
        let counters = self.counters(channel.transport);
        match channel.tx.try_send(Outbound {
            token: channel.token,
            batch,
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
