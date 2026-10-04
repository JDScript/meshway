use super::*;
use std::{
    future::{Future, poll_fn},
    task::Poll,
};

// No control authority is constructed here. These tests exercise the actual
// circuit lifecycle and cancellation links independently of opaque snapshots.
pub(super) fn circuit(state: State, parent: &CancellationToken) -> Circuit {
    Circuit {
        context: Context {
            network: "00000000-0000-4000-8000-000000000001".parse().unwrap(),
            demand: "00000000-0000-4000-8000-000000000002".parse().unwrap(),
            path: vec![
                "00000000-0000-4000-8000-000000000003".parse().unwrap(),
                "00000000-0000-4000-8000-000000000004".parse().unwrap(),
            ],
            circuit: [5; 32],
            permit: [6; 32].into(),
            keys: [7; 32].into(),
        },
        state,
        expires_at: Instant::now() + Duration::from_secs(60),
        expires_at_ms: u64::MAX,
        handshake_at: Instant::now() + Duration::from_secs(10),
        handshake_at_ms: u64::MAX,
        permit_expires_at_ms: u64::MAX,
        send: Default::default(),
        receive: Default::default(),
        cancellation: parent.child_token(),
    }
}

#[tokio::test]
async fn retirement_wakes_existing_output_waiters_and_preserves_siblings_and_tombstone() {
    let root = CancellationToken::new();
    let mut closing = circuit(State::RelayEstablished, &root);
    let sibling = circuit(State::RelayEstablished, &root);
    let queued_output = closing.cancellation.clone();
    let io_child = queued_output.child_token();
    let waiting = queued_output.cancelled();
    tokio::pin!(waiting);
    poll_fn(|cx| {
        assert!(waiting.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    closing.receive[0].commit(17);
    let id = closing.context.circuit;
    closing.retire();
    tokio::time::timeout(Duration::from_secs(1), waiting)
        .await
        .unwrap();
    assert!(io_child.is_cancelled());
    assert!(!root.is_cancelled());
    assert!(!sibling.cancellation.is_cancelled());
    assert!(matches!(closing.state, State::Closed));
    assert_eq!(closing.context.circuit, id);
    assert!(closing.receive[0].check(17).is_err());
}

#[tokio::test]
async fn circuit_drop_and_table_clear_cancel_outputs_but_io_child_cancel_is_local() {
    let root = CancellationToken::new();
    let first = circuit(State::RelayAwaiting, &root);
    let second = circuit(State::RelayEstablished, &root);
    let first_output = first.cancellation.clone();
    let second_output = second.cancellation.clone();
    let io_child = second_output.child_token();
    io_child.cancel();
    assert!(!second_output.is_cancelled());
    drop(first);
    assert!(first_output.is_cancelled());
    assert!(!second_output.is_cancelled());
    let id = second.context.circuit;
    let mut retained = BTreeMap::from([(id, second)]);
    // receive/send temporarily remove the value while mutating the shared
    // Engine. Moving and reinserting it must not cancel already queued work.
    let moving = retained.remove(&id).unwrap();
    assert!(!second_output.is_cancelled());
    retained.insert(id, moving);
    assert!(!second_output.is_cancelled());
    retained.clear();
    tokio::time::timeout(Duration::from_secs(1), second_output.cancelled())
        .await
        .unwrap();
    assert!(!root.is_cancelled());
}
