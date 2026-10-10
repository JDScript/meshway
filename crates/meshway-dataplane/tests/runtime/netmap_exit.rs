//! Exit projection lifecycle on netmap snapshots: A → R → exit B with real
//! SecureNodes. Projection ACKs stand for the separate kernel owner. Scopes
//! bind the netmap snapshot: same-content renewal needs no kernel operation,
//! new content renews an exit projection only on activity, and a changed
//! selection or exit eligibility retires it.
use super::support::netmap::{DAY_MS, NetmapMesh, rule};
use meshway_dataplane::{
    Packet, PendingAction,
    exit::{ExitProjection, ExitReplyIngress, Limits},
    secure::{Applied, CircuitState, Outbound, Received},
};
use std::time::Duration;

async fn fixture(validity_ms: u64, exit: Limits, idle: Duration) -> NetmapMesh {
    let mut f = NetmapMesh::with(validity_ms, |_, _| {}).await;
    f.rebuild(
        meshway_dataplane::secure::Limits {
            flows: meshway_dataplane::Limits {
                idle_timeout: idle,
                ..Default::default()
            },
            ..Default::default()
        },
        exit,
    );
    f
}

async fn standard() -> NetmapMesh {
    fixture(DAY_MS, Default::default(), Duration::from_secs(60)).await
}

async fn handshake(f: &mut NetmapMesh) -> [u8; 32] {
    let target = f.exit_target();
    f.handshake(target).await
}

fn udp(f: &NetmapMesh, reply: bool, port: u16) -> Packet {
    f.exit_udp(reply, port)
}

fn forward(f: &mut NetmapMesh, id: [u8; 32], packet: Packet) -> PendingAction {
    let first =
        f.a.send(id, packet)
            .unwrap()
            .release(|bytes| bytes)
            .unwrap();
    let Received::Forward(next) = f.r.receive(&f.ar.carrier, first).unwrap() else {
        panic!("opaque relay")
    };
    assert_eq!(f.r.flow_count(), 0);
    let Received::Packet(packet) =
        f.b.receive(&f.rb.carrier, next.release(|bytes| bytes).unwrap())
            .unwrap()
    else {
        panic!("exit packet")
    };
    packet
}

fn rejected_forward(f: &mut NetmapMesh, id: [u8; 32], port: u16) {
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

/// Reverse-NAT input at B back to A, completing an activity Renew if one is due.
fn deliver_reply(f: &mut NetmapMesh, id: [u8; 32], packet: Packet) {
    let expected = packet.as_bytes().to_vec();
    let reply = f.b.exit_reply(ExitReplyIngress::new(packet)).unwrap();
    if let Some(event) = f.b.next_exit_projection() {
        let ExitProjection::Renew { ack, .. } = event else {
            panic!("only renewal on an established reply")
        };
        ack.complete(true).unwrap();
    }
    assert_eq!(reply.circuit(), id);
    let Received::Forward(next) =
        f.r.receive(&f.rb.carrier, reply.release(|bytes| bytes).unwrap())
            .unwrap()
    else {
        panic!("reverse relay")
    };
    let Received::Packet(packet) =
        f.a.receive(&f.ar.carrier, next.release(|bytes| bytes).unwrap())
            .unwrap()
    else {
        panic!("source reply")
    };
    let delivered = packet
        .release(|action| match action {
            meshway_dataplane::Action::Deliver { packet, .. } => packet.as_bytes().to_vec(),
            _ => panic!("delivery"),
        })
        .unwrap();
    assert_eq!(delivered, expected);
}

fn install(f: &mut NetmapMesh) -> meshway_dataplane::exit::FlowIncarnation {
    let ExitProjection::Install { flow, ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("install")
    };
    ack.complete(true).unwrap();
    flow
}

#[tokio::test]
async fn netmap_exit_first_packet_selects_the_ready_exit_locally() {
    let mut f = standard().await;
    let calls = f.sync_calls();
    let b = f.ids()[2];
    let packet = udp(&f, false, 41000);
    assert!(matches!(f.a.outbound(packet).unwrap(), Outbound::Denied(_)));
    f.a.set_ready_exits([b].into());
    let packet = udp(&f, false, 41000);
    let Outbound::Queued { init: Some(init) } = f.a.outbound(packet).unwrap() else {
        panic!("exit circuit")
    };
    let id = f.finish_handshake(init).await;
    let mut flushed = f.a.flush(32);
    let Ok(meshway_dataplane::secure::Flushed::Send(job)) = flushed.remove(0) else {
        panic!("queued exit packet")
    };
    let frame = meshway_dataplane::secure::pipeline::seal(job).unwrap();
    let Received::Forward(next) =
        f.r.receive(&f.ar.carrier, frame.release(|b| b).unwrap())
            .unwrap()
    else {
        panic!("relay")
    };
    let Received::Packet(pending) =
        f.b.receive(&f.rb.carrier, next.release(|b| b).unwrap())
            .unwrap()
    else {
        panic!("exit")
    };
    assert_eq!(pending.hop(), meshway_dataplane::Hop::Exit);
    install(&mut f);
    pending.release(|_| ()).unwrap();
    let reply = udp(&f, true, 41000);
    deliver_reply(&mut f, id, reply);
    assert_eq!(f.sync_calls(), calls);
    f.close().await;
}

#[tokio::test]
async fn netmap_exit_confirm_failure_isolates_shared_cid_and_survivor_continues_across_renewal() {
    let mut f = standard().await;
    let id = handshake(&mut f).await;
    let input = udp(&f, false, 41000);
    let failed_output = forward(&mut f, id, input);
    let failed = install(&mut f);
    let failed_scope = failed_output.exit_scope().unwrap();
    let old_cipher =
        f.b.exit_reply(ExitReplyIngress::new(udp(&f, true, 41000)))
            .unwrap();
    let input = udp(&f, false, 41001);
    let surviving_output = forward(&mut f, id, input);
    let survivor = install(&mut f);
    assert_ne!(failed, survivor);
    f.b.fail_exit_projection(&failed);
    assert_eq!(f.b.flow_count(), 1);
    assert!(failed_scope.ensure_current().is_err());
    tokio::time::timeout(Duration::from_secs(1), failed_scope.ended())
        .await
        .unwrap();
    assert!(failed_output.release(|_| panic!("failed tuple")).is_err());
    assert!(old_cipher.release(|_| panic!("failed cipher")).is_err());
    surviving_output.release(|_| ()).unwrap();
    assert_eq!(
        f.b.circuit_info(id).unwrap().unwrap().state,
        CircuitState::Established
    );
    let ExitProjection::Retire { flow, ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("exact retirement")
    };
    assert_eq!(flow, failed);
    ack.complete(true);
    assert!(f.b.next_exit_projection().is_none());
    assert!(
        f.b.exit_reply(ExitReplyIngress::new(udp(&f, true, 41000)))
            .is_err()
    );
    rejected_forward(&mut f, id, 41000);
    // Same-content renewal needs no kernel operation and keeps the survivor.
    assert_eq!(f.renew().await, [Applied::default(); 3]);
    assert!(f.b.next_exit_projection().is_none());
    let input = udp(&f, false, 41001);
    forward(&mut f, id, input).release(|_| ()).unwrap();
    let reply = udp(&f, true, 41001);
    deliver_reply(&mut f, id, reply);
    assert_eq!(f.b.flow_count(), 1);
    f.close().await;
}

#[tokio::test]
async fn netmap_exit_confirm_failure_of_last_flow_retires_cipher_and_is_idempotent() {
    let mut f = standard().await;
    let id = handshake(&mut f).await;
    let input = udp(&f, false, 41000);
    let queued = forward(&mut f, id, input);
    let flow = install(&mut f);
    let old_cipher =
        f.b.exit_reply(ExitReplyIngress::new(udp(&f, true, 41000)))
            .unwrap();
    // The source owns a different incarnation of the same tuple and CID.
    f.a.fail_exit_projection(&flow);
    assert_eq!(f.a.flow_count(), 1);
    assert!(f.a.next_exit_projection().is_none());
    let reply = udp(&f, true, 41000);
    deliver_reply(&mut f, id, reply);
    f.b.fail_exit_projection(&flow);
    f.b.fail_exit_projection(&flow);
    assert_eq!(f.b.flow_count(), 0);
    assert!(
        f.b.circuit_info(id)
            .unwrap()
            .is_none_or(|info| info.state == CircuitState::Closed)
    );
    assert!(queued.release(|_| panic!("failed output")).is_err());
    assert!(old_cipher.release(|_| panic!("retired cipher")).is_err());
    let ExitProjection::Retire {
        flow: retired, ack, ..
    } = f.b.next_exit_projection().unwrap()
    else {
        panic!("single retirement")
    };
    assert_eq!(retired, flow);
    f.b.fail_exit_projection(&flow);
    assert!(f.b.next_exit_projection().is_none());
    ack.complete(true);
    assert!(f.b.next_exit_projection().is_none());
    f.close().await;
}

#[tokio::test]
async fn netmap_exit_pending_projection_budget_and_ack_timeout_are_fail_closed() {
    let limits = Limits {
        max_pending: 1,
        projection_timeout: Duration::from_millis(250),
        ..Default::default()
    };
    let mut f = fixture(DAY_MS, limits, Duration::from_secs(60)).await;
    let id = handshake(&mut f).await;
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
    assert!(pending.release(|_| panic!("unacknowledged")).is_err());
    assert!(f.b.next_exit_projection().is_none());
    assert!(ack.complete(true).is_err());
    let ExitProjection::Retire { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("retire")
    };
    ack.complete(true);
    assert_eq!(f.b.flow_count(), 0);
    f.close().await;
}

#[tokio::test]
async fn netmap_exit_flow_and_reserved_tombstone_capacity_block_new_tuples() {
    let limits = Limits {
        max_flows: 1,
        max_tombstones: 1,
        ..Default::default()
    };
    let mut f = fixture(DAY_MS, limits, Duration::from_secs(60)).await;
    let id = handshake(&mut f).await;
    let input = udp(&f, false, 41000);
    let pending = forward(&mut f, id, input);
    install(&mut f);
    pending.release(|_| ()).unwrap();
    assert!(f.a.send(id, udp(&f, false, 41001)).is_err());
    f.a.retire_exits();
    f.b.retire_exits();
    let ExitProjection::Retire { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("retire")
    };
    ack.complete(true);
    let next = handshake(&mut f).await;
    assert!(f.a.send(next, udp(&f, false, 41000)).is_err());
    assert!(f.a.send(next, udp(&f, false, 41001)).is_err());
    // A restarted source loses its own tombstones; B still rejects both.
    let r = f.ids()[1];
    let (desired, keys) = f.content(0);
    f.a = meshway_dataplane::secure::SecureNode::new(
        &desired,
        keys,
        Default::default(),
        std::collections::BTreeMap::from([(r, f.ar.carrier.clone())]),
    )
    .unwrap();
    let restarted = handshake(&mut f).await;
    rejected_forward(&mut f, restarted, 41000);
    rejected_forward(&mut f, restarted, 41001);
    assert!(
        f.b.exit_reply(ExitReplyIngress::new(udp(&f, true, 41000)))
            .is_err()
    );
    f.close().await;
}

#[tokio::test]
async fn netmap_exit_receive_only_flow_is_bounded_by_its_own_activity_not_a_lease() {
    let mut f = fixture(8_000, Default::default(), Duration::from_secs(60)).await;
    let id = handshake(&mut f).await;
    let input = udp(&f, false, 41000);
    let queued = forward(&mut f, id, input);
    let ExitProjection::Install { scope, ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("install")
    };
    ack.complete(true).unwrap();
    let deadline = scope.deadline();
    f.validity_ms = DAY_MS;
    assert_eq!(f.renew().await, [Applied::default(); 3]);
    // Renewal reaches no component: the queued output and its kernel scope
    // stay valid, and the scope keeps its own activity bound.
    queued.release(|_| ()).unwrap();
    assert_eq!(scope.deadline(), deadline);
    assert!(f.b.next_exit_projection().is_none());
    let reply = udp(&f, true, 41000);
    deliver_reply(&mut f, id, reply);
    // The original 8 s lease has ended; nothing here depends on it.
    tokio::time::sleep(Duration::from_millis(8_020)).await;
    scope.ensure_current().unwrap();
    let reply = udp(&f, true, 41000);
    deliver_reply(&mut f, id, reply);
    assert_eq!(f.b.flow_count(), 1);
    f.close().await;
}

#[tokio::test]
async fn netmap_exit_ineligibility_and_withdrawal_retire_cipher_and_queued_outputs() {
    let mut f = standard().await;
    let id = handshake(&mut f).await;
    let input = udp(&f, false, 41000);
    let queued = forward(&mut f, id, input);
    install(&mut f);
    let old_cipher =
        f.b.exit_reply(ExitReplyIngress::new(udp(&f, true, 41000)))
            .unwrap();
    let updates = f
        .publish(|cfg, [_, _, b]| cfg.nodes.get_mut(&b).unwrap().exit_enabled = false)
        .await;
    assert!(matches!(
        updates[2],
        Applied {
            retired_circuits: 1,
            ..
        }
    ));
    assert!(queued.release(|_| panic!("replaced output")).is_err());
    assert!(old_cipher.release(|_| panic!("replaced cipher")).is_err());
    let ExitProjection::Retire { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("retire")
    };
    ack.complete(true);
    assert!(f.b.circuit_info(id).unwrap().is_none());
    assert_eq!(f.b.flow_count(), 0);
    f.close().await;

    // Engine's empty apply (lease end, revocation): the exit owner retires
    // every exit flow, which ends their cipher and queued outputs at once.
    let mut f = standard().await;
    let id = handshake(&mut f).await;
    let input = udp(&f, false, 41000);
    let queued = forward(&mut f, id, input);
    let ExitProjection::Install { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("install")
    };
    ack.complete(true).unwrap();
    let old_cipher =
        f.b.exit_reply(ExitReplyIngress::new(udp(&f, true, 41000)))
            .unwrap();
    f.b.retire_exits();
    assert!(queued.release(|_| panic!("withdrawn output")).is_err());
    assert!(old_cipher.release(|_| panic!("withdrawn cipher")).is_err());
    assert!(matches!(
        f.b.next_exit_projection(),
        Some(ExitProjection::Retire { .. })
    ));
    assert_eq!(f.b.flow_count(), 0);
    f.close().await;
}

#[tokio::test]
async fn netmap_exit_content_update_renews_only_on_activity_within_the_pending_budget() {
    let limits = Limits {
        max_pending: 1,
        ..Default::default()
    };
    let mut f = fixture(DAY_MS, limits, Duration::from_secs(2)).await;
    let id = handshake(&mut f).await;
    for port in [41000, 41001] {
        let input = udp(&f, false, port);
        let pending = forward(&mut f, id, input);
        install(&mut f);
        pending.release(|_| ()).unwrap();
    }
    // Unrelated new content keeps both flows and renews nothing by itself.
    let updates = f
        .publish(|cfg, [a, _, b]| cfg.access.push(rule(a, &format!("node:{b}"), &["tcp:22"])))
        .await;
    assert_eq!(
        updates[2],
        Applied {
            retired_circuits: 0,
            retired_flows: 0
        }
    );
    assert!(f.b.next_exit_projection().is_none());
    assert_eq!(f.b.flow_count(), 2);
    // Activity near the installed idle bound renews it; no lease is involved.
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let mut renewed = Vec::new();
    for port in [41000, 41001] {
        let reply =
            f.b.exit_reply(ExitReplyIngress::new(udp(&f, true, port)))
                .unwrap();
        let ExitProjection::Renew { flow, ack, .. } = f.b.next_exit_projection().unwrap() else {
            panic!("activity renewal")
        };
        assert!(!renewed.contains(&flow));
        renewed.push(flow);
        // The single command credit is held until this ACK.
        assert!(f.b.next_exit_projection().is_none());
        ack.complete(true).unwrap();
        reply.release(|_| ()).unwrap();
    }
    for port in [41000, 41001] {
        let reply = udp(&f, true, port);
        deliver_reply(&mut f, id, reply);
    }
    assert_eq!(f.b.flow_count(), 2);
    f.close().await;
}

#[tokio::test]
async fn netmap_exit_renew_ack_after_previous_gate_deadline_cannot_revive_flow() {
    let mut f = fixture(DAY_MS, Default::default(), Duration::from_secs(1)).await;
    let id = handshake(&mut f).await;
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
    assert!(scope.ensure_current().is_err());
    assert!(ack.complete(true).is_err());
    assert!(pending.release(|_| panic!("late renewal")).is_err());
    assert!(matches!(
        f.b.next_exit_projection(),
        Some(ExitProjection::Retire { .. })
    ));
    assert_eq!(f.b.flow_count(), 0);
    f.close().await;
}

#[tokio::test]
async fn netmap_exit_reserved_install_drains_when_retirement_waits_for_credit() {
    let limits = Limits {
        max_pending: 1,
        ..Default::default()
    };
    let mut f = fixture(DAY_MS, limits, Duration::from_secs(60)).await;
    let first = handshake(&mut f).await;
    let input = udp(&f, false, 41000);
    let pending = forward(&mut f, first, input);
    install(&mut f);
    pending.release(|_| ()).unwrap();
    let second = handshake(&mut f).await;
    let input = udp(&f, false, 41001);
    let pending = forward(&mut f, second, input);
    f.b.close(first);
    install(&mut f);
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
async fn netmap_exit_idle_tuple_retirement_preserves_other_tuples_on_the_same_cipher() {
    let mut f = fixture(DAY_MS, Default::default(), Duration::from_secs(1)).await;
    let id = handshake(&mut f).await;
    let input = udp(&f, false, 41000);
    let first = forward(&mut f, id, input);
    let ExitProjection::Install { scope, ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("install")
    };
    ack.complete(true).unwrap();
    tokio::time::sleep_until((scope.deadline() - Duration::from_millis(350)).into()).await;
    let input = udp(&f, false, 41001);
    let second = forward(&mut f, id, input);
    install(&mut f);
    second.release(|_| ()).unwrap();
    tokio::time::sleep_until((scope.deadline() + Duration::from_millis(10)).into()).await;
    assert!(matches!(
        f.b.next_exit_projection(),
        Some(ExitProjection::Retire { .. })
    ));
    assert!(first.release(|_| panic!("retired tuple")).is_err());
    assert_eq!(
        f.b.circuit_info(id).unwrap().unwrap().state,
        CircuitState::Established
    );
    let reply = udp(&f, true, 41001);
    deliver_reply(&mut f, id, reply);
    assert_eq!(f.b.flow_count(), 1);
    f.close().await;
}

#[tokio::test]
async fn netmap_exit_selection_change_ends_the_old_exit_connection() {
    let mut f = standard().await;
    let id = handshake(&mut f).await;
    let input = udp(&f, false, 41000);
    let pending = forward(&mut f, id, input);
    install(&mut f);
    pending.release(|_| ()).unwrap();
    let mesh = f.mesh_target();
    let mesh_id = f.handshake(mesh).await;
    let input = f.mesh_udp(false, 42424);
    f.a.send(mesh_id, input).unwrap();
    // A routing.egress change is a new selection: exit flows and circuits end,
    // the Mesh circuit and flow are unaffected.
    let updates = f
        .publish(|cfg, _| {
            cfg.routing
                .egress
                .rules
                .push(meshway_configuration::EgressRule {
                    remark: String::new(),
                    matcher: meshway_configuration::RouteMatch {
                        ip_cidr: vec!["198.51.100.0/24".into()],
                        domain: vec![],
                        domain_suffix: vec![],
                        rule_sets: vec![],
                    },
                    action: meshway_configuration::RuleAction::Direct,
                })
        })
        .await;
    for update in [updates[0], updates[2]] {
        assert_eq!(
            update,
            Applied {
                retired_circuits: 1,
                retired_flows: 1
            }
        );
    }
    assert!(matches!(
        f.b.next_exit_projection(),
        Some(ExitProjection::Retire { .. })
    ));
    assert!(f.a.circuit_info(id).unwrap().is_none());
    assert_eq!(
        f.a.circuit_info(mesh_id).unwrap().unwrap().state,
        CircuitState::Established
    );
    let input = f.mesh_udp(false, 42424);
    f.a.send(mesh_id, input).unwrap();
    f.close().await;
}
