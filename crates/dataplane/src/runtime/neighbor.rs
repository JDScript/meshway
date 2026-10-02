//! Fixed-size hop control, separate from E2E packets and path authorization.
use super::{Incoming, Inner, NeighborProbeObservation, StageResult, lock};
use crate::{Batch, Error, Result};
use std::{sync::Arc, time::Duration};
use tokio::{sync::oneshot, time::Instant};

// Packet envelopes start with 1. All control frames have exactly these 18 bytes:
// namespace, request/reply discriminator, unpredictable 128-bit request ID.
pub(super) const NAMESPACE: u8 = 2;
const REQUEST: u8 = 1;
const REPLY: u8 = 2;
const FRAME_LEN: usize = 18;
const REQUESTS_PER_SECOND: u8 = 8;

pub(super) struct ProbeBudget {
    since: Instant,
    remaining: u8,
}

impl Default for ProbeBudget {
    fn default() -> Self {
        Self {
            since: Instant::now(),
            remaining: REQUESTS_PER_SECOND,
        }
    }
}

impl ProbeBudget {
    fn take(&mut self) -> Result<()> {
        let now = Instant::now();
        if now.duration_since(self.since) >= Duration::from_secs(1) {
            self.since = now;
            self.remaining = REQUESTS_PER_SECOND;
        }
        if self.remaining == 0 {
            return Err(Error::Capacity);
        }
        self.remaining -= 1;
        Ok(())
    }
}

pub(super) struct Waiting {
    peer: String,
    token: [u8; 32],
    deadline: Instant,
    tx: oneshot::Sender<Instant>,
}

struct Guard {
    inner: Arc<Inner>,
    id: [u8; 16],
}

impl Drop for Guard {
    fn drop(&mut self) {
        lock(&self.inner.neighbor_pending).remove(&self.id);
    }
}

fn frame(kind: u8, id: &[u8; 16]) -> Result<Batch> {
    let mut body = Vec::with_capacity(FRAME_LEN);
    body.extend_from_slice(&[NAMESPACE, kind]);
    body.extend_from_slice(id);
    Batch::new(vec![body])
}

pub(super) async fn probe_observed(
    inner: Arc<Inner>,
    peer: &str,
    timeout: Duration,
) -> Result<NeighborProbeObservation> {
    if timeout.is_zero() || timeout > Duration::from_secs(60) {
        return Err(Error::InvalidConfig("invalid neighbor probe timeout"));
    }
    let mut shutdown = inner.shutdown.subscribe();
    if *shutdown.borrow() {
        return Err(Error::Closed);
    }
    let channel = lock(&inner.peers)
        .get(peer)
        .cloned()
        .ok_or(Error::Unavailable)?;
    // Metadata and private token come from this one immutable Channel clone.
    // Never fetch metadata from the registry again after receiving the reply.
    let snapshot = channel.snapshot(peer);
    let mut closed = channel.close.subscribe();
    let started = Instant::now();
    let deadline = started + timeout;
    let mut id = [0; 16];
    getrandom::fill(&mut id).map_err(|_| Error::Authentication)?;
    let (tx, mut rx) = oneshot::channel();
    {
        let mut pending = lock(&inner.neighbor_pending);
        if pending.len() >= inner.config.limits.max_pending_probes
            || pending.contains_key(&id)
            || pending
                .values()
                .any(|entry| entry.peer == peer && entry.token == snapshot.token)
        {
            return Err(Error::Capacity);
        }
        pending.insert(
            id,
            Waiting {
                peer: peer.into(),
                token: snapshot.token,
                deadline,
                tx,
            },
        );
    }
    let _guard = Guard {
        inner: inner.clone(),
        id,
    };
    if *closed.borrow() || !inner.active(peer, &snapshot.token) {
        return Err(Error::Unavailable);
    }
    inner
        .enqueue(&channel, frame(REQUEST, &id)?)
        .map_err(|failure| inner.local_send_failure(failure))?;
    let retry_interval = timeout / 3;
    let mut retry_at = started + retry_interval;
    let mut retries = 0;
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => return Err(Error::Closed),
            _ = closed.changed() => return Err(Error::Unavailable),
            _ = tokio::time::sleep_until(deadline) => {
                // unregister compares the full private channel token under the registry
                // lock and closes the selected engine; a replacement is never removed.
                inner.unregister(peer, &snapshot.token);
                return Err(Error::Timeout);
            }
            result = &mut rx => {
                let received = result.map_err(|_| Error::Closed)?;
                if received >= deadline || Instant::now() >= deadline {
                    inner.unregister(peer, &snapshot.token);
                    return Err(Error::Timeout);
                }
                if !inner.active(peer, &snapshot.token) {
                    return Err(Error::Unavailable);
                }
                return Ok(NeighborProbeObservation {
                    connection: snapshot.connection,
                    round_trip: received.duration_since(started),
                });
            }
            _ = tokio::time::sleep_until(retry_at), if retries < 2 => {
                retries += 1;
                retry_at += retry_interval;
                if !inner.active(peer, &snapshot.token) {
                    return Err(Error::Unavailable);
                }
                // Retransmit the same unpredictable request on the exact channel.
                // Its writer still assigns fresh hop nonces. One pending slot,
                // responder's existing 8/s budget, and the original deadline remain.
                // A full local queue must not discard an already in-flight reply.
                if let Err(failure) = inner.enqueue(&channel, frame(REQUEST, &id)?) {
                    let error = inner.local_send_failure(failure);
                    if error != Error::Capacity { return Err(error); }
                }
            }
        }
    }
}

pub(super) fn process(inner: &Inner, incoming: &Incoming) -> StageResult<()> {
    if incoming.body.len() != FRAME_LEN || incoming.body[0] != NAMESPACE {
        return Err(Error::Protocol.into());
    }
    let id: [u8; 16] = incoming.body[2..].try_into().map_err(|_| Error::Protocol)?;
    let channel = lock(&inner.peers)
        .get(&incoming.peer)
        .filter(|channel| channel.token == incoming.token)
        .cloned()
        .ok_or(Error::Authentication)?;
    match incoming.body[1] {
        REQUEST => {
            lock(&channel.probe_budget).take()?;
            // Echo on the exact authenticated channel, never a newly selected carrier.
            inner.enqueue(&channel, frame(REPLY, &id)?)
        }
        REPLY => {
            let mut pending = lock(&inner.neighbor_pending);
            let waiting = pending.get(&id).ok_or(Error::Authentication)?;
            let now = Instant::now();
            if waiting.peer != incoming.peer
                || waiting.token != incoming.token
                || now >= waiting.deadline
            {
                return Err(Error::Authentication.into());
            }
            let waiting = pending.remove(&id).ok_or(Error::Authentication)?;
            waiting.tx.send(now).map_err(|_| Error::Closed)?;
            Ok(())
        }
        _ => Err(Error::Protocol.into()),
    }
}
