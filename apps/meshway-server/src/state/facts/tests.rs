use super::*;

#[test]
fn fifty_steady_reporters_renew_freshness_without_cursor_or_wakeup_fanout() {
    let network = NetworkId::new(uuid::Uuid::from_u128(1)).unwrap();
    let epoch = StateEpoch::new(uuid::Uuid::from_u128(2)).unwrap();
    let mut ledger = FactLedger::new(epoch);
    let nodes: BTreeSet<_> = (10..60)
        .map(|id| NodeId::new(uuid::Uuid::from_u128(id)).unwrap())
        .collect();
    let now = Utc::now();
    let facts = |node, at| PeerFacts {
        node_id: node,
        listeners: BTreeMap::new(),
        exit: None,
        connected: Vec::new(),
        observed_at: at,
        expires_at: at + chrono::Duration::seconds(60),
    };
    for node in &nodes {
        assert!(ledger.record(network, facts(*node, now), now).unwrap());
    }
    let sequence = ledger.networks[&network].sequence;
    let (sender, receiver) = watch::channel(0);
    ledger.waiters.insert(
        (network, *nodes.first().unwrap()),
        Waiter {
            sender,
            visible: nodes.clone(),
        },
    );
    for seconds in [20, 40, 60, 80, 100, 120] {
        let at = now + chrono::Duration::seconds(seconds);
        for node in &nodes {
            assert!(!ledger.record(network, facts(*node, at), at).unwrap());
        }
        assert_eq!(ledger.networks[&network].sequence, sequence);
        assert!(!receiver.has_changed().unwrap());
    }
    assert!(ledger.remove(network, *nodes.last().unwrap()).unwrap());
    assert!(receiver.has_changed().unwrap());
    assert!(ledger.networks[&network].sequence > sequence);
}
