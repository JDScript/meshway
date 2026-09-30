mod packet;
mod probe;
mod stream;
#[cfg(test)]
mod tests;
mod udp;
mod wire;

use crate::{
    Batch, DuplexStream, Error, Identity, LabConfig, Result, TransportKind, config::public_key,
};
use std::{
    collections::BTreeMap,
    fmt,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::{TcpListener, UdpSocket},
    sync::{Notify, Semaphore, mpsc, watch},
    task::JoinHandle,
    time::Instant,
};
use wire::{Packet, Phase};

pub const MAX_PACKET_PAYLOAD: usize = wire::MAX_PAYLOAD;
pub const MAX_NATIVE_UDP_DATAGRAM: usize = 9_000;

/// Exact UDP payload length for an established endpoint packet, excluding outer IP/UDP.
pub fn native_udp_datagram_len(path: &[String], payload_len: usize) -> Result<usize> {
    if !(2..=8).contains(&path.len()) || path.iter().any(|id| id.is_empty() || id.len() > 48) {
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

#[derive(Clone, Debug)]
pub struct BoundListener {
    pub transport: TransportKind,
    pub address: SocketAddr,
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

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct TransportStatistics {
    pub received: u64,
    pub sent: u64,
    pub forwarded: u64,
    pub dropped: u64,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Statistics {
    pub native_udp: TransportStatistics,
    pub native_tcp: TransportStatistics,
    pub vless: TransportStatistics,
}

#[derive(Default)]
struct Counters {
    received: AtomicU64,
    sent: AtomicU64,
    forwarded: AtomicU64,
    dropped: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> TransportStatistics {
        TransportStatistics {
            received: self.received.load(Ordering::Relaxed),
            sent: self.sent.load(Ordering::Relaxed),
            forwarded: self.forwarded.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
        }
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
    endpoints: Mutex<BTreeMap<(String, [u8; 16]), EndpointSession>>,
    sending: tokio::sync::Mutex<BTreeMap<Vec<String>, SendingSession>>,
    delivery: Mutex<Option<mpsc::Sender<ReceivedPacket>>>,
    handles: Mutex<Vec<JoinHandle<()>>>,
    semaphore: Arc<Semaphore>,
    incoming: mpsc::Sender<Incoming>,
    shutdown: watch::Sender<bool>,
    changed: Notify,
    counters: [Counters; 3],
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
            endpoints: Mutex::default(),
            sending: tokio::sync::Mutex::default(),
            delivery: Mutex::new(Some(delivery)),
            handles: Mutex::default(),
            incoming,
            shutdown,
            changed: Notify::new(),
            counters: Default::default(),
            #[cfg(test)]
            observed_transit: Mutex::default(),
        });
        // Bind every native listener before spawning any tasks, so startup failure is atomic.
        let mut tcp = Vec::new();
        let mut udp = Vec::new();
        let mut bound = Vec::new();
        for listener in &inner.config.listeners {
            let address = match listener.transport {
                TransportKind::NativeTcp => {
                    let socket = TcpListener::bind(listener.bind).await?;
                    let address = socket.local_addr()?;
                    tcp.push(socket);
                    address
                }
                TransportKind::NativeUdp => {
                    let socket = UdpSocket::bind(listener.bind).await?;
                    let address = socket.local_addr()?;
                    udp.push(socket);
                    address
                }
                TransportKind::Vless => {
                    return Err(Error::InvalidConfig("external VLESS listener required"));
                }
            };
            bound.push(BoundListener {
                transport: listener.transport,
                address,
            });
        }
        let runtime = Self {
            handle: Arc::new(Handle {
                inner: inner.clone(),
                bound,
                receiver: Mutex::new(Some(delivery_receiver)),
            }),
        };
        inner.spawn(probe::dispatch(inner.clone(), receiver));
        for listener in tcp {
            inner.spawn(stream::listen(inner.clone(), listener));
        }
        for socket in udp {
            inner.spawn(udp::run(inner.clone(), socket, None));
        }
        for candidate in inner.config.outgoing.clone() {
            match candidate.transport {
                TransportKind::NativeTcp => inner.spawn(stream::maintain(inner.clone(), candidate)),
                TransportKind::NativeUdp => inner.spawn(udp::maintain(inner.clone(), candidate)),
                TransportKind::Vless => (), // Established by CLI adapter, then attach_stream.
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
        if transport == TransportKind::NativeUdp {
            return Err(Error::Protocol);
        }
        let inner = &self.handle.inner;
        if *inner.shutdown.borrow() {
            return Err(Error::Closed);
        }
        let permit = inner
            .semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)?;
        let established = stream::authenticate(inner, stream, role, transport, priority).await?;
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

    pub fn statistics(&self) -> Statistics {
        let inner = &self.handle.inner;
        Statistics {
            native_udp: inner.counters[0].snapshot(),
            native_tcp: inner.counters[1].snapshot(),
            vless: inner.counters[2].snapshot(),
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
        &self.counters[match transport {
            TransportKind::NativeUdp => 0,
            TransportKind::NativeTcp => 1,
            TransportKind::Vless => 2,
        }]
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
        let mut peers = lock(&self.peers);
        if *self.shutdown.borrow() {
            return Err(Error::Closed);
        }
        if let Some(previous) = peers.get(&peer) {
            if (previous.priority, previous.token) <= (channel.priority, channel.token) {
                return Err(Error::Capacity);
            }
            let _ = previous.close.send(true);
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
        {
            peers.remove(peer);
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
        let channel = lock(&self.peers)
            .get(peer)
            .cloned()
            .ok_or(Error::Unavailable)?;
        let batch = Batch::new(vec![packet.encode()?])?;
        channel
            .tx
            .try_send(Outbound {
                token: channel.token,
                batch,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => Error::Capacity,
                mpsc::error::TrySendError::Closed(_) => Error::Unavailable,
            })
    }
    fn deliver(&self, incoming: Incoming) {
        let counters = self.counters(incoming.transport);
        counters.received.fetch_add(1, Ordering::Relaxed);
        if self.incoming.try_send(incoming).is_err() {
            counters.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}
