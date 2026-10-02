use super::{
    Channel, DropReason, Failure, Incoming, Inner, Outbound, StageResult, StreamContext,
    connectivity, discovery, lock,
    sockets::NativeUdp,
    wire::{HOP_ACK, HOP_CONFIRM, hop_prologue},
    work_budget::WorkBudget,
};
use crate::{
    Error, OutgoingCandidate, Result, TransportKind,
    crypto::{Cipher, read_handshake, write_handshake},
};
use snow::HandshakeState;
use socket2::{Domain, Protocol, SockRef, Socket, Type};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use tokio::{
    net::UdpSocket,
    sync::{OwnedSemaphorePermit, mpsc, watch},
};

const UDP_LIMIT: usize = super::MAX_NATIVE_UDP_DATAGRAM;
const DEAD_AFTER: Duration = Duration::from_secs(6);
const PING_EVERY: Duration = Duration::from_secs(1);
const CONFIRM_RETRY: Duration = Duration::from_millis(100);

#[derive(Clone, Copy)]
enum Kind {
    Hello = 1,
    Response = 2,
    Data = 3,
    Confirm = 4,
    Ack = 5,
}

impl TryFrom<u8> for Kind {
    type Error = Error;
    fn try_from(value: u8) -> Result<Self> {
        match value {
            1 => Ok(Self::Hello),
            2 => Ok(Self::Response),
            3 => Ok(Self::Data),
            4 => Ok(Self::Confirm),
            5 => Ok(Self::Ack),
            _ => Err(Error::Protocol),
        }
    }
}

struct Session {
    peer: String,
    address: SocketAddr,
    cipher: Cipher,
    received: Instant,
    pinged: Instant,
    responder: bool,
    _permit: OwnedSemaphorePermit,
}

struct Pending {
    state: HandshakeState,
    address: SocketAddr,
    peer: String,
    priority: u16,
    expires: Instant,
    permit: OwnedSemaphorePermit,
}

struct Confirming {
    peer: String,
    address: SocketAddr,
    cipher: Cipher,
    priority: u16,
    expires: Instant,
    retried: Instant,
    initiator: bool,
    permit: OwnedSemaphorePermit,
}

/// Prepare every UDP socket before spawning its engine, so setup failures are
/// returned to the caller rather than silently terminating a reconnect task.
pub(super) fn bind(address: SocketAddr, receive_buffer_bytes: usize) -> Result<UdpSocket> {
    let socket = Socket::new(
        Domain::for_address(address),
        Type::DGRAM,
        Some(Protocol::UDP),
    )?;
    if receive_buffer_bytes != 0 {
        SockRef::from(&socket).set_recv_buffer_size(receive_buffer_bytes)?;
    }
    socket.set_nonblocking(true)?;
    socket.bind(&address.into())?;
    Ok(UdpSocket::from_std(socket.into())?)
}

#[cfg(test)]
pub(super) fn bind_outgoing(
    candidate: &OutgoingCandidate,
    receive_buffer_bytes: usize,
) -> Result<UdpSocket> {
    let address = candidate
        .endpoint
        .parse::<SocketAddr>()
        .map_err(|_| Error::Protocol)?;
    let bind: SocketAddr = if address.is_ipv4() {
        ([0, 0, 0, 0], 0).into()
    } else {
        ([0_u16; 8], 0).into()
    };
    self::bind(bind, receive_buffer_bytes)
}

pub(super) async fn run(
    inner: Arc<Inner>,
    socket: Box<dyn NativeUdp>,
    candidate: Option<OutgoingCandidate>,
    context: StreamContext,
    mut live: discovery::LiveSocket,
    socket_generation: Arc<super::generation::Fence>,
    coordinated_peer: Option<String>,
) {
    let mut shutdown = inner.shutdown.subscribe();
    if *shutdown.borrow() {
        return;
    }
    let (tx, mut rx) = mpsc::channel::<Outbound>(inner.config.limits.queue_capacity);
    let mut sessions = BTreeMap::<[u8; 32], Session>::new();
    let mut pending: Option<Pending> = None;
    let mut confirming = BTreeMap::<[u8; 32], Confirming>::new();
    let mut next_attempt = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    // The socket engine shares one budget across receive and outbound frame work.
    let mut work = WorkBudget::new(inner.config.limits.work_budget_frames);
    // One extra byte detects truncation of oversized datagrams without allocating from a wire length.
    let mut buffer = vec![0; UDP_LIMIT + 1];
    let mut discovery_pending = None;
    let mut punch: Option<connectivity::Request> = None;
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            request = connectivity::receive(&mut live.punch_rx), if punch.is_none() => {
                let Some(request) = request else { break };
                // The socket capability has one immutable peer. Candidate hints
                // cannot turn it into a wildcard listener or change its identity.
                if coordinated_peer.as_deref() != Some(request.peer.as_str()) {
                    request.finish(Err(Error::UnauthorizedTransport));
                } else if !request.reply.is_closed() {
                    punch = Some(request);
                }
            }
            _ = connectivity::deadline(&punch) => {
                if let Some(request) = punch.take() { request.finish(Err(Error::Timeout)); }
                pending = None;
                confirming.clear();
            }
            _ = connectivity::cancelled(&punch) => {
                punch = None;
                pending = None;
                confirming.clear();
            }
            request = live.rx.recv(), if discovery_pending.is_none() => {
                let Some(request) = request else { break };
                discovery_pending = Some(request);
            }
            _ = discovery::deadline(&discovery_pending) => discovery::expire(&mut discovery_pending),
            _ = discovery::cancelled(&discovery_pending) => discovery_pending = None,
            ready = discovery::writable(socket.as_ref(), &discovery_pending) => {
                discovery::send(socket.as_ref(), &mut discovery_pending, ready);
                work.completed_frame().await;
            }
            _ = tick.tick() => {
                let now = Instant::now();
                sessions.retain(|token, session| {
                    let keep = now.duration_since(session.received) < DEAD_AFTER && inner.active(&session.peer, token);
                    if !keep { inner.unregister(&session.peer, token); }
                    keep
                });
                if pending.as_ref().is_some_and(|pending| pending.expires <= now) { pending = None; }
                confirming.retain(|_, pending| pending.expires > now);
                for pending in confirming.values_mut().filter(|pending| pending.initiator) {
                    if now.duration_since(pending.retried) >= CONFIRM_RETRY {
                        pending.retried = now;
                        // Fresh nonce for every retry; exact ciphertext replay remains rejected.
                        let send = send_encrypted(socket.as_ref(), pending.address, &mut pending.cipher, Kind::Confirm, HOP_CONFIRM);
                        if coordinated_peer.is_some() {
                            if let Some(request) = punch.as_ref() { let _ = request.run(send).await; }
                        } else { let _ = send.await; }
                    }
                }
                for session in sessions.values_mut() {
                    if now.duration_since(session.pinged) >= PING_EVERY {
                        session.pinged = now;
                        if let Err(error) = send_datagram(socket.as_ref(), session, &[0]).await {
                            inner.counters(TransportKind::NativeUdp).drop(error.into());
                        }
                    }
                }
                if let Some(request) = punch.as_mut() {
                    if request.reply.is_closed() || tokio::time::Instant::now() >= request.deadline {
                        let request = punch.take().unwrap();
                        request.finish(Err(Error::Timeout));
                        pending = None;
                        confirming.clear();
                    } else if let Some((token, session)) = sessions.iter().find(|(token, session)| {
                        session.peer == request.peer && request.accepts(session.address) && inner.active(&session.peer, token)
                    }) {
                        // Capture evidence from this engine's confirmed channel,
                        // never from a different socket/transport for this peer.
                        let connection = lock(&inner.peers).get(&session.peer)
                            .filter(|channel| channel.token == *token).map(|channel| channel.snapshot(&session.peer).connection);
                        if let Some(connection) = connection {
                            punch.take().unwrap().finish(Ok(connection));
                            pending = None;
                            confirming.clear();
                        }
                    } else {
                        // Both sides send from their eventual data socket to open
                        // endpoint-dependent filtering. This byte is untrusted,
                        // never answered and never installs or refreshes a peer.
                        for address in &request.candidates {
                            if !request.accepts(*address) { break; }
                            let _ = socket.try_send_to(&[0], *address);
                            work.completed_frame().await;
                        }
                        // Stable identity tie-break prevents simultaneous IK
                        // exchanges from racing the selected channel registry.
                        if inner.config.node_id < request.peer && pending.is_none()
                            && !confirming.values().any(|waiting| waiting.initiator)
                            && tokio::time::Instant::now() >= request.next_attempt {
                            let address = request.candidates[request.next_candidate];
                            request.next_candidate = (request.next_candidate + 1) % request.candidates.len();
                            request.next_attempt = tokio::time::Instant::now() + Duration::from_millis(400);
                            let candidate = OutgoingCandidate { peer: request.peer.clone(), transport: TransportKind::NativeUdp,
                                endpoint: address.to_string(), priority: request.priority, id: context.candidate_id.clone() };
                            pending = request.run(initiate(&inner, socket.as_ref(), &candidate)).await.ok();
                            if let Some(waiting) = pending.as_mut() {
                                waiting.expires = waiting.expires.min(Instant::now() + Duration::from_millis(400)).min(request.deadline.into_std());
                            }
                        }
                    }
                }
                if let Some(candidate) = &candidate {
                    let need = lock(&inner.peers).get(&candidate.peer).is_none_or(|peer| peer.priority > candidate.priority);
                    if need && pending.is_none() && !confirming.values().any(|pending| pending.initiator) && now >= next_attempt {
                        next_attempt = now + Duration::from_millis(inner.config.limits.reconnect_delay_ms);
                        pending = initiate(&inner, socket.as_ref(), candidate).await.ok();
                    }
                }
            }
            outbound = rx.recv() => {
                let Some(outbound) = outbound else { break };
                if outbound.generation.as_ref().is_some_and(|fence| fence.retired()) {
                    work.completed_frame().await;
                    continue;
                }
                let Some(session) = sessions.get_mut(&outbound.token) else {
                    inner.counters(TransportKind::NativeUdp).drop(DropReason::StaleChannel);
                    for _ in 0..outbound.batch.len() {
                        work.completed_frame().await;
                    }
                    continue;
                };
                for frame in outbound.batch.into_frames() {
                    let mut plaintext = Vec::with_capacity(frame.len() + 1);
                    plaintext.push(1);
                    plaintext.extend_from_slice(&frame);
                    let send = send_datagram(socket.as_ref(), session, &plaintext);
                    let result = if let Some(fence) = &outbound.generation {
                        fence.run(send).await
                    } else {
                        send.await
                    };
                    match result {
                        Ok(()) => {
                            if frame.first() != Some(&super::neighbor::NAMESPACE) {
                                inner.counters(TransportKind::NativeUdp).sent.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Err(error) => { inner.counters(TransportKind::NativeUdp).drop(error.into()); }
                    }
                    // Account each sealed/sent frame, not just the bounded batch entry.
                    work.completed_frame().await;
                }
            }
            datagram = socket.recv_from(&mut buffer) => {
                let Ok((size, source)) = datagram else { break };
                // Outbound opening datagrams are deliberately not a probe:
                // no response, freshness update, or authorization effect.
                let ignored_opening = coordinated_peer.is_some() && size == 1 && buffer[0] == 0;
                let result = if ignored_opening || discovery::receive(&mut discovery_pending, &buffer[..size], source) { Ok(()) }
                    else if size == 0 || size > UDP_LIMIT { Err(Error::Oversize.into()) }
                    else { receive(&inner, socket.as_ref(), &tx, &mut sessions, &mut pending, &mut confirming, &buffer[..size], source,
                        candidate.is_none() && coordinated_peer.is_none(), &context, &socket_generation, punch.as_ref(), coordinated_peer.is_some()).await };
                if let Err(failure) = result { inner.counters(TransportKind::NativeUdp).drop(failure.reason); }
                // All datagrams consume work, including rejected input and hop control.
                // Authentication/confirmation and any reply finish before yielding.
                work.completed_frame().await;
            }
        }
    }
    for (token, session) in sessions {
        inner.unregister(&session.peer, &token);
    }
}

async fn initiate(
    inner: &Inner,
    socket: &dyn NativeUdp,
    candidate: &OutgoingCandidate,
) -> Result<Pending> {
    inner.authorize_transport(&candidate.peer, TransportKind::NativeUdp)?;
    let permit = inner
        .semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::Capacity)?;
    let address = candidate.endpoint.parse().map_err(|_| Error::Protocol)?;
    let key = inner
        .keys
        .get(&candidate.peer)
        .ok_or(Error::Authentication)?;
    let mut state = inner
        .identity
        .handshake(Some(key), &hop_prologue(TransportKind::NativeUdp))?;
    let setup = [1, (candidate.priority >> 8) as u8, candidate.priority as u8];
    let first = write_handshake(&mut state, &setup)?;
    let mut datagram = vec![Kind::Hello as u8];
    datagram.extend_from_slice(&first);
    send_raw(socket, &datagram, address).await?;
    Ok(Pending {
        state,
        address,
        peer: candidate.peer.clone(),
        priority: candidate.priority,
        expires: Instant::now() + Duration::from_millis(inner.config.limits.handshake_timeout_ms),
        permit,
    })
}

#[allow(clippy::too_many_arguments)]
async fn receive(
    inner: &Inner,
    socket: &dyn NativeUdp,
    tx: &mpsc::Sender<Outbound>,
    sessions: &mut BTreeMap<[u8; 32], Session>,
    pending: &mut Option<Pending>,
    confirming: &mut BTreeMap<[u8; 32], Confirming>,
    datagram: &[u8],
    source: SocketAddr,
    listener: bool,
    context: &StreamContext,
    socket_generation: &Arc<super::generation::Fence>,
    punch: Option<&connectivity::Request>,
    coordinated: bool,
) -> StageResult<()> {
    match Kind::try_from(datagram[0])? {
        Kind::Hello => {
            // Outgoing candidate sockets are dial-only, not implicit listeners.
            // Responses and confirmed bidirectional data still use this socket.
            if !listener
                && !punch.is_some_and(|request| {
                    request.accepts(source) && inner.config.node_id > request.peer
                })
            {
                return Err(Error::UnauthorizedTransport.into());
            }
            let permit = inner
                .semaphore
                .clone()
                .try_acquire_owned()
                .map_err(|_| Error::Capacity)?;
            let mut state = inner
                .identity
                .handshake(None, &hop_prologue(TransportKind::NativeUdp))?;
            let setup = read_handshake(&mut state, &datagram[1..])?;
            if setup.len() != 3 || setup[0] != 1 {
                return Err(Error::Protocol.into());
            }
            let peer = inner.identify(state.get_remote_static())?;
            inner.authorize_transport(&peer, TransportKind::NativeUdp)?;
            context.authorize(&peer)?;
            if coordinated
                && !punch.is_some_and(|request| {
                    request.accepts(source)
                        && request.peer == peer
                        && setup[1..] == request.priority.to_be_bytes()
                })
            {
                return Err(Error::UnauthorizedTransport.into());
            }
            let response = write_handshake(&mut state, &setup)?;
            let cipher = Cipher::finish(state)?;
            let priority = u16::from_be_bytes([setup[1], setup[2]]);
            let mut response_datagram = vec![Kind::Response as u8];
            response_datagram.extend_from_slice(&response);
            let send = send_raw(socket, &response_datagram, source);
            if coordinated {
                punch.ok_or(Error::Authentication)?.run(send).await?;
            } else {
                send.await?;
            }
            // A replayed IK first flight can allocate only this expiring, permit-bounded
            // pending state. It cannot enter or replace the live registry.
            confirming.insert(
                cipher.id,
                Confirming {
                    peer,
                    address: source,
                    cipher,
                    priority,
                    expires: Instant::now()
                        + Duration::from_millis(inner.config.limits.handshake_timeout_ms),
                    retried: Instant::now(),
                    initiator: false,
                    permit,
                },
            );
            Ok(())
        }
        Kind::Response => {
            let waiting = pending.as_mut().ok_or(Error::Authentication)?;
            if coordinated
                && !punch
                    .is_some_and(|request| request.accepts(source) && request.peer == waiting.peer)
            {
                return Err(Error::Authentication.into());
            }
            if waiting.address != source || waiting.expires <= Instant::now() {
                return Err(Error::Authentication.into());
            }
            let setup = read_handshake(&mut waiting.state, &datagram[1..])?;
            if setup != [1, (waiting.priority >> 8) as u8, waiting.priority as u8]
                || inner.identify(waiting.state.get_remote_static())? != waiting.peer
            {
                return Err(Error::Authentication.into());
            }
            let waiting = pending.take().ok_or(Error::Protocol)?;
            let mut cipher = Cipher::finish(waiting.state)?;
            let send = send_encrypted(socket, source, &mut cipher, Kind::Confirm, HOP_CONFIRM);
            if coordinated {
                punch.ok_or(Error::Authentication)?.run(send).await?;
            } else {
                send.await?;
            }
            confirming.insert(
                cipher.id,
                Confirming {
                    peer: waiting.peer,
                    address: source,
                    cipher,
                    priority: waiting.priority,
                    permit: waiting.permit,
                    initiator: true,
                    expires: Instant::now()
                        + Duration::from_millis(inner.config.limits.handshake_timeout_ms),
                    retried: Instant::now(),
                },
            );
            Ok(())
        }
        Kind::Confirm | Kind::Ack => {
            let kind = Kind::try_from(datagram[0])?;
            if datagram.len() < 33 {
                return Err(Error::Protocol.into());
            }
            let token: [u8; 32] = datagram[1..33].try_into().map_err(|_| Error::Protocol)?;
            let is_ack = matches!(kind, Kind::Ack);
            if let Some(waiting) = confirming.get_mut(&token) {
                if coordinated
                    && !punch.is_some_and(|request| {
                        request.accepts(source)
                            && request.peer == waiting.peer
                            && request.priority == waiting.priority
                    })
                {
                    return Err(Error::Authentication.into());
                }
                if waiting.address != source
                    || waiting.expires <= Instant::now()
                    || waiting.initiator != is_ack
                {
                    return Err(Error::Authentication.into());
                }
                let expected = if is_ack { HOP_ACK } else { HOP_CONFIRM };
                if waiting.cipher.open(&datagram[33..])? != expected {
                    return Err(Error::Authentication.into());
                }
                let waiting = confirming.remove(&token).ok_or(Error::Protocol)?;
                install(
                    inner,
                    tx,
                    sessions,
                    waiting.peer,
                    source,
                    waiting.cipher,
                    waiting.priority,
                    waiting.permit,
                    !is_ack,
                    context,
                )?;
            } else if is_ack {
                return Err(Error::Authentication.into());
            } else {
                // A lost ACK can be recovered by a fresh-nonce confirmation from
                // the original source address; replays never refresh or promote state.
                let session = sessions.get_mut(&token).ok_or(Error::Authentication)?;
                if !session.responder
                    || session.address != source
                    || !inner.active(&session.peer, &token)
                    || session.cipher.open(&datagram[33..])? != HOP_CONFIRM
                {
                    return Err(Error::Authentication.into());
                }
            }
            if !is_ack {
                let session = sessions.get_mut(&token).ok_or(Error::Protocol)?;
                session.received = Instant::now();
                send_encrypted(socket, source, &mut session.cipher, Kind::Ack, HOP_ACK).await?;
            }
            Ok(())
        }
        Kind::Data => {
            if datagram.len() < 33 {
                return Err(Error::Protocol.into());
            }
            let token: [u8; 32] = datagram[1..33].try_into().map_err(|_| Error::Protocol)?;
            let session = sessions.get_mut(&token).ok_or(Error::Authentication)?;
            if session.address != source {
                return Err(Error::Authentication.into());
            }
            if !inner.active(&session.peer, &token) {
                return Err(Failure::new(
                    Error::Authentication,
                    DropReason::StaleChannel,
                ));
            }
            let plaintext = session.cipher.open(&datagram[33..])?;
            session.received = Instant::now();
            match plaintext.as_slice() {
                [0] => send_datagram(socket, session, &[2])
                    .await
                    .map_err(Into::into),
                [2] => Ok(()),
                [1, body @ ..] if !body.is_empty() => {
                    inner.deliver(Incoming {
                        socket_generation: Some(socket_generation.clone()),
                        peer: session.peer.clone(),
                        token,
                        transport: TransportKind::NativeUdp,
                        body: body.to_vec(),
                        generation: inner.packet_generation(),
                    });
                    Ok(())
                }
                _ => Err(Error::Protocol.into()),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn install(
    inner: &Inner,
    tx: &mpsc::Sender<Outbound>,
    sessions: &mut BTreeMap<[u8; 32], Session>,
    peer: String,
    address: SocketAddr,
    cipher: Cipher,
    priority: u16,
    permit: OwnedSemaphorePermit,
    responder: bool,
    context: &StreamContext,
) -> Result<()> {
    let token = cipher.id;
    let (close, _) = watch::channel(false);
    inner.register(
        peer.clone(),
        Channel {
            token,
            priority,
            transport: TransportKind::NativeUdp,
            tx: tx.clone(),
            close,
            initiator: !responder,
            context: Arc::new(StreamContext {
                remote_addr: Some(address),
                ..context.clone()
            }),
            probe_budget: Default::default(),
        },
    )?;
    sessions.insert(
        token,
        Session {
            peer,
            address,
            cipher,
            received: Instant::now(),
            pinged: Instant::now(),
            responder,
            _permit: permit,
        },
    );
    Ok(())
}

async fn send_datagram(
    socket: &dyn NativeUdp,
    session: &mut Session,
    plaintext: &[u8],
) -> Result<()> {
    send_encrypted(
        socket,
        session.address,
        &mut session.cipher,
        Kind::Data,
        plaintext,
    )
    .await
}

async fn send_encrypted(
    socket: &dyn NativeUdp,
    address: SocketAddr,
    cipher: &mut Cipher,
    kind: Kind,
    plaintext: &[u8],
) -> Result<()> {
    let ciphertext = cipher.seal(plaintext)?;
    let mut datagram = Vec::with_capacity(33 + ciphertext.len());
    datagram.push(kind as u8);
    datagram.extend_from_slice(&cipher.id);
    datagram.extend_from_slice(&ciphertext);
    send_raw(socket, &datagram, address).await
}

async fn send_raw(socket: &dyn NativeUdp, datagram: &[u8], address: SocketAddr) -> Result<()> {
    if datagram.len() > UDP_LIMIT {
        return Err(Error::Oversize);
    }
    // Even socket backpressure cannot keep the single UDP engine from shutdown indefinitely.
    tokio::time::timeout(
        Duration::from_millis(250),
        socket.send_to(datagram, address),
    )
    .await
    .map_err(|_| Error::Timeout)??;
    Ok(())
}
