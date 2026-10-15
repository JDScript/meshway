//! Peer facts are last known: past `expiresAt` an exit's readiness stays a
//! selection input (after every current one) while the lease lasts; only
//! the netmap ending ends it.
use super::{
    HOUR, enrolled,
    support::{Facts, compiled_with_peer, full},
};
use crate::{Server, faults::Proxy};
use meshway_model::exit::{ExitFamily, ExitStatus};
use meshway_protocol::{PeerExitFacts, PeerFacts, PeerFactsDelta, StateCursor};
use std::time::Duration;

fn exit_facts(peer: meshway_model::NodeId, status: ExitStatus, left: Duration) -> PeerFacts {
    let now = chrono::Utc::now();
    PeerFacts {
        node_id: peer,
        listeners: Default::default(),
        exit: Some(PeerExitFacts {
            role_generation: meshway_model::Digest::from_bytes([1; 32]),
            status,
            installed_families: vec![ExitFamily::Ipv4],
        }),
        connected: Vec::new(),
        observed_at: now,
        expires_at: now + chrono::Duration::from_std(left).unwrap(),
    }
}

#[tokio::test]
async fn an_exit_last_known_ready_stays_selectable_past_its_freshness_within_the_lease() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "facts-stale").await;
    let (netmap, peer) = compiled_with_peer(&binding, "facts-stale");
    let mut response = full(&binding, &netmap, HOUR);
    response.peer_facts = Some(PeerFactsDelta {
        base: None,
        cursor: StateCursor {
            epoch: "00000000-0000-4000-8000-0000000000e4".parse().unwrap(),
            sequence: meshway_model::Sequence::new(1).unwrap(),
        },
        netmap_serial: netmap.netmap.serial,
        updated: [(
            peer,
            exit_facts(peer, ExitStatus::Ready, Duration::from_millis(300)),
        )]
        .into(),
        removed: vec![],
        refreshed: Default::default(),
    });
    script.push(Ok(response));
    control.sync(&mut Facts::default()).await.unwrap();
    let known = control.known_peers().unwrap();
    let fresh = known.ready_exits(chrono::Utc::now());
    assert_eq!(fresh.current, [peer].into());
    assert!(fresh.last_known.is_empty());
    // No further answer (Node HTTP closed): the fact expires, the lease does not.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(control.netmap().is_some(), "the lease still holds");
    let known = control.known_peers().unwrap();
    let stale = known.ready_exits(chrono::Utc::now());
    assert!(stale.current.is_empty());
    assert_eq!(
        stale.last_known,
        [peer].into(),
        "last known ready, not refused"
    );
    // A last known value that is not ready is never chosen.
    let mut peers = (*known.peers).clone();
    peers.insert(peer, exit_facts(peer, ExitStatus::Probing, Duration::ZERO));
    let probing = meshway_control::KnownPeers {
        peers: std::sync::Arc::new(peers),
        ..known
    };
    assert_eq!(probing.ready_exits(chrono::Utc::now()), Default::default());
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}
