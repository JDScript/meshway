//! Real Server/Control grants and the production SecureNode on A -> R -> E.
//! Projection ACKs stand for the separate kernel owner; no NAT is simulated.
use super::support::{packet, request, secure::SecureMesh};
use meshway_configuration::{AccessRule, EgressRule, RouteMatch, RuleAction, execution::*};
use meshway_dataplane::{
    Action, Error, Packet, PendingAction,
    exit::{ExitProjection, ExitReplyIngress},
    secure::Received,
};
use meshway_model::{
    exit::{ExitCapability, ExitFamily, ExitStatus},
    state::*,
};
use meshway_protocol::DemandRequest;
use std::{collections::BTreeMap, net::IpAddr};

pub(super) async fn fixture(protocol: BusinessProtocol, seconds: u16) -> SecureMesh {
    fixture_with_limits(protocol, seconds, Default::default()).await
}

pub(super) async fn fixture_with_limits(
    protocol: BusinessProtocol,
    seconds: u16,
    limits: meshway_dataplane::exit::Limits,
) -> SecureMesh {
    fixture_with_timing(
        protocol,
        seconds,
        limits,
        std::time::Duration::from_secs(60),
    )
    .await
}

pub(super) async fn fixture_with_timing(
    protocol: BusinessProtocol,
    seconds: u16,
    limits: meshway_dataplane::exit::Limits,
    idle_timeout: std::time::Duration,
) -> SecureMesh {
    fixture_destination(
        protocol,
        seconds,
        limits,
        idle_timeout,
        "8.8.8.8",
        "autogroup:internet",
    )
    .await
}

async fn fixture_destination(
    protocol: BusinessProtocol,
    seconds: u16,
    limits: meshway_dataplane::exit::Limits,
    idle_timeout: std::time::Duration,
    destination: &str,
    access: &str,
) -> SecureMesh {
    let mut f = SecureMesh::start().await;
    let a = f.mesh.a.id;
    let r = f.mesh.r.id;
    let e = f.mesh.b.id;
    f.mesh
        .server
        .configure(&f.mesh.owner, |input| {
            input.nodes.get_mut(&e).unwrap().exit_enabled = true;
            input.access.push(AccessRule {
                remark: String::new(),
                src: vec!["*".into()],
                dst: vec![access.into()],
                ip: vec!["udp:42424".into(), "tcp:42424".into(), "icmp:*".into()],
                bidirectional: false,
            });
            input.routing.egress.rules.push(EgressRule {
                remark: String::new(),
                matcher: RouteMatch {
                    ip_cidr: vec![format!("{destination}/32")],
                    ..Default::default()
                },
                action: RuleAction::Exit {
                    nodes: vec![format!("node:{e}")],
                    via: vec![format!("node:{r}")],
                },
            });
        })
        .await;
    let snapshot = f.mesh.b.sync_current().await;
    let now = std::time::SystemTime::now().into();
    let cfg = snapshot.configuration().unwrap();
    f.mesh
        .b
        .control
        .report_runtime_state(
            NodeState {
                system: SystemState {
                    hostname: "exit-fixture".into(),
                    os: "linux".into(),
                    arch: "aarch64".into(),
                    agent_version: "test".into(),
                },
                capabilities: cfg.self_keys.capabilities.clone(),
                configuration: ConfigurationState::default(),
                exit_capability: Some(ExitCapability {
                    instance_id: snapshot.instance().instance_id,
                    target: snapshot.reference(),
                    supported_families: vec![ExitFamily::Ipv4],
                    installed_families: vec![],
                    status: ExitStatus::Ready,
                    observed_at: now,
                }),
                listeners: BTreeMap::new(),
                sockets: BTreeMap::new(),
                links: BTreeMap::new(),
                paths: BTreeMap::new(),
                selection: SelectionState::default(),
                probes: vec![],
                problems: vec![],
            },
            now,
        )
        .await
        .unwrap();
    f.mesh.demand = request();
    let response = f
        .mesh
        .a
        .control
        .create_demand(&DemandRequest {
            instance: f.mesh.a.instance(),
            request_id: f.mesh.demand,
            target: BusinessTarget::Ip {
                address: ExecutionAddress(destination.parse().unwrap()),
            },
            protocol,
            port: (protocol != BusinessProtocol::IcmpEcho).then(|| 42424.try_into().unwrap()),
            profile: "mesh".into(),
            duration_seconds: seconds,
        })
        .await
        .unwrap();
    assert!(
        matches!(response.decision, DemandDecision::Permitted { .. }),
        "{:?}",
        response.decision
    );
    // The capability report's preparatory snapshot was intentionally not used
    // by SecureNode. Recreate before any business flow, then use direct handoffs.
    for (node, secure) in [
        (&mut f.mesh.a, &mut f.a),
        (&mut f.mesh.r, &mut f.r),
        (&mut f.mesh.b, &mut f.b),
    ] {
        let snapshot = node.sync_current().await;
        let cfg = snapshot.configuration().unwrap();
        assert_eq!(cfg.permits[&f.mesh.demand].paths[0].nodes, f.mesh.path);
        let carriers = if node.id == a {
            BTreeMap::from([(r, f.ar.carrier.clone())])
        } else if node.id == e {
            BTreeMap::from([(r, f.rb.carrier.clone())])
        } else {
            BTreeMap::from([(a, f.ar.carrier.clone()), (e, f.rb.carrier.clone())])
        };
        *secure = meshway_dataplane::secure::SecureNode::with_exit_limits(
            node.control.dataplane_identity(snapshot).unwrap(),
            meshway_dataplane::secure::Limits {
                flows: meshway_dataplane::Limits {
                    idle_timeout,
                    ..Default::default()
                },
                ..Default::default()
            },
            carriers,
            limits,
        )
        .unwrap();
    }
    f
}

fn addresses(f: &SecureMesh) -> (IpAddr, IpAddr) {
    let snapshot = f.mesh.a.control.execution_snapshot().unwrap();
    let BusinessTarget::Ip { address } =
        snapshot.configuration().unwrap().permits[&f.mesh.demand].target
    else {
        panic!("exact IP exit")
    };
    (
        f.mesh
            .a
            .control
            .execution_snapshot()
            .unwrap()
            .configuration()
            .unwrap()
            .addresses
            .ipv4
            .unwrap()
            .into(),
        address.0,
    )
}
pub(super) fn udp(f: &SecureMesh, reply: bool, port: u16) -> Packet {
    let (a, e) = addresses(f);
    if reply {
        packet::udp(e, a, 42424, port, b"exit reply")
    } else {
        packet::udp(a, e, port, 42424, b"exit forward")
    }
}
pub(super) fn forward(f: &mut SecureMesh, id: [u8; 32], packet: Packet) -> PendingAction {
    let first =
        f.a.send(id, packet)
            .unwrap()
            .release(|bytes| bytes)
            .unwrap();
    let Received::Forward(next) = f.r.receive(&f.ar.carrier, first).unwrap() else {
        panic!("opaque relay")
    };
    assert_eq!(f.r.retained_flows(), 0);
    let Received::Packet(packet) =
        f.b.receive(&f.rb.carrier, next.release(|bytes| bytes).unwrap())
            .unwrap()
    else {
        panic!("exit packet")
    };
    packet
}
pub(super) fn deliver_reply(f: &mut SecureMesh, id: [u8; 32], packet: Packet) {
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
    packet
        .release(|action| {
            let Action::Deliver { packet, .. } = action else {
                panic!("original source delivery")
            };
            assert_eq!(packet.as_bytes(), expected);
        })
        .unwrap();
}

#[tokio::test]
async fn exit_udp_install_ack_gates_first_packet_and_reply_uses_original_cid() {
    let mut f = fixture(BusinessProtocol::Udp, 120).await;
    let id = f.handshake().await;
    let input = udp(&f, false, 41000);
    let packet = forward(&mut f, id, input);
    assert!(matches!(
        packet.ensure_current(),
        Err(Error::ProjectionPending)
    ));
    assert!(packet.exit_scope().is_err());
    let another = udp(&f, false, 41000);
    assert!(
        forward(&mut f, id, another)
            .release(|_| panic!("first packet cannot bypass ACK"))
            .is_err()
    );
    let ExitProjection::Install {
        flow,
        scope,
        tuple,
        ack,
    } = f.b.next_exit_projection().unwrap()
    else {
        panic!("install")
    };
    assert_eq!(tuple.destination.to_string(), "8.8.8.8");
    assert_eq!(packet.exit_flow(), Some(flow));
    assert!(
        scope.deadline_ms()
            <= f.mesh
                .b
                .control
                .execution_snapshot()
                .unwrap()
                .configuration()
                .unwrap()
                .permits[&f.mesh.demand]
                .expires_at_ms
    );
    assert!(
        f.b.exit_reply(ExitReplyIngress::new(udp(&f, true, 41000)))
            .is_err()
    );
    ack.complete(true).unwrap();
    packet.exit_scope().unwrap().ensure_current().unwrap();
    packet
        .release(|action| assert!(matches!(action, Action::Exit { .. })))
        .unwrap();
    let reply = udp(&f, true, 41000);
    deliver_reply(&mut f, id, reply);
    assert!(
        f.b.exit_reply(ExitReplyIngress::new(udp(&f, true, 41001)))
            .is_err()
    );
    assert_eq!(f.b.exit_counters().unassociated_reply, 1);
    assert_eq!(f.b.exit_counters().reply_rejected, 2);
    f.close().await;
}

#[tokio::test]
async fn exit_projection_failure_retires_flow_and_blocks_tuple_reuse() {
    let mut f = fixture(BusinessProtocol::Udp, 120).await;
    let id = f.handshake().await;
    let input = udp(&f, false, 41000);
    let packet = forward(&mut f, id, input);
    let ExitProjection::Install { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("install")
    };
    assert!(ack.complete(false).is_err());
    assert!(packet.release(|_| panic!("failed install output")).is_err());
    let ExitProjection::Retire { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("retire")
    };
    ack.complete(true);
    assert_eq!(f.b.retained_flows(), 0);
    assert!(
        f.b.exit_reply(ExitReplyIngress::new(udp(&f, true, 41000)))
            .is_err()
    );
    f.close().await;
}

#[tokio::test]
async fn exit_tcp_and_echo_reply_require_the_forward_protocol_state() {
    for protocol in [BusinessProtocol::Tcp, BusinessProtocol::IcmpEcho] {
        let mut f = fixture(protocol, 120).await;
        let id = f.handshake().await;
        let (a, e) = addresses(&f);
        let (first, reply) = if protocol == BusinessProtocol::Tcp {
            (
                packet::tcp(a, e, (41000, 42424), 10, 0, 2),
                packet::tcp(e, a, (42424, 41000), 20, 11, 0x12),
            )
        } else {
            (
                packet::echo(a, e, false, 77, 8, b"echo"),
                packet::echo(e, a, true, 77, 8, b"echo"),
            )
        };
        let pending = forward(&mut f, id, first);
        let ExitProjection::Install { ack, tuple, .. } = f.b.next_exit_projection().unwrap() else {
            panic!("install")
        };
        assert_eq!(tuple.protocol, protocol);
        ack.complete(true).unwrap();
        pending.release(|_| ()).unwrap();
        deliver_reply(&mut f, id, reply);
        if protocol == BusinessProtocol::Tcp {
            forward(&mut f, id, packet::tcp(a, e, (41000, 42424), 11, 21, 0x10))
                .release(|_| ())
                .unwrap();
            forward(
                &mut f,
                id,
                packet::tcp_with_payload(a, e, (41000, 42424), 11, 21, 0x18, b"hello"),
            )
            .release(|action| {
                let Action::Exit { packet, .. } = action else {
                    panic!("TCP exit")
                };
                assert!(packet.as_bytes().ends_with(b"hello"));
            })
            .unwrap();
            deliver_reply(
                &mut f,
                id,
                packet::tcp_with_payload(e, a, (42424, 41000), 21, 16, 0x18, b"world"),
            );
            // TCP completion must reach the kernel before retirement instead
            // of cancelling its own final RST at FlowTable commit.
            forward(&mut f, id, packet::tcp(a, e, (41000, 42424), 16, 26, 0x14))
                .release(|action| assert!(matches!(action, Action::Exit { .. })))
                .unwrap();
            assert!(f.b.next_exit_projection().is_none());
            assert!(
                f.a.send(id, packet::tcp(a, e, (41000, 42424), 16, 26, 0x10))
                    .is_err()
            );
        } else {
            assert!(
                f.b.exit_reply(ExitReplyIngress::new(packet::echo(
                    e,
                    a,
                    true,
                    77,
                    9,
                    b"wrong sequence"
                )))
                .is_err()
            );
        }
        f.close().await;
    }
}

#[tokio::test]
async fn exit_rejects_forged_source_wrong_path_cid_tuple_hairpin_and_ipv6() {
    let mut f = fixture(BusinessProtocol::Udp, 120).await;
    let id = f.handshake().await;
    let (a, e) = addresses(&f);
    assert!(
        f.a.start(f.mesh.demand, vec![f.mesh.a.id, f.mesh.b.id])
            .is_err()
    );
    assert!(
        f.a.send(
            id,
            packet::udp("10.0.0.1".parse().unwrap(), e, 41000, 42424, b"forged")
        )
        .is_err()
    );
    assert!(
        f.a.send(id, packet::udp(a, a, 41000, 42424, b"hairpin"))
            .is_err()
    );
    assert!(
        f.a.send(id, packet::udp(a, e, 41000, 443, b"wrong port"))
            .is_err()
    );
    assert!(
        f.a.send(
            id,
            packet::udp(
                "fd00::1".parse().unwrap(),
                "2001:4860:4860::8888".parse().unwrap(),
                41000,
                42424,
                b"v6"
            )
        )
        .is_err()
    );
    assert_eq!(f.a.exit_counters().ipv6_rejected, 1);
    let input = udp(&f, false, 41000);
    let wire =
        f.a.send(id, input)
            .unwrap()
            .release(|bytes| bytes.into_vec())
            .unwrap();
    assert!(f.b.receive(&f.ar.carrier, wire.clone()).is_err());
    let mut bad_cid = wire.clone();
    bad_cid[10] ^= 1;
    assert!(f.r.receive(&f.ar.carrier, bad_cid).is_err());
    let Received::Forward(next) = f.r.receive(&f.ar.carrier, wire).unwrap() else {
        panic!("relay")
    };
    let Received::Packet(pending) =
        f.b.receive(&f.rb.carrier, next.release(|b| b).unwrap())
            .unwrap()
    else {
        panic!("exit")
    };
    let ExitProjection::Install { ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("install")
    };
    ack.complete(true).unwrap();
    pending.release(|_| ()).unwrap();
    let other = f.handshake().await;
    assert!(f.b.send(other, udp(&f, true, 41000)).is_err());
    assert!(
        f.b.exit_reply(ExitReplyIngress::new(udp(&f, true, 41001)))
            .is_err()
    );
    f.close().await;
}

#[tokio::test]
async fn exit_explicit_lan_cidr_uses_the_same_flow_projection_and_reverse_path() {
    let mut f = fixture_destination(
        BusinessProtocol::Udp,
        120,
        Default::default(),
        std::time::Duration::from_secs(60),
        "192.168.77.4",
        "192.168.77.0/24",
    )
    .await;
    let scope = f.mesh.a.control.execution_snapshot().unwrap();
    assert!(matches!(
        scope.configuration().unwrap().permits[&f.mesh.demand]
            .exit
            .as_ref()
            .unwrap()
            .destination,
        ExitDestinationScope::Cidr { .. }
    ));
    let id = f.handshake().await;
    let input = udp(&f, false, 41000);
    let pending = forward(&mut f, id, input);
    let ExitProjection::Install { tuple, ack, .. } = f.b.next_exit_projection().unwrap() else {
        panic!("LAN install")
    };
    assert_eq!(tuple.destination.to_string(), "192.168.77.4");
    ack.complete(true).unwrap();
    pending.release(|_| ()).unwrap();
    let reply = udp(&f, true, 41000);
    deliver_reply(&mut f, id, reply);
    f.close().await;
}
