//! Traversal signalling: a prepared attempt reaches the other participant's
//! held `sync` at once, and only as a hint naming the exact offer.
use super::*;
use meshway_model::UdpTraversalId;
use meshway_protocol::{UdpTraversalRequestIds, netmap::SyncResponse};
use std::time::{Duration, Instant};

/// One sync of `node` holding its current netmap and `state` cursor.
async fn hold(
    netmaps: &crate::netmap::Netmaps,
    node: &Node,
    state: Option<meshway_protocol::netmap::StateCursor>,
    held: Vec<UdpTraversalId>,
) -> (SyncResponse, Duration) {
    let started = Instant::now();
    let response = netmaps
        .sync(
            &node.token,
            SyncRequest {
                netmap: Some(node.reference()),
                state,
                server_key_generation: None,
                traversal_requests: UdpTraversalRequestIds::try_from(held).unwrap(),
            },
        )
        .await
        .unwrap();
    (response, started.elapsed())
}

/// The cursor after which `node`'s hold has no fact to deliver.
async fn settled(app: &App, node: &Node) -> meshway_protocol::netmap::StateCursor {
    let (response, _) = hold(&app.netmaps(), node, None, vec![]).await;
    response.peer_facts.unwrap().cursor
}

#[tokio::test]
async fn a_prepared_attempt_answers_the_targets_hold_and_ends_with_acceptance() {
    let (_dir, app, _, a, b) = setup(true).await;
    let cursor = settled(&app, &b).await;
    let held = {
        let (netmaps, b) = (app.netmaps(), clone(&b));
        tokio::spawn(async move { hold(&netmaps, &b, Some(cursor), vec![]).await })
    };
    for _ in 0..400 {
        if app.effects.netmaps.pending() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(app.effects.netmaps.pending(), 1, "b's sync is held");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let request = request(&app, &a, &b).await;
    let offer = offer(&app, &a, request).await;
    let (response, elapsed) = held.await.unwrap();
    // Far inside one hold (50 s nominal): the prepare itself woke it.
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    assert!(matches!(response.netmap, SyncNetmap::Unchanged));
    let requests = response.traversal_requests.as_slice();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].attempt_id, offer.attempt_id);
    assert_eq!(requests[0].peer_id, a.id);
    assert_eq!(requests[0].carrier, offer.carrier);
    assert_eq!(requests[0].expires_at_ms, offer.expires_at_ms);

    // The preparer is never asked to accept its own attempt.
    let (own, _) = tokio::time::timeout(
        Duration::from_secs(5),
        hold(&app.netmaps(), &a, None, vec![]),
    )
    .await
    .unwrap();
    assert!(own.traversal_requests.as_slice().is_empty());

    // Echoing the held request does not answer at once: the hold continues
    // until something else (here b's acceptance ends the request).
    let cursor = settled(&app, &b).await;
    let echoed = {
        let (netmaps, b) = (app.netmaps(), clone(&b));
        tokio::spawn(async move { hold(&netmaps, &b, Some(cursor), vec![offer.attempt_id]).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !echoed.is_finished(),
        "a held request does not answer the hold"
    );
    echoed.abort();
    let accepted = app
        .connectivity()
        .accept(&b.token, offer.attempt_id, b.cas(&offer))
        .await
        .unwrap();
    assert!(matches!(
        accepted,
        AcceptUdpTraversalResponse::Waiting { .. }
    ));
    let (after, _) = hold(&app.netmaps(), &b, None, vec![]).await;
    assert!(
        after.traversal_requests.as_slice().is_empty(),
        "an accepted participant is not asked again"
    );
}

fn clone(node: &Node) -> Node {
    Node {
        id: node.id,
        token: node.token.clone(),
        instance: node.instance.clone(),
        netmap: node.netmap.clone(),
        report: node.report.clone(),
    }
}
