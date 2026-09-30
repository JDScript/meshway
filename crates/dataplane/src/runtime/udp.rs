use super::{
    Channel, Incoming, Inner, Outbound, lock,
    wire::{HOP_ACK, HOP_CONFIRM, hop_prologue},
};
use crate::{
    Error, OutgoingCandidate, Result, TransportKind,
    crypto::{Cipher, read_handshake, write_handshake},
};
use snow::HandshakeState;
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

pub(super) async fn maintain(inner: Arc<Inner>, candidate: OutgoingCandidate) {
    let Ok(address) = candidate.endpoint.parse::<SocketAddr>() else {
        return;
    };
    let bind: SocketAddr = if address.is_ipv4() {
        ([0, 0, 0, 0], 0).into()
    } else {
        ([0_u16; 8], 0).into()
    };
    let mut shutdown = inner.shutdown.subscribe();
    if *shutdown.borrow() {
        return;
    }
    let socket = tokio::select! {
        _ = shutdown.changed() => return,
        result = UdpSocket::bind(bind) => match result { Ok(socket) => socket, Err(_) => return },
    };
    run(inner, socket, Some(candidate)).await;
}

pub(super) async fn run(
    inner: Arc<Inner>,
    socket: UdpSocket,
    candidate: Option<OutgoingCandidate>,
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
    // One extra byte detects truncation of oversized datagrams without allocating from a wire length.
    let mut buffer = vec![0; UDP_LIMIT + 1];
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
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
                        let _ = send_encrypted(&socket, pending.address, &mut pending.cipher, Kind::Confirm, HOP_CONFIRM).await;
                    }
                }
                for session in sessions.values_mut() {
                    if now.duration_since(session.pinged) >= PING_EVERY {
                        session.pinged = now;
                        if send_datagram(&socket, session, &[0]).await.is_err() {
                            inner.counters(TransportKind::NativeUdp).dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                if let Some(candidate) = &candidate {
                    let need = lock(&inner.peers).get(&candidate.peer).is_none_or(|peer| peer.priority > candidate.priority);
                    if need && pending.is_none() && !confirming.values().any(|pending| pending.initiator) && now >= next_attempt {
                        next_attempt = now + Duration::from_millis(inner.config.limits.reconnect_delay_ms);
                        pending = initiate(&inner, &socket, candidate).await.ok();
                    }
                }
            }
            outbound = rx.recv() => {
                let Some(outbound) = outbound else { break };
                let Some(session) = sessions.get_mut(&outbound.token) else { continue };
                for frame in outbound.batch.into_frames() {
                    let mut plaintext = Vec::with_capacity(frame.len() + 1);
                    plaintext.push(1);
                    plaintext.extend_from_slice(&frame);
                    match send_datagram(&socket, session, &plaintext).await {
                        Ok(()) => { inner.counters(TransportKind::NativeUdp).sent.fetch_add(1, Ordering::Relaxed); }
                        Err(_) => { inner.counters(TransportKind::NativeUdp).dropped.fetch_add(1, Ordering::Relaxed); }
                    }
                }
            }
            datagram = socket.recv_from(&mut buffer) => {
                let Ok((size, source)) = datagram else { break };
                let result = if size == 0 || size > UDP_LIMIT { Err(Error::Oversize) }
                    else { receive(&inner, &socket, &tx, &mut sessions, &mut pending, &mut confirming, &buffer[..size], source).await };
                if result.is_err() { inner.counters(TransportKind::NativeUdp).dropped.fetch_add(1, Ordering::Relaxed); }
            }
        }
    }
    for (token, session) in sessions {
        inner.unregister(&session.peer, &token);
    }
}

async fn initiate(
    inner: &Inner,
    socket: &UdpSocket,
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
    socket: &UdpSocket,
    tx: &mpsc::Sender<Outbound>,
    sessions: &mut BTreeMap<[u8; 32], Session>,
    pending: &mut Option<Pending>,
    confirming: &mut BTreeMap<[u8; 32], Confirming>,
    datagram: &[u8],
    source: SocketAddr,
) -> Result<()> {
    match Kind::try_from(datagram[0])? {
        Kind::Hello => {
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
                return Err(Error::Protocol);
            }
            let peer = inner.identify(state.get_remote_static())?;
            inner.authorize_transport(&peer, TransportKind::NativeUdp)?;
            let response = write_handshake(&mut state, &setup)?;
            let cipher = Cipher::finish(state)?;
            let priority = u16::from_be_bytes([setup[1], setup[2]]);
            let mut response_datagram = vec![Kind::Response as u8];
            response_datagram.extend_from_slice(&response);
            send_raw(socket, &response_datagram, source).await?;
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
            if waiting.address != source || waiting.expires <= Instant::now() {
                return Err(Error::Authentication);
            }
            let setup = read_handshake(&mut waiting.state, &datagram[1..])?;
            if setup != [1, (waiting.priority >> 8) as u8, waiting.priority as u8]
                || inner.identify(waiting.state.get_remote_static())? != waiting.peer
            {
                return Err(Error::Authentication);
            }
            let waiting = pending.take().ok_or(Error::Protocol)?;
            let mut cipher = Cipher::finish(waiting.state)?;
            send_encrypted(socket, source, &mut cipher, Kind::Confirm, HOP_CONFIRM).await?;
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
                return Err(Error::Protocol);
            }
            let token: [u8; 32] = datagram[1..33].try_into().map_err(|_| Error::Protocol)?;
            let is_ack = matches!(kind, Kind::Ack);
            if let Some(waiting) = confirming.get_mut(&token) {
                if waiting.address != source
                    || waiting.expires <= Instant::now()
                    || waiting.initiator != is_ack
                {
                    return Err(Error::Authentication);
                }
                let expected = if is_ack { HOP_ACK } else { HOP_CONFIRM };
                if waiting.cipher.open(&datagram[33..])? != expected {
                    return Err(Error::Authentication);
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
                )?;
            } else if is_ack {
                return Err(Error::Authentication);
            } else {
                // A lost ACK can be recovered by a fresh-nonce confirmation from
                // the original source address; replays never refresh or promote state.
                let session = sessions.get_mut(&token).ok_or(Error::Authentication)?;
                if !session.responder
                    || session.address != source
                    || !inner.active(&session.peer, &token)
                    || session.cipher.open(&datagram[33..])? != HOP_CONFIRM
                {
                    return Err(Error::Authentication);
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
                return Err(Error::Protocol);
            }
            let token: [u8; 32] = datagram[1..33].try_into().map_err(|_| Error::Protocol)?;
            let session = sessions.get_mut(&token).ok_or(Error::Authentication)?;
            if session.address != source || !inner.active(&session.peer, &token) {
                return Err(Error::Authentication);
            }
            let plaintext = session.cipher.open(&datagram[33..])?;
            session.received = Instant::now();
            match plaintext.as_slice() {
                [0] => send_datagram(socket, session, &[2]).await,
                [2] => Ok(()),
                [1, body @ ..] if !body.is_empty() => {
                    inner.deliver(Incoming {
                        peer: session.peer.clone(),
                        token,
                        transport: TransportKind::NativeUdp,
                        body: body.to_vec(),
                    });
                    Ok(())
                }
                _ => Err(Error::Protocol),
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

async fn send_datagram(socket: &UdpSocket, session: &mut Session, plaintext: &[u8]) -> Result<()> {
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
    socket: &UdpSocket,
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

async fn send_raw(socket: &UdpSocket, datagram: &[u8], address: SocketAddr) -> Result<()> {
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
