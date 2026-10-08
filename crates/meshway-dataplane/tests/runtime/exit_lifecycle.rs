use super::exit::{deliver_reply, fixture, fixture_with_limits, fixture_with_timing, forward, udp};
use super::support::secure::SecureMesh;
use meshway_configuration::execution::BusinessProtocol;
use meshway_dataplane::{
    exit::{ExitProjection, ExitReplyIngress, Limits},
    secure::{CircuitState, Received},
};
use std::time::Duration;

fn rejected_forward(f: &mut SecureMesh, id: [u8; 32], port: u16) {
    let packet = udp(f, false, port);
    let first =
        f.a.send(id, packet)
            .unwrap()
            .release(|bytes| bytes)
            .unwrap();
    let Received::Forward(next) = f.r.receive(&f.ar.carrier, first).unwrap() else {
        panic!("relay")
    };
    assert!(
        f.b.receive(&f.rb.carrier, next.release(|bytes| bytes).unwrap())
            .is_err()
    );
}

#[tokio::test]
async fn exit_pending_projection_budget_and_ack_timeout_are_fail_closed() {
    let mut f = fixture_with_limits(
        BusinessProtocol::Udp,
        120,
        Limits {
            max_pending: 1,
            projection_timeout: Duration::from_millis(250),
            ..Default::default()
        },
    )
    .await;
    let id = f.handshake().await;
    let input = udp(&f, false, 41000);
    let pending = forward(&mut f, id, input);
    let ExitProjection::Install { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("install")
    };
    rejected_forward(&mut f, id, 41001);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), pending.projection_ready())
            .await
            .unwrap()
            .is_err()
    );
    assert!(
        pending
            .release(|_| panic!("unacknowledged output"))
            .is_err()
    );
    // In-flight callbacks keep their slot even after timeout. Retirement does
    // not silently overbook the host's command queue.
    assert!(f.b.next_exit_projection().is_none());
    assert!(ack.complete(true).is_err());
    let ExitProjection::Retire { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("retire")
    };
    ack.complete(true);
    assert_eq!(f.b.retained_flows(), 0);
    f.close().await;
}

#[tokio::test]
async fn exit_flow_and_reserved_tombstone_capacity_block_new_tuples() {
    let mut f = fixture_with_limits(
        BusinessProtocol::Udp,
        120,
        Limits {
            max_flows: 1,
            max_tombstones: 1,
            ..Default::default()
        },
    )
    .await;
    let id = f.handshake().await;
    let input = udp(&f, false, 41000);
    let pending = forward(&mut f, id, input);
    let ExitProjection::Install { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("install")
    };
    ack.complete(true).unwrap();
    pending.release(|_| ()).unwrap();
    // Capacity at the source also reserves the eventual tombstone.
    assert!(f.a.send(id, udp(&f, false, 41001)).is_err());
    f.a.retire_exits();
    f.b.retire_exits();
    let ExitProjection::Retire { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("retire")
    };
    ack.complete(true);
    let next = f.handshake().await;
    assert!(f.a.send(next, udp(&f, false, 41000)).is_err());
    assert!(f.a.send(next, udp(&f, false, 41001)).is_err());
    // A source process can lose its own tombstones. Recreate only that owner
    // from the same real Control identity: E must independently reject both
    // same-tuple reuse and a different tuple while its quarantine is full.
    f.a = meshway_dataplane::secure::SecureNode::new(
        f.mesh
            .a
            .control
            .dataplane_identity(f.mesh.a.control.execution_snapshot().unwrap())
            .unwrap(),
        Default::default(),
        std::collections::BTreeMap::from([(f.mesh.r.id, f.ar.carrier.clone())]),
    )
    .unwrap();
    let restarted = f.handshake().await;
    rejected_forward(&mut f, restarted, 41000);
    rejected_forward(&mut f, restarted, 41001);
    assert!(
        f.b.exit_reply(ExitReplyIngress::new(udp(&f, true, 41000)))
            .is_err()
    );
    f.close().await;
}

#[tokio::test]
async fn exit_receive_only_renewal_crosses_old_demand_deadline_without_rebinding_output() {
    let mut f = fixture(BusinessProtocol::Udp, 8).await;
    let id = f.handshake().await;
    let input = udp(&f, false, 41000);
    let queued = forward(&mut f, id, input);
    let ExitProjection::Install {
        flow: incarnation,
        scope: old_scope,
        ack,
        ..
    } = f.b.next_exit_projection().unwrap()
    else {
        panic!("install")
    };
    ack.complete(true).unwrap();
    let old_deadline = old_scope.deadline();
    f.renew().await;
    assert!(queued.release(|_| panic!("old scope output")).is_err());
    let ExitProjection::Renew {
        flow, scope, ack, ..
    } = f.b.next_exit_projection().unwrap()
    else {
        panic!("renew")
    };
    assert_eq!(flow, incarnation);
    assert!(scope.deadline() <= old_deadline);
    ack.complete(true).unwrap();
    // An actual admitted reply refreshes the live flow; reconciliation alone
    // never extends its old retention deadline.
    let reply = udp(&f, true, 41000);
    deliver_reply(&mut f, id, reply);
    tokio::time::sleep_until((old_deadline + Duration::from_millis(20)).into()).await;
    let reply = udp(&f, true, 41000);
    deliver_reply(&mut f, id, reply);
    assert_eq!(f.b.retained_flows(), 1);
    f.close().await;
}

#[tokio::test]
async fn exit_expiry_and_replacement_retire_cipher_and_queued_outputs() {
    let mut f = fixture(BusinessProtocol::Udp, 120).await;
    let id = f.handshake().await;
    let input = udp(&f, false, 41000);
    let queued = forward(&mut f, id, input);
    let ExitProjection::Install { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("install")
    };
    ack.complete(true).unwrap();
    let old_cipher =
        f.b.exit_reply(ExitReplyIngress::new(udp(&f, true, 41000)))
            .unwrap();
    let e = f.mesh.b.id;
    f.mesh
        .server
        .configure(&f.mesh.owner, |input| {
            input.nodes.get_mut(&e).unwrap().exit_enabled = false;
        })
        .await;
    f.reconcile().await;
    assert!(queued.release(|_| panic!("replaced output")).is_err());
    assert!(old_cipher.release(|_| panic!("replaced cipher")).is_err());
    let ExitProjection::Retire { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("retire")
    };
    ack.complete(true);
    assert_eq!(
        f.b.circuit_info(id).unwrap().unwrap().state,
        CircuitState::Closed
    );
    assert_eq!(f.b.retained_flows(), 0);
    f.close().await;

    let mut f = fixture(BusinessProtocol::Udp, 5).await;
    let id = f.handshake().await;
    let input = udp(&f, false, 41000);
    let queued = forward(&mut f, id, input);
    let ExitProjection::Install { scope, ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("install")
    };
    ack.complete(true).unwrap();
    let old_cipher =
        f.b.exit_reply(ExitReplyIngress::new(udp(&f, true, 41000)))
            .unwrap();
    tokio::time::sleep_until((scope.deadline() + Duration::from_millis(10)).into()).await;
    assert!(queued.release(|_| panic!("expired output")).is_err());
    assert!(old_cipher.release(|_| panic!("expired cipher")).is_err());
    assert!(matches!(
        f.b.next_exit_projection(),
        Some(ExitProjection::Retire { .. })
    ));
    assert_eq!(f.b.retained_flows(), 0);
    f.close().await;
}

#[tokio::test]
async fn exit_successor_renews_more_flows_than_pending_budget_without_overbooking() {
    let mut f = fixture_with_limits(
        BusinessProtocol::Udp,
        120,
        Limits {
            max_pending: 1,
            ..Default::default()
        },
    )
    .await;
    let id = f.handshake().await;
    for port in [41000, 41001] {
        let input = udp(&f, false, port);
        let pending = forward(&mut f, id, input);
        let ExitProjection::Install { ack, .. } = f.b.next_exit_projection().unwrap() else {
            panic!("install")
        };
        ack.complete(true).unwrap();
        pending.release(|_| ()).unwrap();
    }
    f.renew().await;
    let mut incarnations = Vec::new();
    for _ in 0..2 {
        let ExitProjection::Renew { flow, ack, .. } = f.b.next_exit_projection().unwrap() else {
            panic!("renew")
        };
        assert!(!incarnations.contains(&flow));
        incarnations.push(flow);
        assert!(f.b.next_exit_projection().is_none());
        ack.complete(true).unwrap();
    }
    for port in [41000, 41001] {
        let reply = udp(&f, true, port);
        deliver_reply(&mut f, id, reply);
    }
    assert_eq!(f.b.retained_flows(), 2);
    f.close().await;
}

#[tokio::test]
async fn exit_renew_ack_after_previous_gate_deadline_cannot_revive_flow() {
    let mut f = fixture_with_timing(
        BusinessProtocol::Udp,
        120,
        Default::default(),
        Duration::from_secs(1),
    )
    .await;
    let id = f.handshake().await;
    let input = udp(&f, false, 41000);
    let first = forward(&mut f, id, input);
    let ExitProjection::Install {
        scope: old, ack, ..
    } = f.b.next_exit_projection().unwrap()
    else {
        panic!("install")
    };
    ack.complete(true).unwrap();
    first.release(|_| ()).unwrap();
    tokio::time::sleep_until((old.deadline() - Duration::from_millis(350)).into()).await;
    let input = udp(&f, false, 41000);
    let pending = forward(&mut f, id, input);
    let ExitProjection::Renew { scope, ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("renew")
    };
    assert!(scope.deadline() > old.deadline());
    tokio::time::sleep_until((old.deadline() + Duration::from_millis(10)).into()).await;
    assert!(
        scope.ensure_current().is_err(),
        "kernel submission itself must expire before a late ACK"
    );
    assert!(ack.complete(true).is_err());
    assert!(pending.release(|_| panic!("late renewal")).is_err());
    assert!(matches!(
        f.b.next_exit_projection(),
        Some(ExitProjection::Retire { .. })
    ));
    assert_eq!(f.b.retained_flows(), 0);
    f.close().await;
}

#[tokio::test]
async fn exit_reserved_install_can_drain_when_retirement_waits_for_command_credit() {
    let mut f = fixture_with_limits(
        BusinessProtocol::Udp,
        120,
        Limits {
            max_pending: 1,
            ..Default::default()
        },
    )
    .await;
    let first = f.handshake().await;
    let input = udp(&f, false, 41000);
    let pending = forward(&mut f, first, input);
    let ExitProjection::Install { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("install")
    };
    ack.complete(true).unwrap();
    pending.release(|_| ()).unwrap();
    let second = f.handshake().await;
    let input = udp(&f, false, 41001);
    let pending = forward(&mut f, second, input);
    f.b.close(first);
    // The only command credit belongs to this unclaimed Install. Returning
    // None forever would prevent its ACK from making Retire schedulable.
    let ExitProjection::Install { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("reserved install")
    };
    ack.complete(true).unwrap();
    let ExitProjection::Retire { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("retire after credit")
    };
    ack.complete(true);
    pending.release(|_| ()).unwrap();
    let reply = udp(&f, true, 41001);
    deliver_reply(&mut f, second, reply);
    f.close().await;
}

#[tokio::test]
async fn exit_idle_tuple_retirement_preserves_other_live_tuples_on_the_same_cipher() {
    let mut f = fixture_with_timing(
        BusinessProtocol::Udp,
        120,
        Default::default(),
        Duration::from_secs(1),
    )
    .await;
    let id = f.handshake().await;
    let input = udp(&f, false, 41000);
    let first = forward(&mut f, id, input);
    let ExitProjection::Install { scope, ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("install")
    };
    ack.complete(true).unwrap();
    // Retain this output to check per-flow cancellation independently of CID.
    tokio::time::sleep_until((scope.deadline() - Duration::from_millis(350)).into()).await;
    let input = udp(&f, false, 41001);
    let second = forward(&mut f, id, input);
    let ExitProjection::Install { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("second install")
    };
    ack.complete(true).unwrap();
    second.release(|_| ()).unwrap();
    tokio::time::sleep_until((scope.deadline() + Duration::from_millis(10)).into()).await;
    assert!(matches!(
        f.b.next_exit_projection(),
        Some(ExitProjection::Retire { .. })
    ));
    assert!(first.release(|_| panic!("retired tuple output")).is_err());
    assert_eq!(
        f.b.circuit_info(id).unwrap().unwrap().state,
        CircuitState::Established
    );
    let reply = udp(&f, true, 41001);
    deliver_reply(&mut f, id, reply);
    assert_eq!(f.b.retained_flows(), 1);
    f.close().await;
}
