use super::{Inner, Packet, Phase, SendingSession, WaitingProbe, lock, probe::PendingGuard};
use crate::{
    Batch, Error, MAX_BATCH_BYTES, MAX_BATCH_FRAMES, Result,
    crypto::{Cipher, read_handshake, write_handshake},
};
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::Instant};
use zeroize::Zeroizing;

pub(super) const SESSION_LIFETIME: Duration = Duration::from_secs(60);

pub(super) async fn send(
    inner: Arc<Inner>,
    path: Vec<String>,
    payloads: Vec<Vec<u8>>,
) -> Result<()> {
    if payloads.is_empty()
        || payloads.len() > MAX_BATCH_FRAMES
        || payloads
            .iter()
            .any(|payload| payload.len() > super::MAX_PACKET_PAYLOAD)
        || payloads.iter().map(Vec::len).sum::<usize>() > MAX_BATCH_BYTES
    {
        return Err(Error::Oversize);
    }
    let payloads: Vec<_> = payloads.into_iter().map(Zeroizing::new).collect();
    let generation = inner.packet_generation();
    if inner.authorize(&path)? != 0 {
        return Err(Error::UnauthorizedPath);
    }
    let mut shutdown = inner.shutdown.subscribe();
    if *shutdown.borrow() {
        return Err(Error::Closed);
    }
    let operation = async {
        // The cache lock still owns nonce assignment through enqueue, preserving
        // same-key order. Only a missing path takes its own setup permit; no
        // network await holds the cache lock, so cached paths keep progressing.
        let mut sessions = inner.sending.lock().await;
        let now = Instant::now();
        sessions.retain(|_, session| session.expires > now);
        if !sessions.contains_key(&path) {
            drop(sessions);
            let _setup = inner
                .packet_setup
                .get(&path)
                .ok_or(Error::UnauthorizedPath)?
                .lock()
                .await;
            sessions = inner.sending.lock().await;
            sessions.retain(|_, session| session.expires > Instant::now());
            // A preceding caller may have established this exact path while we
            // waited. Cancellation releases the permit and PendingGuard without
            // spawning work that could later publish a session.
            if !sessions.contains_key(&path) {
                if sessions.len() >= inner.config.limits.max_pending_probes {
                    return Err(Error::Capacity);
                }
                drop(sessions);
                let session = establish(&inner, &path).await?;
                sessions = inner.sending.lock().await;
                if *inner.shutdown.borrow() {
                    return Err(Error::Closed);
                }
                sessions.retain(|_, session| session.expires > Instant::now());
                // Other paths can complete during setup. Recheck rather than
                // increasing the bounded cipher cache under concurrent setup.
                if sessions.len() >= inner.config.limits.max_pending_probes {
                    return Err(Error::Capacity);
                }
                sessions.insert(path.clone(), session);
            }
        }
        if *inner.shutdown.borrow() || generation.retired() {
            return Err(Error::Closed);
        }
        let session = sessions.get_mut(&path).ok_or(Error::Protocol)?;
        let mut frames = Vec::with_capacity(payloads.len());
        for payload in payloads {
            let body = session.cipher.seal(&payload)?;
            frames.push(
                Packet {
                    phase: Phase::Packet,
                    id: session.id,
                    path: path.clone(),
                    body,
                }
                .encode()?,
            );
        }
        let channel = lock(&inner.peers)
            .get(&path[1])
            .cloned()
            .ok_or(Error::Unavailable)?;
        let batch = Batch::new(frames)?;
        inner
            .enqueue_scoped(&channel, batch, Some(generation.clone()))
            .map_err(|failure| inner.local_send_failure(failure))
    };
    tokio::select! {
        biased;
        _ = shutdown.changed() => Err(Error::Closed),
        result = tokio::time::timeout(Duration::from_millis(inner.config.limits.probe_timeout_ms), operation) => result.map_err(|_| Error::Timeout)?,
    }
}

async fn establish(inner: &Arc<Inner>, path: &[String]) -> Result<SendingSession> {
    // Anchor the lifetime before emitting Init. A delayed Ready must consume,
    // rather than restart, this budget; receiver pending retention is 60s.
    let expires = Instant::now() + SESSION_LIFETIME - Duration::from_secs(1);
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
                path: path.to_vec(),
                tx,
            },
        );
    }
    let _guard = PendingGuard {
        inner: inner.clone(),
        id,
    };
    let mut first = Packet {
        phase: Phase::Init,
        id,
        path: path.to_vec(),
        body: Vec::new(),
    };
    let mut state = inner
        .identity
        .handshake(Some(remote), &first.e2e_prologue())?;
    first.body = write_handshake(&mut state, b"packet-v1")?;
    inner.send(&path[1], first)?;
    let ready = rx.recv().await.ok_or(Error::Closed)?;
    if ready.phase != Phase::Ready
        || read_handshake(&mut state, &ready.body)? != b"packet-v1"
        || state.get_remote_static() != Some(remote.as_slice())
    {
        return Err(Error::Authentication);
    }
    if Instant::now() >= expires {
        return Err(Error::Timeout);
    }
    Ok(SendingSession {
        id,
        cipher: Cipher::finish(state)?,
        expires,
    })
}
