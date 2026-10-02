//! Bounded observations on the carrier socket; never connection endpoint selection.
use super::sockets::NativeUdp;
use super::{Inner, Runtime, lock};
use crate::{Error, Result};
use meshway_protocol::stun::{self, Response};
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    time::Instant,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UdpSocketInfo {
    pub socket_id: u32,
    pub local_addr: SocketAddr,
    pub listener_id: Option<String>,
    pub candidate_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UdpDiscoveryOutcome {
    /// Fresh, strictly validated Binding success from the requested observer.
    Mapped,
    /// No matching response before the absolute transaction deadline.
    Timeout,
    /// Matching malformed responses were seen, without a valid result by deadline.
    InvalidResponse,
    /// The carrier socket could not send the Binding request.
    IoError,
    /// A strictly validated STUN error response from the requested observer.
    StunError,
}

#[derive(Clone, Debug)]
pub struct UdpDiscoveryObservation {
    pub socket: UdpSocketInfo,
    pub observer: SocketAddr,
    pub outcome: UdpDiscoveryOutcome,
    pub mapped_endpoint: Option<SocketAddr>,
    pub round_trip: Option<Duration>,
}

pub(super) struct Registration {
    pub generation: Arc<super::generation::Fence>,
    pub info: UdpSocketInfo,
    pub tx: mpsc::Sender<Request>,
    pub slot: Arc<Semaphore>,
}

pub(super) struct LiveSocket {
    pub inner: Arc<Inner>,
    pub id: u32,
    pub rx: mpsc::Receiver<Request>,
}

impl Drop for LiveSocket {
    fn drop(&mut self) {
        lock(&self.inner.udp_sockets).remove(&self.id);
    }
}

pub(super) struct Request {
    cancelled: Arc<tokio::sync::Notify>,
    info: UdpSocketInfo,
    server: SocketAddr,
    transaction: [u8; 12],
    deadline: Instant,
    sent: Option<Instant>,
    invalid: Arc<AtomicBool>,
    reply: oneshot::Sender<UdpDiscoveryObservation>,
    _socket_permit: OwnedSemaphorePermit,
    _global_permit: OwnedSemaphorePermit,
}

impl Runtime {
    /// Current live engine sockets only. Ordinals are local to this Runtime.
    pub fn udp_sockets(&self) -> Vec<UdpSocketInfo> {
        let inner = &self.handle.inner;
        let sockets = lock(&inner.udp_sockets);
        if *inner.shutdown.borrow() {
            return Vec::new();
        }
        sockets
            .values()
            .filter(|entry| !entry.tx.is_closed() && !entry.generation.retired())
            .map(|entry| entry.info.clone())
            .collect()
    }

    /// One bounded Binding measurement; the result never changes dialing or authorization.
    /// Timeouts include enqueue/send/receive. Capacity is one per socket, sixteen per Runtime.
    pub async fn discover_udp(
        &self,
        socket_id: u32,
        server: SocketAddr,
        timeout: Duration,
    ) -> Result<UdpDiscoveryObservation> {
        let inner = &self.handle.inner;
        if !(Duration::from_millis(250)..=Duration::from_millis(1500)).contains(&timeout) {
            return Err(Error::InvalidConfig(
                "UDP discovery timeout outside 250..1500 ms",
            ));
        }
        let deadline = Instant::now() + timeout;
        let (tx, info, socket_permit, generation) = {
            let sockets = lock(&inner.udp_sockets);
            if *inner.shutdown.borrow() {
                return Err(Error::Closed);
            }
            let entry = sockets.get(&socket_id).ok_or(Error::Unavailable)?;
            if entry.generation.retired() {
                return Err(Error::Closed);
            }
            validate_server(server, entry.info.local_addr)?;
            let permit = entry
                .slot
                .clone()
                .try_acquire_owned()
                .map_err(|_| Error::Capacity)?;
            (
                entry.tx.clone(),
                entry.info.clone(),
                permit,
                entry.generation.clone(),
            )
        };
        let global_permit = inner
            .udp_discovery_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)?;
        let mut transaction = [0; 12];
        getrandom::fill(&mut transaction).map_err(|_| Error::Io)?;
        let (reply, receive) = oneshot::channel();
        let cancelled = Arc::new(tokio::sync::Notify::new());
        let invalid = Arc::new(AtomicBool::new(false));
        let _cancel = Cancel(cancelled.clone());
        tx.try_send(Request {
            cancelled,
            info: info.clone(),
            server,
            transaction,
            deadline,
            sent: None,
            invalid: invalid.clone(),
            reply,
            _socket_permit: socket_permit,
            _global_permit: global_permit,
        })
        .map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => Error::Capacity,
            mpsc::error::TrySendError::Closed(_) => Error::Closed,
        })?;
        let mut shutdown = inner.shutdown.subscribe();
        if *shutdown.borrow() {
            return Err(Error::Closed);
        }
        let observation = tokio::select! {
            _ = shutdown.changed() => return Err(Error::Closed),
            observation = receive => observation.map_err(|_| Error::Closed)?,
            _ = tokio::time::sleep_until(deadline) => UdpDiscoveryObservation {
                socket: info.clone(), observer: server,
                outcome: if invalid.load(Ordering::Relaxed) { UdpDiscoveryOutcome::InvalidResponse } else { UdpDiscoveryOutcome::Timeout },
                mapped_endpoint: None, round_trip: None,
            },
        };
        // Reject metadata from an engine that exited or a retired Runtime, even if
        // its response had already entered the oneshot queue before retirement.
        let sockets = lock(&inner.udp_sockets);
        if *inner.shutdown.borrow()
            || generation.retired()
            || !sockets.get(&socket_id).is_some_and(|entry| {
                entry.info == info
                    && Arc::ptr_eq(&entry.generation, &generation)
                    && !entry.tx.is_closed()
            })
        {
            return Err(Error::Closed);
        }
        Ok(observation)
    }
}

fn validate_server(server: SocketAddr, local: SocketAddr) -> Result<()> {
    let ip = server.ip();
    let invalid_v6 = matches!(server, SocketAddr::V6(v6) if v6.scope_id() != 0 || v6.flowinfo() != 0 || v6.ip().to_ipv4_mapped().is_some() || v6.ip().is_unicast_link_local());
    if server.port() == 0
        || server.is_ipv4() != local.is_ipv4()
        || ip.is_unspecified()
        || ip.is_multicast()
        || matches!(ip, std::net::IpAddr::V4(v4) if v4.is_broadcast())
        || invalid_v6
    {
        return Err(Error::InvalidConfig("invalid UDP discovery observer"));
    }
    Ok(())
}

impl Request {
    pub fn finish(self, outcome: UdpDiscoveryOutcome, mapped_endpoint: Option<SocketAddr>) {
        let round_trip = if outcome == UdpDiscoveryOutcome::Mapped {
            self.sent.map(|sent| sent.elapsed())
        } else {
            None
        };
        let _ = self.reply.send(UdpDiscoveryObservation {
            socket: self.info,
            observer: self.server,
            outcome,
            mapped_endpoint,
            round_trip,
        });
    }
}

/// Cheap fixed-header demux, without allocations or full parsing on Noise traffic.
pub(super) fn receive(pending: &mut Option<Request>, packet: &[u8], source: SocketAddr) -> bool {
    if let Some(request) = pending.as_mut()
        && source == request.server
        && packet.len() >= 20
        && packet[8..20] == request.transaction
    {
        if request.reply.is_closed() {
            *pending = None;
            return true;
        }
        if Instant::now() >= request.deadline {
            expire(pending);
            return true;
        }
        if request.sent.is_none() {
            return true;
        }
        match stun::parse(
            packet,
            request.transaction,
            request.info.local_addr.is_ipv4(),
        ) {
            Response::Mapped(endpoint) => pending
                .take()
                .unwrap()
                .finish(UdpDiscoveryOutcome::Mapped, Some(endpoint)),
            Response::Error => pending
                .take()
                .unwrap()
                .finish(UdpDiscoveryOutcome::StunError, None),
            Response::Invalid | Response::Ignore => request.invalid.store(true, Ordering::Relaxed),
        }
        return true;
    }
    // Binding requests and unrelated responses are not Noise Hello (byte 1).
    // This can only discard input; it never promotes a peer or sends a reply.
    packet.len() >= 20
        && matches!((packet[0], packet[1]), (0, 1 | 0x11) | (1, 1 | 0x11))
        && packet[4..8] == [0x21, 0x12, 0xa4, 0x42]
}

pub(super) fn expire(pending: &mut Option<Request>) {
    if let Some(request) = pending.take() {
        let outcome = if request.invalid.load(Ordering::Relaxed) {
            UdpDiscoveryOutcome::InvalidResponse
        } else {
            UdpDiscoveryOutcome::Timeout
        };
        request.finish(outcome, None);
    }
}

pub(super) async fn deadline(pending: &Option<Request>) {
    match pending {
        Some(request) => tokio::time::sleep_until(request.deadline).await,
        None => std::future::pending().await,
    }
}

struct Cancel(Arc<tokio::sync::Notify>);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}

pub(super) async fn cancelled(pending: &Option<Request>) {
    match pending {
        Some(request) => request.cancelled.notified().await,
        None => std::future::pending().await,
    }
}

pub(super) async fn writable(
    socket: &dyn NativeUdp,
    pending: &Option<Request>,
) -> std::io::Result<()> {
    if pending
        .as_ref()
        .is_some_and(|request| request.sent.is_none())
    {
        socket.writable().await
    } else {
        std::future::pending().await
    }
}

pub(super) fn send(
    socket: &dyn NativeUdp,
    pending: &mut Option<Request>,
    ready: std::io::Result<()>,
) {
    let Some(request) = pending.as_mut() else {
        return;
    };
    if request.reply.is_closed() {
        *pending = None;
        return;
    }
    if Instant::now() >= request.deadline {
        expire(pending);
        return;
    }
    let sent = Instant::now();
    let result = ready
        .and_then(|()| socket.try_send_to(&stun::request(request.transaction), request.server));
    match result {
        Ok(20) => request.sent = Some(sent),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => (),
        _ => pending
            .take()
            .unwrap()
            .finish(UdpDiscoveryOutcome::IoError, None),
    }
}
