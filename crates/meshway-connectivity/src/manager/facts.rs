//! Peer facts: selection input only. Control owns the last known facts of
//! the held netmap and publishes them ([`KnownPeers`]); the Manager reads
//! that published value and keeps no ledger of its own. A changed peer's
//! worker only re-reads its dial candidates; nothing is reconfigured.
use super::*;
use meshway_control::KnownPeers;
use meshway_protocol::PeerFacts;
use tokio::sync::watch;

/// The published facts the Manager reads; survives Manager replacement.
#[derive(Default)]
pub(super) struct Store {
    source: Mutex<Option<watch::Receiver<Option<KnownPeers>>>>,
    attached: tokio::sync::Notify,
}

impl Store {
    /// The last known facts of `peer`, as Control last published them.
    pub(super) fn get(&self, peer: NodeId) -> Option<PeerFacts> {
        let source = self.source.lock().ok()?;
        let known = source.as_ref()?.borrow();
        known.as_ref()?.peers.get(&peer).cloned()
    }

    /// Whether `peer` holds a live traversal request toward this Node.
    pub(super) fn asked(&self, peer: NodeId, now_ms: u64) -> bool {
        let Ok(source) = self.source.lock() else {
            return false;
        };
        let Some(source) = source.as_ref() else {
            return false;
        };
        asked(source.borrow().as_ref(), now_ms).contains(&peer)
    }

    fn attach(&self, source: watch::Receiver<Option<KnownPeers>>) {
        if let Ok(mut current) = self.source.lock() {
            *current = Some(source);
        }
        self.attached.notify_one();
    }

    fn source(&self) -> Option<watch::Receiver<Option<KnownPeers>>> {
        self.source.lock().ok()?.clone()
    }
}

impl Manager {
    /// Read peer facts from Control's publication.
    pub fn attach_facts(&self, source: watch::Receiver<Option<KnownPeers>>) {
        self.shared.facts.attach(source);
    }
}

/// Peers whose facts differ between two publications.
fn changed(before: Option<&KnownPeers>, after: Option<&KnownPeers>) -> Vec<NodeId> {
    let empty = BTreeMap::new();
    let before = before.map_or(&empty, |known| &*known.peers);
    let after = after.map_or(&empty, |known| &*known.peers);
    before
        .keys()
        .chain(after.keys())
        .filter(|peer| before.get(peer) != after.get(peer))
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Peers whose live traversal requests ask this Node to accept.
fn asked(known: Option<&KnownPeers>, now_ms: u64) -> Vec<NodeId> {
    known.map_or_else(Vec::new, |known| {
        known
            .traversal_requests
            .iter()
            .filter(|request| request.expires_at_ms > now_ms)
            .map(|request| request.peer_id)
            .collect()
    })
}

fn wall_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

/// Wakes the workers of peers whose published facts changed.
pub(super) async fn run(shared: Arc<Shared>) {
    let mut previous: Option<KnownPeers> = None;
    loop {
        let Some(mut source) = shared.facts.source() else {
            tokio::select! { biased;
                _ = shared.stop.cancelled() => return,
                _ = shared.facts.attached.notified() => {},
            }
            continue;
        };
        loop {
            let next = source.borrow_and_update().clone();
            for peer in changed(previous.as_ref(), next.as_ref()) {
                shared.interests.wake(peer);
            }
            // A peer prepared an attempt toward this Node: take part as if
            // the adjacency were needed here. Interest is still bounded by
            // the installed plan's peers and acquires only admitted carriers.
            for peer in asked(next.as_ref(), wall_ms()) {
                shared.interests.touch(peer);
            }
            previous = next;
            tokio::select! { biased;
                _ = shared.stop.cancelled() => return,
                // Another source replaces this one.
                _ = shared.facts.attached.notified() => break,
                changed = source.changed() => if changed.is_err() { break },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{datagram::tests::shared, tests::netmap::support::Netmaps};

    fn node(n: u8) -> NodeId {
        format!("00000000-0000-4000-8000-{n:012}").parse().unwrap()
    }

    fn known(peers: &[(u8, i64)]) -> KnownPeers {
        let at = |seconds| chrono::DateTime::from_timestamp(seconds, 0).unwrap();
        KnownPeers {
            netmap: meshway_model::NetmapRef {
                revision: meshway_model::Revision::MIN,
                serial: meshway_model::Version::MIN,
                digest: meshway_model::Digest::from_bytes([1; 32]),
            },
            peers: Arc::new(
                peers
                    .iter()
                    .map(|(n, seconds)| {
                        (
                            node(*n),
                            PeerFacts {
                                node_id: node(*n),
                                listeners: BTreeMap::new(),
                                exit: None,
                                connected: Vec::new(),
                                observed_at: at(*seconds),
                                expires_at: at(*seconds),
                            },
                        )
                    })
                    .collect(),
            ),
            traversal_requests: Default::default(),
        }
    }

    #[tokio::test]
    async fn a_traversal_request_makes_the_target_take_part_without_business_interest() {
        let fixture = Netmaps::start(false, false).await;
        let ids = fixture.ids();
        let (shared, _) = shared(&fixture.desired[1], &fixture.keys[1]);
        let (a, b) = (ids[0], ids[1]);
        assert!(!shared.wanted(a));
        let (publish, source) = tokio::sync::watch::channel(None);
        shared.facts.attach(source);
        let task = tokio::spawn(run(shared.clone()));
        let request = |expires_at_ms| meshway_protocol::UdpTraversalRequest {
            attempt_id: meshway_model::UdpTraversalId::new(uuid::Uuid::from_u128(7)).unwrap(),
            peer_id: a,
            carrier: meshway_configuration::execution::Carrier {
                dialer: a,
                listener: b,
                listener_key: "udp".into(),
                protocol: meshway_model::TransportProtocol::NativeUdp,
            },
            expires_at_ms,
        };
        let known = |requests| meshway_control::KnownPeers {
            netmap: shared.plan().reference,
            peers: Arc::default(),
            traversal_requests: Arc::new(requests),
        };
        // An expired request is void.
        publish.send_replace(Some(known(vec![request(wall_ms() - 1)])));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!shared.wanted(a));
        publish.send_replace(Some(known(vec![request(wall_ms() + 9_000)])));
        for _ in 0..100 {
            if shared.wanted(a) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        // Interest is what makes the traversal owner query for the offer.
        assert!(
            shared.wanted(a),
            "the target registers interest in the preparer"
        );
        shared.stop.cancel();
        task.await.unwrap();
        fixture.close().await;
    }

    #[test]
    fn the_store_reads_the_latest_publication_and_names_only_changed_peers() {
        let (publish, source) = watch::channel(None);
        let store = Store::default();
        assert!(store.get(node(1)).is_none(), "nothing attached");
        store.attach(source);
        publish.send_replace(Some(known(&[(1, 1), (2, 1)])));
        assert!(store.get(node(1)).is_some() && store.get(node(3)).is_none());
        assert_eq!(
            changed(None, Some(&known(&[(1, 1), (2, 1)]))),
            vec![node(1), node(2)]
        );
        assert_eq!(
            changed(
                Some(&known(&[(1, 1), (2, 1)])),
                Some(&known(&[(1, 1), (2, 5), (3, 1)]))
            ),
            vec![node(2), node(3)]
        );
        assert_eq!(
            changed(Some(&known(&[(1, 1)])), Some(&known(&[]))),
            vec![node(1)]
        );
    }
}
