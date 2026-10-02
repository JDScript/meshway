use super::{
    DropReason, EndpointSession, Failure, Incoming, Inner, Packet, Phase, ProbeReply, StageResult,
    WaitingProbe, lock, work_budget::WorkBudget,
};
use crate::{
    Error, Result,
    crypto::{Cipher, read_handshake, write_handshake},
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::{sync::mpsc, time::Instant};
use zeroize::Zeroizing;

pub(super) struct PendingGuard {
    pub inner: Arc<Inner>,
    pub id: [u8; 16],
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        lock(&self.inner.pending).remove(&self.id);
    }
}

pub(super) async fn probe(
    inner: Arc<Inner>,
    path: Vec<String>,
    payload: Vec<u8>,
) -> Result<ProbeReply> {
    if payload.len() > super::wire::MAX_PAYLOAD {
        return Err(Error::Oversize);
    }
    let payload = Zeroizing::new(payload);
    if inner.authorize(&path)? != 0 {
        return Err(Error::UnauthorizedPath);
    }
    let destination = path.last().ok_or(Error::UnauthorizedPath)?;
    let remote = inner.keys.get(destination).ok_or(Error::Authentication)?;
    let mut id = [0; 16];
    getrandom::fill(&mut id).map_err(|_| Error::Authentication)?;
    let (tx, mut rx) = mpsc::channel(2);
    {
        let mut pending = lock(&inner.pending);
        if pending.len() >= inner.config.limits.max_pending_probes || pending.contains_key(&id) {
            return Err(Error::Capacity);
        }
        pending.insert(
            id,
            WaitingProbe {
                path: path.clone(),
                tx,
            },
        );
    }
    let _guard = PendingGuard {
        inner: inner.clone(),
        id,
    };
    let started = Instant::now();
    let mut shutdown = inner.shutdown.subscribe();
    if *shutdown.borrow() {
        return Err(Error::Closed);
    }
    let operation = async {
        let mut first = Packet {
            phase: Phase::Init,
            id,
            path: path.clone(),
            body: Vec::new(),
        };
        let mut state = inner
            .identity
            .handshake(Some(remote), &first.e2e_prologue())?;
        first.body = write_handshake(&mut state, b"echo-v1")?;
        inner.send(&path[1], first)?;
        let ready = rx.recv().await.ok_or(Error::Closed)?;
        if ready.phase != Phase::Ready
            || read_handshake(&mut state, &ready.body)? != b"echo-v1"
            || state.get_remote_static() != Some(remote.as_slice())
        {
            return Err(Error::Authentication);
        }
        let mut cipher = Cipher::finish(state)?;
        let body = cipher.seal(&payload)?;
        inner.send(
            &path[1],
            Packet {
                phase: Phase::Request,
                id,
                path: path.clone(),
                body,
            },
        )?;
        let echo = rx.recv().await.ok_or(Error::Closed)?;
        if echo.phase != Phase::Echo {
            return Err(Error::Protocol);
        }
        let response = cipher.open(&echo.body)?;
        if response.as_slice() != payload.as_slice() {
            return Err(Error::Authentication);
        }
        Ok(ProbeReply {
            payload: response,
            path,
            elapsed: started.elapsed(),
        })
    };
    tokio::select! {
        _ = shutdown.changed() => Err(Error::Closed),
        result = tokio::time::timeout(Duration::from_millis(inner.config.limits.probe_timeout_ms), operation) => result.map_err(|_| Error::Timeout)?,
    }
}

pub(super) async fn dispatch(inner: Arc<Inner>, mut receiver: mpsc::Receiver<Incoming>) {
    let mut shutdown = inner.shutdown.subscribe();
    if *shutdown.borrow() {
        return;
    }
    let mut cleanup = tokio::time::interval(Duration::from_millis(250));
    let mut work = WorkBudget::new(inner.config.limits.work_budget_frames);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = cleanup.tick() => {
                let now = Instant::now();
                lock(&inner.endpoints).retain(|_, session| session.expires > now);
                lock(&inner.packet_retries).retain(|_, setup| setup.current());
                lock(&inner.recovery).cleanup(now);
            }
            incoming = receiver.recv() => {
                let Some(incoming) = incoming else { break };
                let result = process_dispatch(&inner, &incoming);
                if let Err(failure) = result {
                    inner.counters(incoming.transport).drop(failure.reason);
                }
                // process is synchronous: no endpoint/registry/delivery lock crosses this yield.
                work.completed_frame().await;
            }
        }
    }
}

fn process_dispatch(inner: &Inner, incoming: &Incoming) -> StageResult<()> {
    // Only the regression opts into scheduler assistance while deliberately
    // blocking a synchronous dispatcher on a real mutex. Production is unchanged.
    #[cfg(test)]
    if inner.blocking_dispatch_test.load(Ordering::Relaxed) {
        return tokio::task::block_in_place(|| process(inner, incoming));
    }
    process(inner, incoming)
}

fn process(inner: &Inner, incoming: &Incoming) -> StageResult<()> {
    if !inner.active(&incoming.peer, &incoming.token) {
        return Err(Failure::new(
            Error::Authentication,
            DropReason::StaleChannel,
        ));
    }
    let packet = (incoming.body.first() != Some(&super::neighbor::NAMESPACE))
        .then(|| Packet::decode(&incoming.body))
        .transpose()?;
    // Keep the same Packet -> Socket lock order as the packet/carrier write
    // polls. They are independent owners; never nest these fences in reverse.
    let _generation = if packet
        .as_ref()
        .is_some_and(|packet| packet.phase == Phase::Packet || packet.phase.recovery())
    {
        Some(incoming.generation.enter()?)
    } else {
        None
    };
    // Hold originating carrier admission through all synchronous forwarding,
    // endpoint/cache mutation and final delivery. Retirement waits for admitted
    // dispatches; queued old ingress cannot enter after rebind completes.
    let _socket_generation = incoming
        .socket_generation
        .as_ref()
        .map(|generation| generation.enter())
        .transpose()?;
    if !inner.active(&incoming.peer, &incoming.token) {
        return Err(Failure::new(
            Error::Authentication,
            DropReason::StaleChannel,
        ));
    }
    let Some(packet) = packet else {
        return super::neighbor::process(inner, incoming);
    };
    let position = inner.authorize(&packet.path)?;
    let previous = if packet.phase.forward() {
        position.checked_sub(1)
    } else {
        position.checked_add(1)
    }
    .and_then(|index| packet.path.get(index))
    .ok_or(Error::UnauthorizedPath)?;
    if previous != &incoming.peer {
        return Err(Error::UnauthorizedPath.into());
    }
    let next = if packet.phase.forward() {
        position.checked_add(1)
    } else {
        position.checked_sub(1)
    }
    .and_then(|index| packet.path.get(index))
    .cloned();
    if let Some(next) = next {
        #[cfg(test)]
        {
            let mut observed = lock(&inner.observed_transit);
            if observed.len() < 16 {
                observed.push(packet.body.clone());
            }
        }
        inner.send_classified(&next, packet)?;
        // Attributed to ingress transport: successfully enqueued for the exact next hop.
        inner
            .counters(incoming.transport)
            .forwarded
            .fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }
    if packet.phase.recovery() {
        let previous = previous.clone();
        return super::recovery::process(inner, packet, &previous);
    }
    match packet.phase {
        Phase::Init => {
            let origin = packet.path.first().ok_or(Error::Protocol)?;
            let key = inner.keys.get(origin).ok_or(Error::Authentication)?;
            let session_key = (origin.clone(), packet.id);
            let mut sessions = lock(&inner.endpoints);
            // Admission must see current occupancy, not wait for the cleanup tick.
            sessions.retain(|_, session| session.expires > Instant::now());
            if let Some(session) = sessions.get(&session_key) {
                if session.path == packet.path
                    && let Some((init, ready)) = &session.retry
                    && init == &packet.body
                {
                    // Exact authenticated Init retransmission: replay the same
                    // handshake response, never replace a cipher, reset replay
                    // state, promote a key, or extend its original expiration.
                    return inner.send_classified(
                        previous,
                        Packet {
                            phase: Phase::Ready,
                            id: packet.id,
                            path: packet.path.clone(),
                            body: ready.clone(),
                        },
                    );
                }
                return Err(Error::Capacity.into());
            }
            let mut state = inner.identity.handshake(None, &packet.e2e_prologue())?;
            let mode = read_handshake(&mut state, &packet.body)?;
            let packets = match mode.as_slice() {
                b"echo-v1" => false,
                b"packet-v1" => true,
                _ => return Err(Error::Protocol.into()),
            };
            if state.get_remote_static() != Some(key.as_slice()) {
                return Err(Error::Authentication.into());
            }
            let limit = inner.config.limits.max_pending_probes;
            if packets {
                // Reserve at most L exact (origin, ordered-path) contexts, each with
                // one active key and one pending replacement. A replayed Init can
                // occupy only the pending slot; it never retires the active key.
                let contexts: std::collections::BTreeSet<_> = sessions
                    .values()
                    .filter(|session| session.packets)
                    .map(|session| &session.path)
                    .collect();
                let existing = contexts.contains(&packet.path);
                if (!existing && contexts.len() >= limit)
                    || sessions.values().any(|session| {
                        session.packets && session.path == packet.path && !session.confirmed
                    })
                {
                    return Err(Error::Capacity.into());
                }
            } else if sessions.values().filter(|session| !session.packets).count() >= limit {
                // Probe setup has its own bounded pool, so it cannot consume packet
                // rollover headroom (or vice versa). Total receiving state <= 3L.
                return Err(Error::Capacity.into());
            }
            let body = write_handshake(&mut state, &mode)?;
            let retry = packets.then(|| (packet.body.clone(), body.clone()));
            let cipher = Cipher::finish(state)?;
            inner.send_classified(
                previous,
                Packet {
                    phase: Phase::Ready,
                    id: packet.id,
                    path: packet.path.clone(),
                    body,
                },
            )?;
            let lifetime = if packets {
                // Ready lets the sender cache this key for 59s from Init start.
                // Keep the unconfirmed key for the matching 60s budget, so a lost
                // first application packet does not strand that valid sender cache.
                super::packet::SESSION_LIFETIME
            } else {
                Duration::from_millis(inner.config.limits.probe_timeout_ms)
            };
            sessions.insert(
                session_key,
                EndpointSession {
                    path: packet.path,
                    cipher,
                    packets,
                    confirmed: false,
                    expires: Instant::now() + lifetime,
                    retry,
                },
            );
            Ok(())
        }
        Phase::Request | Phase::Packet => {
            let origin = packet.path.first().ok_or(Error::Protocol)?;
            let mut sessions = lock(&inner.endpoints);
            let session_key = (origin.clone(), packet.id);
            if packet.phase == Phase::Packet
                && sessions
                    .get(&session_key)
                    .is_none_or(|session| session.expires <= Instant::now())
            {
                drop(sessions);
                super::recovery::unknown(inner, &packet, previous);
                return Err(Error::Authentication.into());
            }
            let session = sessions
                .get_mut(&session_key)
                .ok_or(Error::Authentication)?;
            if session.path != packet.path
                || session.expires <= Instant::now()
                || session.packets != (packet.phase == Phase::Packet)
            {
                return Err(Error::Authentication.into());
            }
            let plaintext = Zeroizing::new(session.cipher.open(&packet.body)?);
            if plaintext.len() > super::wire::MAX_PAYLOAD {
                return Err(Error::Oversize.into());
            }
            if session.packets {
                let promote = !session.confirmed;
                if promote {
                    // Only the first authenticated transport message proves possession
                    // of the fresh E2E key. Retire this exact context's old key now.
                    session.confirmed = true;
                    session.retry = None;
                    session.expires = Instant::now() + super::packet::SESSION_LIFETIME;
                    sessions.retain(|(source, id), session| {
                        source != origin
                            || *id == packet.id
                            || !session.packets
                            || session.path != packet.path
                    });
                }
                let delivery = lock(&inner.delivery);
                let delivery = delivery.as_ref().ok_or_else(|| {
                    Failure::new(Error::Closed, DropReason::EndpointDeliveryClosed)
                })?;
                return match delivery.try_send(super::ReceivedPacket {
                    source: origin.clone(),
                    path: packet.path,
                    payload: plaintext.to_vec(),
                }) {
                    Ok(()) => {
                        inner.delivery_high_water.enqueued(delivery);
                        Ok(())
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        inner.delivery_high_water.observe(delivery.max_capacity());
                        Err(Failure::new(
                            Error::Capacity,
                            DropReason::EndpointDeliveryFull,
                        ))
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => Err(Failure::new(
                        Error::Capacity,
                        DropReason::EndpointDeliveryClosed,
                    )),
                };
            }
            let body = session.cipher.seal(&plaintext)?;
            inner.send_classified(
                previous,
                Packet {
                    phase: Phase::Echo,
                    id: packet.id,
                    path: packet.path.clone(),
                    body,
                },
            )?;
            sessions.remove(&(origin.clone(), packet.id));
            Ok(())
        }
        Phase::Ready | Phase::Echo => {
            let pending = lock(&inner.pending);
            let waiting = pending.get(&packet.id).ok_or(Error::Authentication)?;
            if waiting.path != packet.path {
                return Err(Error::UnauthorizedPath.into());
            }
            waiting
                .tx
                .try_send(packet)
                .map_err(|_| Error::Capacity.into())
        }
        _ => Err(Error::Protocol.into()),
    }
}
