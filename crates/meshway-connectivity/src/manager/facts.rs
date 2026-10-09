//! Peer facts from the sync long poll: selection input only. Applying them
//! never changes the authority epoch, scopes or other peers; a changed peer's
//! worker only re-reads its dial candidates.
use super::*;
use meshway_control::{FactsRejected, PeerFactsSink};
use meshway_model::NetmapRef;
use meshway_protocol::{PeerFacts, PeerFactsDelta};
use std::collections::VecDeque;

/// Deltas waiting for the Manager's facts owner. Full means rejection; Control
/// then asks for complete facts on its next sync.
const QUEUE: usize = 8;

#[derive(Default)]
pub(super) struct Store {
    queue: Mutex<VecDeque<PeerFactsDelta>>,
    ready: tokio::sync::Notify,
    peers: Mutex<BTreeMap<NodeId, PeerFacts>>,
}

impl Store {
    pub(super) fn get(&self, peer: NodeId) -> Option<PeerFacts> {
        self.peers.lock().ok()?.get(&peer).cloned()
    }

    fn push(&self, delta: PeerFactsDelta) -> Result<(), FactsRejected> {
        let mut queue = self.queue.lock().map_err(|_| FactsRejected)?;
        if queue.len() >= QUEUE {
            return Err(FactsRejected);
        }
        queue.push_back(delta);
        drop(queue);
        self.ready.notify_one();
        Ok(())
    }

    /// Apply one delta; returns the peers whose facts changed.
    fn apply(&self, delta: PeerFactsDelta) -> Vec<NodeId> {
        let Ok(mut peers) = self.peers.lock() else {
            return Vec::new();
        };
        let mut changed: Vec<NodeId> = Vec::new();
        if delta.base.is_none() {
            changed.extend(
                peers
                    .keys()
                    .filter(|peer| !delta.updated.contains_key(peer))
                    .copied(),
            );
            peers.clear();
        }
        for peer in delta.removed {
            if peers.remove(&peer).is_some() {
                changed.push(peer);
            }
        }
        for (peer, facts) in delta.updated {
            if facts.node_id != peer {
                continue;
            }
            if peers.get(&peer) != Some(&facts) {
                changed.push(peer);
                peers.insert(peer, facts);
            }
        }
        changed.sort();
        changed.dedup();
        changed
    }
}

/// [`PeerFactsSink`] for Control's sync loop. Bounded and non-blocking.
#[derive(Clone)]
pub struct FactsSink(pub(super) Arc<Store>);

impl PeerFactsSink for FactsSink {
    fn apply(&mut self, _netmap: NetmapRef, facts: PeerFactsDelta) -> Result<(), FactsRejected> {
        // Facts are scoped by the netmap Control has installed; the Manager
        // filters every peer by its own current plan when it uses them.
        self.0.push(facts)
    }
}

/// The single consumer of queued deltas.
pub(super) async fn run(shared: Arc<EpochContext>) {
    loop {
        let next = shared
            .facts
            .queue
            .lock()
            .ok()
            .and_then(|mut queue| queue.pop_front());
        let Some(delta) = next else {
            tokio::select! { biased;
                _ = shared.stop.cancelled() => return,
                _ = shared.facts.ready.notified() => {},
            }
            continue;
        };
        for peer in shared.facts.apply(delta) {
            shared.interests.wake(peer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshway_model::{Sequence, Version};
    use meshway_protocol::StateCursor;

    fn node(n: u8) -> NodeId {
        format!("00000000-0000-4000-8000-{n:012}").parse().unwrap()
    }

    fn delta(base: Option<u64>, updated: &[u8], removed: &[u8]) -> PeerFactsDelta {
        let epoch = "00000000-0000-4000-8000-000000000099".parse().unwrap();
        let cursor = |n| StateCursor {
            epoch,
            sequence: Sequence::new(n).unwrap(),
        };
        let now = chrono::DateTime::from_timestamp_millis(1_000).unwrap();
        PeerFactsDelta {
            base: base.map(cursor),
            cursor: cursor(base.unwrap_or(0) + 1),
            netmap_serial: Version::MIN,
            updated: updated
                .iter()
                .map(|n| {
                    (
                        node(*n),
                        PeerFacts {
                            node_id: node(*n),
                            listeners: BTreeMap::new(),
                            exit: None,
                            observed_at: now,
                            expires_at: now,
                        },
                    )
                })
                .collect(),
            removed: removed.iter().map(|n| node(*n)).collect(),
        }
    }

    #[test]
    fn full_queue_rejects_and_a_complete_delta_replaces_stale_peers() {
        let store = Arc::new(Store::default());
        let mut sink = FactsSink(store.clone());
        let netmap = meshway_model::NetmapRef {
            revision: meshway_model::Revision::MIN,
            serial: Version::MIN,
            digest: meshway_model::Digest::from_bytes([1; 32]),
        };
        for _ in 0..QUEUE {
            sink.apply(netmap, delta(Some(1), &[1], &[])).unwrap();
        }
        assert_eq!(
            sink.apply(netmap, delta(Some(1), &[2], &[])),
            Err(FactsRejected)
        );
        store.queue.lock().unwrap().clear();
        assert_eq!(
            store.apply(delta(None, &[1, 2], &[])),
            vec![node(1), node(2)]
        );
        assert!(
            store.apply(delta(Some(1), &[1], &[])).is_empty(),
            "unchanged"
        );
        assert_eq!(store.apply(delta(Some(2), &[], &[2])), vec![node(2)]);
        // After a rejection Control sends complete facts: absent peers go.
        sink.apply(netmap, delta(None, &[3], &[])).unwrap();
        let full = store.queue.lock().unwrap().pop_front().unwrap();
        assert_eq!(store.apply(full), vec![node(1), node(3)]);
        assert!(store.get(node(1)).is_none() && store.get(node(3)).is_some());
    }
}
