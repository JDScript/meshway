//! Peer-scoped, coordinated UDP attempts on an outbound ephemeral carrier.
//! Candidates are hints; only the existing Noise + key-confirmation exchange
//! can install an adjacency. This module does not change logical packet paths.
use super::{
    Inner, InternalChannelSnapshot, PeerConnection, Runtime, UdpSocketInfo, discovery, generation,
    lock, sockets, udp,
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
    engine: Arc<generation::Fence>,
    task_id: tokio::task::Id,
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
    pub reply: oneshot::Sender<Result<InternalChannelSnapshot>>,
    // The existing generation fence is also the local attempt identity and
    // commit/retirement boundary; there is no second handshake state machine.
    pub fence: Arc<generation::Fence>,
    pub socket_generation: Arc<generation::Fence>,
    _permit: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
}

pub(super) struct Registration {
    pub tx: mpsc::Sender<Request>,
    pub slot: Arc<Semaphore>,
    pub peer: String,
    pub engine: Arc<generation::Fence>,
}

/// Actual engine owner, not the cloneable public capability. RAII returns its
/// live-resource permit on every exit, including abort before the first poll.
pub(super) struct SocketOwner {
    pub generation: sockets::SocketGeneration,
    _permit: OwnedSemaphorePermit,
}

impl Request {
    pub fn accepts(&self, source: SocketAddr) -> bool {
        !self.reply.is_closed()
            && !self.fence.retired()
            && Instant::now() < self.deadline
            && self.candidates.contains(&source)
    }
    pub fn finish(self, result: Result<InternalChannelSnapshot>) {
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
            _ = self.fence.cancelled() => Err(Error::Closed),
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
        native.tasks.retain(|task| !task.is_finished());
        {
            let sockets = lock(&inner.udp_sockets);
            if sockets
                .values()
                .any(|entry| entry.info.candidate_id.as_deref() == Some(id))
            {
                return Err(Error::InvalidConfig("duplicate UDP socket resource id"));
            }
            // One live socket per peer/address family; v4 and v6 together may
            // consume two slots, but one peer cannot consume the global budget.
            if sockets.values().any(|entry| {
                entry.info.local_addr.is_ipv6() == ipv6
                    && entry.punch.as_ref().is_some_and(|punch| punch.peer == peer)
            }) {
                return Err(Error::Capacity);
            }
        }
        let permit = native
            .punch_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)?;
        let owner = SocketOwner {
            generation: sockets::SocketGeneration::new(),
            _permit: permit,
        };
        let engine_fence = owner.generation.0.clone();
        let bind = if ipv6 {
            SocketAddr::from(([0_u16; 8], 0))
        } else {
            SocketAddr::from(([0, 0, 0, 0], 0))
        };
        // Nest the individual engine owner around the current protected native
        // factory. Closing one socket cannot retire all carriers in the Runtime.
        let socket = owner.generation.factory(native.factory.clone()).udp(
            bind,
            inner.config.limits.udp_receive_buffer_bytes,
            sockets::NativeSocketPurpose::Dial { id: Some(id), peer },
        )?;
        let socket_id = native.next_socket_id;
        let next_socket_id = socket_id.checked_add(1).ok_or(Error::Capacity)?;
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
                punch: Some(Registration {
                    tx: punch_tx,
                    slot: Arc::new(Semaphore::new(1)),
                    peer: peer.into(),
                    engine: engine_fence.clone(),
                }),
            },
        );
        let live = discovery::LiveSocket {
            inner: inner.clone(),
            id: socket_id,
            rx,
            punch_rx: Some(punch_rx),
            generation: generation.clone(),
            owner: Some(owner),
            channels: None,
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
            engine_fence.clone(),
            Some(peer.into()),
        );
        let fence = generation.clone();
        let closing = engine_fence.clone();
        let task = tokio::spawn(async move {
            tokio::select! { biased; _ = closing.cancelled() => (), _ = fence.cancelled() => (), _ = engine => () }
        });
        let task_id = task.id();
        native.tasks.push(task);
        native.next_socket_id = next_socket_id;
        Ok(UdpPunchSocket {
            info,
            generation,
            engine: engine_fence,
            task_id,
            inner: inner.clone(),
            peer: peer.into(),
        })
    }

    /// Retire one actual socket owner and await cleanup, including its channels
    /// and capacity permit. Dropping a capability clone is not a close request.
    pub async fn close_udp_punch_socket(&self, socket: &UdpPunchSocket) -> Result<()> {
        if !Arc::ptr_eq(&socket.inner, &self.handle.inner) {
            return Err(Error::Unavailable);
        }
        let mut native = self.handle.native.lock().await;
        socket.engine.retire();
        if let Some(index) = native
            .tasks
            .iter()
            .position(|task| task.id() == socket.task_id)
        {
            let _ = native.tasks.remove(index).await;
        }
        Ok(())
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
            let _engine = socket.engine.enter()?;
            let _generation = socket.generation.enter()?;
            let sockets = lock(&inner.udp_sockets);
            if *inner.shutdown.borrow() {
                return Err(Error::Closed);
            }
            let entry = sockets.get(&socket.info.socket_id).ok_or(Error::Closed)?;
            if entry.info != socket.info || !Arc::ptr_eq(&entry.generation, &socket.generation) {
                return Err(Error::Closed);
            }
            let punch = entry.punch.as_ref().ok_or(Error::Unavailable)?;
            if !Arc::ptr_eq(&punch.engine, &socket.engine) || punch.tx.is_closed() {
                return Err(Error::Closed);
            }
            (
                punch.tx.clone(),
                punch
                    .slot
                    .clone()
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
        let fence = generation::Fence::new();
        let _cancel = Cancel(fence.clone());
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
            fence,
            socket_generation: socket.generation.clone(),
        })
        .map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => Error::Capacity,
            mpsc::error::TrySendError::Closed(_) => Error::Closed,
        })?;
        let mut shutdown = inner.shutdown.subscribe();
        let result = tokio::select! {
            biased;
            _ = socket.generation.cancelled() => return Err(Error::Closed),
            _ = socket.engine.cancelled() => return Err(Error::Closed),
            _ = shutdown.changed() => return Err(Error::Closed),
            result = receive => result.map_err(|_| Error::Closed)?,
            _ = tokio::time::sleep_until(deadline) => Err(Error::Timeout),
        };
        // Lock order matches socket polls/registry admission: individual engine,
        // native generation, inventory, then peers. Validate the private token
        // at consumption, not just metadata from a queued successful reply.
        let _engine = socket.engine.enter()?;
        let _generation = socket.generation.enter()?;
        let sockets = lock(&inner.udp_sockets);
        if *inner.shutdown.borrow()
            || !sockets.get(&socket.info.socket_id).is_some_and(|entry| {
                entry.info == socket.info
                    && Arc::ptr_eq(&entry.generation, &socket.generation)
                    && !entry.tx.is_closed()
                    && entry.punch.as_ref().is_some_and(|punch| {
                        Arc::ptr_eq(&punch.engine, &socket.engine) && !punch.tx.is_closed()
                    })
            })
        {
            return Err(Error::Closed);
        }
        let snapshot = result?;
        let peers = lock(&inner.peers);
        if !peers.get(&socket.peer).is_some_and(|channel| {
            channel.token == snapshot.token
                && channel.priority == priority
                && channel.transport == TransportKind::NativeUdp
                && !channel.tx.is_closed()
        }) {
            return Err(Error::Unavailable);
        }
        Ok(UdpPunchObservation {
            socket: socket.info.clone(),
            connection: snapshot.connection,
        })
    }
}

struct Cancel(Arc<generation::Fence>);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.retire();
    }
}
pub(super) async fn cancelled(request: &Option<Request>) {
    match request {
        Some(request) => request.fence.cancelled().await,
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
