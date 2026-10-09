//! On-demand adjacency for netmap authority. A peer worker acquires only while
//! its peer is interested (or its edge is keepalive); a selected channel with
//! no interest and no frames for `Limits::idle` is retired. Publications keep
//! every configured peer resident.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// Local need for an adjacency, registered by the packet owner when it selects
/// a path whose next hop is `peer` (first packet) or relays a handshake toward
/// it. Never an authorization: only peers the installed netmap admits are
/// acquired, and interest expires `Limits::idle` after the last call.
///
/// The packet owner defines the final trait; this is its expected shape.
pub trait AdjacencyInterest: Send + Sync {
    fn interest(&self, peer: NodeId);
}

struct Entry {
    last: u64,
    wake: Arc<tokio::sync::Notify>,
}

/// Cross-epoch interest and wake-up table, bounded by the peers of the
/// current plan.
pub(super) struct Interests {
    base: std::time::Instant,
    peers: Mutex<BTreeMap<NodeId, Entry>>,
    allowed: Mutex<std::collections::BTreeSet<NodeId>>,
}

impl Default for Interests {
    fn default() -> Self {
        Self::new()
    }
}

impl Interests {
    pub(super) fn new() -> Self {
        Self {
            base: std::time::Instant::now(),
            peers: Mutex::default(),
            allowed: Mutex::default(),
        }
    }

    pub(super) fn now_ms(&self) -> u64 {
        u64::try_from(self.base.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Restrict to the peers of a newly installed plan; others are forgotten.
    pub(super) fn allow(&self, plan: &Plan) {
        let allowed: std::collections::BTreeSet<_> = plan.peers().collect();
        if let Ok(mut peers) = self.peers.lock() {
            peers.retain(|peer, entry| {
                let keep = allowed.contains(peer);
                if !keep {
                    entry.wake.notify_waiters();
                }
                keep
            });
        }
        if let Ok(mut current) = self.allowed.lock() {
            *current = allowed;
        }
    }

    pub(super) fn touch(&self, peer: NodeId) {
        if !self
            .allowed
            .lock()
            .is_ok_and(|allowed| allowed.contains(&peer))
        {
            return;
        }
        let now = self.now_ms();
        let Ok(mut peers) = self.peers.lock() else {
            return;
        };
        let entry = peers.entry(peer).or_insert_with(|| Entry {
            last: 0,
            wake: Arc::default(),
        });
        entry.last = now.max(1);
        entry.wake.notify_waiters();
    }

    /// Wake the peer's worker without registering interest (facts changed).
    pub(super) fn wake(&self, peer: NodeId) {
        if let Ok(peers) = self.peers.lock()
            && let Some(entry) = peers.get(&peer)
        {
            entry.wake.notify_waiters();
        }
    }

    pub(super) fn waker(&self, peer: NodeId) -> Arc<tokio::sync::Notify> {
        let Ok(mut peers) = self.peers.lock() else {
            return Arc::default();
        };
        peers
            .entry(peer)
            .or_insert_with(|| Entry {
                last: 0,
                wake: Arc::default(),
            })
            .wake
            .clone()
    }

    pub(super) fn last(&self, peer: NodeId) -> u64 {
        self.peers
            .lock()
            .ok()
            .and_then(|peers| peers.get(&peer).map(|entry| entry.last))
            .unwrap_or(0)
    }

    pub(super) fn recent(&self, peer: NodeId, idle: std::time::Duration) -> bool {
        let last = self.last(peer);
        last != 0 && self.now_ms().saturating_sub(last) < idle.as_millis() as u64
    }
}

/// Activity of one selected channel: the last frame sent or received.
#[derive(Default)]
pub(super) struct Activity(AtomicU64);
impl Activity {
    pub(super) fn record(&self, interests: &Interests) {
        self.0.store(interests.now_ms().max(1), Ordering::Relaxed);
    }
    fn last(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Cloneable handle for the packet owner; see [`AdjacencyInterest`].
#[derive(Clone)]
pub struct Interest(pub(super) Arc<Interests>);
impl AdjacencyInterest for Interest {
    fn interest(&self, peer: NodeId) {
        self.0.touch(peer);
    }
}

impl EpochContext {
    /// Whether the peer worker should hold or acquire an adjacency now.
    pub(super) fn wanted(&self, peer: NodeId) -> bool {
        !self.plan.on_demand()
            || self.plan.keepalive(peer)
            || self.interests.recent(peer, self.limits.idle)
    }
}

/// Retire selected channels that are neither keepalive, interested nor used.
pub(super) async fn idle(shared: Arc<EpochContext>) {
    let period = (shared.limits.idle / 4).clamp(
        std::time::Duration::from_millis(25),
        std::time::Duration::from_secs(5),
    );
    let idle = u64::try_from(shared.limits.idle.as_millis()).unwrap_or(u64::MAX);
    loop {
        tokio::select! { biased;
            _ = shared.stop.cancelled() => return,
            _ = tokio::time::sleep(period) => {},
        }
        let epoch = shared.epochs.borrow().clone();
        if !epoch.plan.on_demand() {
            continue;
        }
        let now = shared.interests.now_ms();
        let Ok(mut links) = shared.links.lock() else {
            return;
        };
        let expired: Vec<_> = links
            .iter()
            .filter(|(peer, link)| {
                !epoch.plan.keepalive(**peer)
                    && now.saturating_sub(link.activity.last().max(shared.interests.last(**peer)))
                        >= idle
            })
            .map(|(peer, _)| *peer)
            .collect();
        for peer in expired {
            if let Some(link) = links.remove(&peer) {
                link.scope.cancellation.cancel();
                diagnostic_link(&shared, &link.fact, &link.scope, "retire", "idle", None);
            }
        }
        drop(links);
        let _ = shared.control_events.try_send(Event::Changed);
    }
}
