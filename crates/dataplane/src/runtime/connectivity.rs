//! Peer-scoped, coordinated UDP attempts on an outbound ephemeral carrier.
//! Candidates are hints; only the existing Noise + key-confirmation exchange
//! can install an adjacency. This module does not change logical packet paths.
use super::{
    Inner, PeerConnection, Runtime, UdpSocketInfo, discovery, generation, lock, sockets, udp,
};
use crate::{Error, Result, TransportKind, config::validate_context_id};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    time::Instant,
};

/// Local capability bound to one socket generation, never a wire identifier.
#[derive(Clone)]
pub struct UdpPunchSocket {
    pub info: UdpSocketInfo,
    generation: Arc<generation::Fence>,
    inner: Arc<Inner>,
    peer: String,
}

#[derive(Clone, Debug)]
pub struct UdpPunchObservation {
    pub socket: UdpSocketInfo,
    /// Exact authenticated selected carrier, not a STUN reachability claim.
    pub connection: PeerConnection,
}

pub(super) struct Request {
    pub peer: String,
    pub candidates: Vec<SocketAddr>,
    pub priority: u16,
    pub deadline: Instant,
    pub next_attempt: Instant,
    pub next_candidate: usize,
    pub reply: oneshot::Sender<Result<PeerConnection>>,
    _permit: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
    cancelled: Arc<tokio::sync::Notify>,
}

impl Request {
    pub fn accepts(&self, source: SocketAddr) -> bool {
        !self.reply.is_closed()
            && Instant::now() < self.deadline
            && self.candidates.contains(&source)
    }
    pub fn finish(self, result: Result<PeerConnection>) {
        let _ = self.reply.send(result);
    }
    /// A writable wait may not extend the attempt lease or keep cancelled
    /// authentication work alive. Established-session I/O is independent.
    pub async fn run<T>(&self, operation: impl Future<Output = Result<T>>) -> Result<T> {
        if self.reply.is_closed() || Instant::now() >= self.deadline {
            return Err(Error::Timeout);
        }
        tokio::select! {
            biased;
            _ = self.cancelled.notified() => Err(Error::Closed),
            _ = tokio::time::sleep_until(self.deadline) => Err(Error::Timeout),
            result = operation => result,
        }
    }
}

impl Runtime {
    /// Open a dial-only socket without a listener or a manually configured peer
    /// endpoint. Uses the Runtime's current owning factory (including protection).
    /// Rebinding retires this capability; the lifecycle owner must open a new one.
    pub async fn open_udp_punch_socket(
        &self,
        peer: &str,
        id: &str,
        ipv6: bool,
    ) -> Result<UdpPunchSocket> {
        validate_context_id(Some(id))?;
        let inner = &self.handle.inner;
        inner.authorize_transport(peer, TransportKind::NativeUdp)?;
        let mut native = self.handle.native.lock().await;
        if *inner.shutdown.borrow() || native.generation.0.retired() {
            return Err(Error::Closed);
        }
        // Bound total dynamic allocations for this generation, including failed
        // engines. No task or socket can grow without a configured peer bound.
        if native.punch_sockets >= 16 {
            return Err(Error::Capacity);
        }
        if lock(&inner.udp_sockets)
            .values()
            .any(|entry| entry.info.candidate_id.as_deref() == Some(id))
        {
            return Err(Error::InvalidConfig("duplicate UDP socket resource id"));
        }
        let bind = if ipv6 {
            SocketAddr::from(([0_u16; 8], 0))
        } else {
            SocketAddr::from(([0, 0, 0, 0], 0))
        };
        let socket = native.factory.udp(
            bind,
            inner.config.limits.udp_receive_buffer_bytes,
            sockets::NativeSocketPurpose::Dial { id: Some(id), peer },
        )?;
        let socket_id = native.next_socket_id;
        let info = UdpSocketInfo {
            socket_id,
            local_addr: socket.local_addr()?,
            listener_id: None,
            candidate_id: Some(id.into()),
        };
        let (tx, rx) = mpsc::channel(1);
        let (punch_tx, punch_rx) = mpsc::channel(1);
        let generation = native.generation.0.clone();
        lock(&inner.udp_sockets).insert(
            socket_id,
            discovery::Registration {
                generation: generation.clone(),
                info: info.clone(),
                tx,
                slot: Arc::new(Semaphore::new(1)),
                punch: Some((punch_tx, Arc::new(Semaphore::new(1)))),
            },
        );
        let live = discovery::LiveSocket {
            inner: inner.clone(),
            id: socket_id,
            rx,
            punch_rx: Some(punch_rx),
        };
        let context = super::StreamContext {
            candidate_id: Some(id.into()),
            ..Default::default()
        };
        let engine = udp::run(
            inner.clone(),
            socket,
            None,
            context,
            live,
            generation.clone(),
            Some(peer.into()),
        );
        let fence = generation.clone();
        native.tasks.push(tokio::spawn(async move {
            tokio::select! { biased; _ = fence.cancelled() => (), _ = engine => () }
        }));
        native.next_socket_id += 1;
        native.punch_sockets += 1;
        Ok(UdpPunchSocket {
            info,
            generation,
            inner: inner.clone(),
            peer: peer.into(),
        })
    }

    /// Execute one bounded coordinated attempt. The controller/lifecycle owner
    /// must schedule both peers with fresh, authorized candidates from the same
    /// policy/key snapshot. No durable authorization comes from these hints.
    /// Timeout/cancellation releases capacity when the engine discards pending
    /// handshakes, asynchronously; overlapping attempts return Capacity.
    pub async fn punch_udp(
        &self,
        socket: &UdpPunchSocket,
        candidates: Vec<SocketAddr>,
        priority: u16,
        timeout: Duration,
    ) -> Result<UdpPunchObservation> {
        if !Arc::ptr_eq(&socket.inner, &self.handle.inner) {
            return Err(Error::Unavailable);
        }
        if !(Duration::from_millis(500)..=Duration::from_secs(5)).contains(&timeout)
            || candidates.is_empty()
            || candidates.len() > 8
        {
            return Err(Error::InvalidConfig("invalid UDP punch bounds"));
        }
        for candidate in &candidates {
            discovery::validate_server(*candidate, socket.info.local_addr)?;
        }
        if candidates
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != candidates.len()
        {
            return Err(Error::InvalidConfig("duplicate UDP punch candidate"));
        }
        let deadline = Instant::now() + timeout;
        let inner = &self.handle.inner;
        let (tx, permit) = {
            let sockets = lock(&inner.udp_sockets);
            if *inner.shutdown.borrow() || socket.generation.retired() {
                return Err(Error::Closed);
            }
            let entry = sockets.get(&socket.info.socket_id).ok_or(Error::Closed)?;
            if entry.info != socket.info || !Arc::ptr_eq(&entry.generation, &socket.generation) {
                return Err(Error::Closed);
            }
            let (tx, slot) = entry.punch.as_ref().ok_or(Error::Unavailable)?;
            (
                tx.clone(),
                slot.clone()
                    .try_acquire_owned()
                    .map_err(|_| Error::Capacity)?,
            )
        };
        let global = inner
            .udp_punch_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)?;
        let (reply, receive) = oneshot::channel();
        let cancelled = Arc::new(tokio::sync::Notify::new());
        let _cancel = Cancel(cancelled.clone());
        tx.try_send(Request {
            peer: socket.peer.clone(),
            candidates,
            priority,
            deadline,
            next_attempt: Instant::now(),
            next_candidate: 0,
            reply,
            _permit: permit,
            _global: global,
            cancelled,
        })
        .map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => Error::Capacity,
            mpsc::error::TrySendError::Closed(_) => Error::Closed,
        })?;
        let mut shutdown = inner.shutdown.subscribe();
        let result = tokio::select! {
            biased;
            _ = socket.generation.cancelled() => return Err(Error::Closed),
            _ = shutdown.changed() => return Err(Error::Closed),
            result = receive => result.map_err(|_| Error::Closed)?,
            _ = tokio::time::sleep_until(deadline) => Err(Error::Timeout),
        };
        if *inner.shutdown.borrow() || socket.generation.retired() {
            return Err(Error::Closed);
        }
        Ok(UdpPunchObservation {
            socket: socket.info.clone(),
            connection: result?,
        })
    }
}

struct Cancel(Arc<tokio::sync::Notify>);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}
pub(super) async fn cancelled(request: &Option<Request>) {
    match request {
        Some(request) => request.cancelled.notified().await,
        None => std::future::pending().await,
    }
}
pub(super) async fn deadline(request: &Option<Request>) {
    match request {
        Some(request) => tokio::time::sleep_until(request.deadline).await,
        None => std::future::pending().await,
    }
}
pub(super) async fn receive(rx: &mut Option<mpsc::Receiver<Request>>) -> Option<Request> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}
