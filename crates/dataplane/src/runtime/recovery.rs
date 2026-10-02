//! Unknown-session feedback is a hint, never a cache reset. A fresh, pinned
//! Noise IK exchange queries one exact packet session on its original path.
use super::{Inner, Packet, Phase, StageResult, WaitingProbe, lock, probe::PendingGuard};
use crate::{
    Error, Result,
    crypto::{Cipher, read_handshake, write_handshake},
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::Instant};

const MODE: &[u8] = b"resync-v1";
const HINT: &[u8] = b"unknown-session-v1";
const COOLDOWN: Duration = Duration::from_secs(1);
const BURST: usize = 8;

struct PathState {
    active: Option<[u8; 16]>,
    hinted: bool,
    query_after: Instant,
    hint_after: Instant,
    init_after: Instant,
}
struct Receiving {
    path: Vec<String>,
    cipher: Cipher,
    expires: Instant,
}
pub(super) struct Control {
    // Allocated only from configured paths, never from untrusted session IDs.
    paths: BTreeMap<Vec<String>, PathState>,
    receiving: BTreeMap<(String, [u8; 16]), Receiving>,
    remaining: usize,
    refill: Instant,
}
impl Control {
    pub(super) fn new(paths: &[Vec<String>]) -> Self {
        let now = Instant::now();
        Self {
            paths: paths
                .iter()
                .map(|path| {
                    (
                        path.clone(),
                        PathState {
                            active: None,
                            hinted: false,
                            query_after: now,
                            hint_after: now,
                            init_after: now,
                        },
                    )
                })
                .collect(),
            receiving: BTreeMap::new(),
            remaining: BURST,
            refill: now + COOLDOWN,
        }
    }
    pub(super) fn cleanup(&mut self, now: Instant) {
        self.receiving.retain(|_, session| session.expires > now);
    }
    pub(super) fn clear(&mut self) {
        self.receiving.clear();
        for state in self.paths.values_mut() {
            state.active = None;
            state.hinted = false;
        }
    }
    fn budget(&mut self, now: Instant) -> bool {
        if now >= self.refill {
            self.remaining = BURST;
            self.refill = now + COOLDOWN;
        }
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        true
    }
    pub(super) fn installed(&mut self, path: &[String], id: [u8; 16]) {
        if let Some(state) = self.paths.get_mut(path) {
            state.active = Some(id);
            state.hinted = false;
        }
    }
    pub(super) fn hinted(&self, path: &[String], id: [u8; 16]) -> bool {
        self.paths.get(path).is_some_and(|state| {
            state.active == Some(id) && state.hinted && Instant::now() >= state.query_after
        })
    }
    pub(super) fn requested(&mut self, path: &[String], id: [u8; 16]) -> bool {
        let now = Instant::now();
        let eligible = self.paths.get(path).is_some_and(|state| {
            state.active == Some(id) && state.hinted && now >= state.query_after
        });
        if !eligible || !self.budget(now) {
            return false;
        }
        let state = self.paths.get_mut(path).expect("configured path");
        state.hinted = false;
        state.query_after = now + COOLDOWN;
        true
    }
}

pub(super) fn unknown(inner: &Inner, packet: &Packet, previous: &str) {
    let mut control = lock(&inner.recovery);
    let now = Instant::now();
    if !control
        .paths
        .get(&packet.path)
        .is_some_and(|state| now >= state.hint_after)
        || !control.budget(now)
    {
        return;
    }
    control
        .paths
        .get_mut(&packet.path)
        .expect("configured path")
        .hint_after = now + COOLDOWN;
    // Best effort: original authentication drop is still reported; no payload
    // is retained and no recovery task is spawned by the receive dispatcher.
    let _ = inner.send_classified(
        previous,
        Packet {
            phase: Phase::UnknownSession,
            id: packet.id,
            path: packet.path.clone(),
            body: HINT.to_vec(),
        },
    );
}

pub(super) fn process(inner: &Inner, packet: Packet, previous: &str) -> StageResult<()> {
    match packet.phase {
        Phase::UnknownSession => {
            if packet.body != HINT {
                return Err(Error::Protocol.into());
            }
            let mut control = lock(&inner.recovery);
            let state = control
                .paths
                .get_mut(&packet.path)
                .ok_or(Error::UnauthorizedPath)?;
            if state.active == Some(packet.id) {
                state.hinted = true;
            }
            Ok(())
        }
        Phase::ResyncInit => {
            let origin = packet.path.first().ok_or(Error::Protocol)?;
            let remote = inner.keys.get(origin).ok_or(Error::Authentication)?;
            let mut control = lock(&inner.recovery);
            let now = Instant::now();
            control.cleanup(now);
            if control.receiving.len() >= inner.config.limits.max_pending_probes
                || control.receiving.contains_key(&(origin.clone(), packet.id))
                || !control
                    .paths
                    .get(&packet.path)
                    .is_some_and(|state| now >= state.init_after)
                || !control.budget(now)
            {
                return Err(Error::Capacity.into());
            }
            control
                .paths
                .get_mut(&packet.path)
                .ok_or(Error::UnauthorizedPath)?
                .init_after = now + COOLDOWN;
            let mut state = inner.identity.handshake(None, &packet.e2e_prologue())?;
            if read_handshake(&mut state, &packet.body)? != MODE
                || state.get_remote_static() != Some(remote.as_slice())
            {
                return Err(Error::Authentication.into());
            }
            let body = write_handshake(&mut state, MODE)?;
            let cipher = Cipher::finish(state)?;
            inner.send_classified(
                previous,
                Packet {
                    phase: Phase::ResyncReady,
                    id: packet.id,
                    path: packet.path.clone(),
                    body,
                },
            )?;
            control.receiving.insert(
                (origin.clone(), packet.id),
                Receiving {
                    path: packet.path,
                    cipher,
                    expires: now + Duration::from_millis(inner.config.limits.probe_timeout_ms),
                },
            );
            Ok(())
        }
        Phase::ResyncQuery => {
            let origin = packet.path.first().ok_or(Error::Protocol)?;
            let mut control = lock(&inner.recovery);
            let key = (origin.clone(), packet.id);
            let session = control
                .receiving
                .get_mut(&key)
                .ok_or(Error::Authentication)?;
            if session.path != packet.path || session.expires <= Instant::now() {
                return Err(Error::Authentication.into());
            }
            let query = session.cipher.open(&packet.body)?;
            if query.len() != 80 {
                return Err(Error::Protocol.into());
            }
            let old_id: [u8; 16] = query[..16].try_into().map_err(|_| Error::Protocol)?;
            // This answer is only produced after initiator key confirmation.
            // An old/replayed Init alone never inspects or mutates active keys.
            let exists = lock(&inner.endpoints)
                .get(&(origin.clone(), old_id))
                .is_some_and(|old| {
                    old.packets
                        && old.path == packet.path
                        && old.expires > Instant::now()
                        && old.cipher.id.as_slice() == &query[16..48]
                });
            let mut answer = query;
            answer.push(u8::from(exists));
            let body = session.cipher.seal(&answer)?;
            inner.send_classified(
                previous,
                Packet {
                    phase: Phase::ResyncAnswer,
                    id: packet.id,
                    path: packet.path.clone(),
                    body,
                },
            )?;
            control.receiving.remove(&key);
            Ok(())
        }
        Phase::ResyncReady | Phase::ResyncAnswer => {
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

// Invoked only by a normal caller while holding that path's setup permit. The
// caller's deadline and packet fence enclose this future; cancellation removes
// pending admission and cannot publish a reset or a late application payload.
pub(super) async fn missing(
    inner: &Arc<Inner>,
    path: &[String],
    old_id: [u8; 16],
    old_key: [u8; 32],
) -> Result<bool> {
    let remote = inner
        .keys
        .get(path.last().ok_or(Error::UnauthorizedPath)?)
        .ok_or(Error::Authentication)?;
    let mut id = [0; 16];
    let mut challenge = [0; 32];
    getrandom::fill(&mut id).map_err(|_| Error::Authentication)?;
    getrandom::fill(&mut challenge).map_err(|_| Error::Authentication)?;
    let mut first = Packet {
        phase: Phase::ResyncInit,
        id,
        path: path.to_vec(),
        body: Vec::new(),
    };
    let mut state = inner
        .identity
        .handshake(Some(remote), &first.e2e_prologue())?;
    first.body = write_handshake(&mut state, MODE)?;
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
    inner.send(&path[1], first)?;
    let ready = rx.recv().await.ok_or(Error::Closed)?;
    if ready.phase != Phase::ResyncReady
        || read_handshake(&mut state, &ready.body)? != MODE
        || state.get_remote_static() != Some(remote.as_slice())
    {
        return Err(Error::Authentication);
    }
    let mut cipher = Cipher::finish(state)?;
    let mut query = old_id.to_vec();
    // Bind the exact cipher's public Noise transcript hash. Replayed old Init
    // can recreate the packet ID after restart, but with a different key.
    query.extend_from_slice(&old_key);
    query.extend_from_slice(&challenge);
    inner.send(
        &path[1],
        Packet {
            phase: Phase::ResyncQuery,
            id,
            path: path.to_vec(),
            body: cipher.seal(&query)?,
        },
    )?;
    let answer = rx.recv().await.ok_or(Error::Closed)?;
    if answer.phase != Phase::ResyncAnswer {
        return Err(Error::Authentication);
    }
    let response = cipher.open(&answer.body)?;
    if response.len() != 81 || response[..80] != query || response[80] > 1 {
        return Err(Error::Authentication);
    }
    Ok(response[80] == 0)
}

#[cfg(test)]
mod tests {
    use super::super::{Channel, Runtime, StreamContext, neighbor};
    use super::*;
    use crate::{Identity, TransportKind};
    use std::sync::Mutex;

    #[tokio::test(start_paused = true)]
    async fn fixed_path_state_and_global_budget_bound_random_hints_and_queries() {
        let paths: Vec<_> = (0..32)
            .map(|n| vec!["a".to_owned(), format!("b{n}")])
            .collect();
        let mut control = Control::new(&paths);
        for path in &paths {
            control.installed(path, [1; 16]);
            let state = control.paths.get_mut(path).unwrap();
            state.hinted = true;
        }
        assert_eq!(
            paths
                .iter()
                .filter(|path| control.requested(path, [1; 16]))
                .count(),
            BURST
        );
        assert!(paths.iter().all(|path| !control.requested(path, [1; 16])));
        for n in 0..1000u128 {
            assert!(!control.requested(&paths[0], n.to_be_bytes()));
            assert!(!control.requested(&["random".into(), format!("id{n}")], [1; 16]));
        }
        assert_eq!(control.paths.len(), paths.len());
        tokio::time::advance(COOLDOWN).await;
        assert_eq!(
            paths
                .iter()
                .filter(|path| control.requested(path, [1; 16]))
                .count(),
            BURST
        );
    }

    #[tokio::test]
    async fn missing_session_hint_is_path_limited_and_random_ids_allocate_no_state() {
        let configs = super::super::tests::configs();
        let a = super::super::Runtime::start(configs[0].clone())
            .await
            .unwrap();
        let b = super::super::Runtime::start(configs[2].clone())
            .await
            .unwrap();
        // No network peer is needed: failed enqueue still consumes the same
        // emission budget, and cannot turn random session IDs into table keys.
        for n in 0..1000u128 {
            unknown(
                &b.handle.inner,
                &Packet {
                    phase: Phase::Packet,
                    id: n.to_be_bytes(),
                    path: configs[0].allowed_paths[0].clone(),
                    body: vec![0; 24],
                },
                "lax",
            );
        }
        {
            let control = lock(&b.handle.inner.recovery);
            assert_eq!(control.paths.len(), 2);
            assert_eq!(control.remaining, BURST - 1);
            assert!(control.receiving.is_empty());
        }
        a.shutdown().await;
        b.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn recipient_control_capacity_and_expiry_do_not_consume_packet_pool() {
        let mut configs = super::super::tests::configs();
        let direct = vec!["nyc".into(), "sha".into()];
        configs[2].allowed_paths.push(direct.clone());
        configs[2].limits.max_pending_probes = 1;
        let b = Runtime::start(configs[2].clone()).await.unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        b.handle
            .inner
            .register(
                "lax".into(),
                Channel {
                    token: [1; 32],
                    priority: 0,
                    transport: TransportKind::NativeTcp,
                    tx,
                    close: tokio::sync::watch::channel(false).0,
                    initiator: false,
                    context: Arc::new(StreamContext::default()),
                    probe_budget: Arc::new(Mutex::new(neighbor::ProbeBudget::default())),
                },
            )
            .unwrap();
        let source = Identity::from_private_key(configs[0].private_key.clone());
        let remote = super::super::public_key(&configs[2].public_key).unwrap();
        let init = |path: Vec<String>, id: [u8; 16]| {
            let mut packet = Packet {
                phase: Phase::ResyncInit,
                id,
                path,
                body: vec![],
            };
            let mut state = source
                .handshake(Some(&remote), &packet.e2e_prologue())
                .unwrap();
            packet.body = write_handshake(&mut state, MODE).unwrap();
            packet
        };
        process(
            &b.handle.inner,
            init(configs[0].allowed_paths[0].clone(), [1; 16]),
            "lax",
        )
        .map_err(|failure| failure.error)
        .unwrap();
        assert!(rx.try_recv().is_ok());
        assert_eq!(lock(&b.handle.inner.recovery).receiving.len(), 1);
        assert!(
            matches!(process(&b.handle.inner, init(direct.clone(), [2;16]), "lax"), Err(failure) if matches!(failure.error, Error::Capacity))
        );
        assert_eq!(lock(&b.handle.inner.recovery).receiving.len(), 1);
        assert!(lock(&b.handle.inner.endpoints).is_empty());
        tokio::time::advance(Duration::from_millis(
            configs[2].limits.probe_timeout_ms + 1,
        ))
        .await;
        process(&b.handle.inner, init(direct, [3; 16]), "lax")
            .map_err(|failure| failure.error)
            .unwrap();
        assert_eq!(lock(&b.handle.inner.recovery).receiving.len(), 1);
        assert!(lock(&b.handle.inner.endpoints).is_empty());
        b.shutdown().await;
    }
}
