use super::*;
use std::{
    future::{Future, poll_fn},
    task::Poll,
};

// No control authority is constructed here. These tests exercise the actual
// circuit lifecycle and cancellation links independently of opaque snapshots.
pub(super) fn circuit(state: State, parent: &CancellationToken) -> Circuit {
    Circuit {
        context: Arc::new(Context {
            network: "00000000-0000-4000-8000-000000000001".parse().unwrap(),
            grant: crate::secure::CircuitGrant {
                purpose: crate::Purpose::Business,
                target: crate::Target::Mesh {
                    node: "00000000-0000-4000-8000-000000000004".parse().unwrap(),
                },
            },
            path: vec![
                "00000000-0000-4000-8000-000000000003".parse().unwrap(),
                "00000000-0000-4000-8000-000000000004".parse().unwrap(),
            ],
            circuit: [5; 32],
            authorization: [6; 32].into(),
            keys: [7; 32].into(),
            initiated_at_ms: 0,
        }),
        state,
        cap: Some(Cap {
            at: Instant::now() + Duration::from_secs(60),
            ms: u64::MAX,
        }),
        handshake: Cap {
            at: Instant::now() + Duration::from_secs(10),
            ms: u64::MAX,
        },
        active: Instant::now(),
        unanswered: None,
        origin: true,
        selected: None,
        send: Default::default(),
        receive: Default::default(),
        cancellation: parent.child_token(),
        recovery: Default::default(),
        session: Default::default(),
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

#[test]
fn established_circuit_ends_when_idle_and_its_origin_ends_it_without_replies() {
    let root = CancellationToken::new();
    let now = Instant::now();
    let mut idle = circuit(State::RelayEstablished, &root);
    idle.cap = None;
    idle.check_now(now, 0).unwrap();
    // Frames in either direction keep a relay or endpoint alive.
    idle.active = now - CIRCUIT_IDLE + Duration::from_secs(1);
    idle.check_now(now, 0).unwrap();
    idle.sent(now);
    idle.check_now(now + CIRCUIT_IDLE - Duration::from_secs(1), 0)
        .unwrap();
    assert_eq!(idle.check_now(now + CIRCUIT_IDLE, 0), Err(Error::Expired));
    // A relay never applies the origin's no-reply bound: it forwards for
    // others and cannot re-handshake.
    idle.active = now;
    idle.unanswered = Some(now - NO_REPLY);
    idle.check_now(now, 0).unwrap();

    // An origin endpoint that keeps sending into a restarted peer ends the
    // circuit after NO_REPLY; any authenticated receive answers it.
    let endpoint = || {
        let mut circuit = circuit(State::Closed, &root);
        circuit.cap = None;
        circuit.state = State::RelayEstablished;
        circuit
    };
    let mut origin = endpoint();
    origin.sent(now);
    assert_eq!(origin.unanswered, Some(now));
    origin.sent(now + Duration::from_secs(5));
    assert_eq!(origin.unanswered, Some(now), "the oldest unanswered send");
    origin.received(now + Duration::from_secs(6));
    assert_eq!(origin.unanswered, None);
    origin.sent(now + Duration::from_secs(7));
    assert_eq!(
        origin.activity_deadline(),
        Some(now + Duration::from_secs(7) + CIRCUIT_IDLE),
        "only an Endpoint origin applies the no-reply bound"
    );
    assert!(NO_REPLY < CIRCUIT_IDLE);
    // A handshaking circuit has only its handshake bound.
    let handshaking = circuit(State::RelayAwaiting, &root);
    assert_eq!(handshaking.activity_deadline(), None);
}
