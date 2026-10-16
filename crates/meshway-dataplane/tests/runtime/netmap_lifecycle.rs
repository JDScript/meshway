//! Circuit lifecycle across adjacent restarts on real SecureNodes A→R→B:
//! boot-bound retirement, relayed Close, passive keepalive, local source
//! checks and the source-admission/policy differential.
use super::netmap_flow::{delivered, first, sealed, send, wire};
use super::support::netmap::{NetmapMesh, rule, sel};
use meshway_dataplane::secure::{CircuitState, Error, Outbound, Received};
use std::collections::BTreeMap;

#[tokio::test]
async fn restarted_terminal_ends_the_relay_circuit_and_its_close_reaches_the_origin() {
    let mut f = NetmapMesh::start().await;
    let [a, r, b] = f.ids();
    let id = {
        let packet = f.mesh_udp(false, 42424);
        first(&mut f, packet)
    }
    .await;
    assert!(
        f.a.set_peer_boots(BTreeMap::from([(r, [1; 16])]))
            .is_empty()
    );
    assert!(
        f.r.set_peer_boots(BTreeMap::from([(a, [1; 16]), (b, [1; 16])]))
            .is_empty()
    );
    // A carrier change of the same peer state keeps every circuit.
    assert!(
        f.r.set_peer_boots(BTreeMap::from([(a, [1; 16]), (b, [1; 16])]))
            .is_empty()
    );
    let frame = {
        let packet = f.mesh_udp(false, 42424);
        send(&mut f, packet)
    };
    delivered(&mut f, frame);
    // B restarted: its new channel to R carries another boot.
    let mut closes =
        f.r.set_peer_boots(BTreeMap::from([(a, [1; 16]), (b, [2; 16])]));
    assert_eq!(closes.len(), 1, "one Close toward the surviving origin");
    let close = closes.remove(0);
    assert_eq!((close.next(), close.circuit()), (a, id));
    assert!(
        f.r.circuit_info(id).unwrap().is_none(),
        "the relay ended it"
    );
    let bytes = wire(close);
    assert!(meshway_dataplane::secure::is_control_frame(&bytes));
    // A forged copy on another carrier is not this hop's Close.
    assert!(f.a.receive(&f.rb.carrier, bytes.clone()).is_err());
    assert!(matches!(
        f.a.receive(&f.ar.carrier, bytes.clone()).unwrap(),
        Received::Closed { circuit } if circuit == id
    ));
    assert!(f.a.circuit_info(id).unwrap().is_none());
    assert!(
        f.a.receive(&f.ar.carrier, bytes).is_err(),
        "a replayed Close names no live circuit"
    );
    // The origin's next packet of the same flow re-handshakes at once.
    let packet = f.mesh_udp(false, 42424);
    let Outbound::Queued { init: Some(init) } = f.a.outbound(packet).unwrap() else {
        panic!("a fresh circuit for the unchanged path")
    };
    assert_ne!(init.circuit(), id);
    f.close().await;
}

#[tokio::test]
async fn restarted_origin_ends_the_terminal_circuit_without_any_frame() {
    let mut f = NetmapMesh::start().await;
    let [a, r, b] = f.ids();
    let id = {
        let packet = f.mesh_udp(false, 42424);
        first(&mut f, packet)
    }
    .await;
    assert!(
        f.b.set_peer_boots(BTreeMap::from([(r, [1; 16])]))
            .is_empty()
    );
    assert!(
        f.r.set_peer_boots(BTreeMap::from([(a, [1; 16]), (b, [1; 16])]))
            .is_empty()
    );
    // A restarted: R closes toward B; B also retires its own circuit when R
    // itself restarts.
    let mut closes =
        f.r.set_peer_boots(BTreeMap::from([(a, [9; 16]), (b, [1; 16])]));
    let close = closes.remove(0);
    assert_eq!(close.next(), b);
    assert!(matches!(
        f.b.receive(&f.rb.carrier, wire(close)).unwrap(),
        Received::Closed { circuit } if circuit == id
    ));
    assert!(f.b.circuit_info(id).unwrap().is_none());
    f.close().await;
}

/// X12: this Node's own adjacencies restarted under a new boot (a new
/// Manager). Every adjacent peer now ends its circuits through it; it ends
/// its own at once too, its Close reaches the far end through the relay, and
/// its next packet re-handshakes at once instead of waiting out the 15 s
/// reply timeout on a circuit the peers no longer hold.
#[tokio::test]
async fn own_new_boot_ends_every_circuit_at_once_and_its_close_reaches_the_far_end() {
    let mut f = NetmapMesh::start().await;
    let [_, r, b] = f.ids();
    let id = {
        let packet = f.mesh_udp(false, 42424);
        first(&mut f, packet)
    }
    .await;
    assert!(f.a.set_own_boot([1; 16]).is_empty(), "the first call binds");
    assert!(
        f.a.set_own_boot([1; 16]).is_empty(),
        "the same boot keeps all"
    );
    assert!(f.a.circuit_info(id).unwrap().is_some());
    let started = std::time::Instant::now();
    let mut closes = f.a.set_own_boot([2; 16]);
    assert!(f.a.circuit_info(id).unwrap().is_none(), "ended at once");
    assert_eq!(closes.len(), 1, "one Close toward the next hop");
    let close = closes.remove(0);
    assert_eq!((close.next(), close.circuit()), (r, id));
    let Received::Forward(forward) = f.r.receive(&f.ar.carrier, wire(close)).unwrap() else {
        panic!("the relay passes the Close on")
    };
    assert_eq!(forward.next(), b);
    assert!(f.r.circuit_info(id).unwrap().is_none());
    assert!(matches!(
        f.b.receive(&f.rb.carrier, wire(forward)).unwrap(),
        Received::Closed { circuit } if circuit == id
    ));
    assert!(f.b.circuit_info(id).unwrap().is_none());
    let packet = f.mesh_udp(false, 42424);
    let Outbound::Queued { init: Some(init) } = f.a.outbound(packet).unwrap() else {
        panic!("a fresh circuit for the unchanged path")
    };
    assert_ne!(init.circuit(), id);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    println!(
        "X12_OWN_BOOT=retired-at-once,close-relayed-to-terminal,new-init-ms={}",
        started.elapsed().as_millis()
    );
    f.close().await;
}

#[tokio::test]
async fn a_terminal_that_only_receives_answers_with_a_passive_keepalive() {
    let mut f = NetmapMesh::start().await;
    let id = {
        let packet = f.mesh_udp(false, 42424);
        first(&mut f, packet)
    }
    .await;
    let due =
        f.b.next_keepalive()
            .expect("received data owes a keepalive");
    assert!(f.a.next_keepalive().is_none(), "the sender owes none");
    assert!(f.b.keepalives(32).is_empty(), "not before it is due");
    tokio::time::sleep(due.saturating_duration_since(std::time::Instant::now())).await;
    let mut jobs = f.b.keepalives(32);
    assert_eq!(jobs.len(), 1);
    let frame = sealed(jobs.remove(0).unwrap());
    assert_eq!(frame.circuit(), id);
    assert!(f.b.next_keepalive().is_none(), "owed once");
    let Received::Forward(next) = f.r.receive(&f.rb.carrier, wire(frame)).unwrap() else {
        panic!("relay forwards it as opaque Data")
    };
    assert!(matches!(
        f.a.receive(&f.ar.carrier, wire(next)).unwrap(),
        Received::Keepalive { circuit } if circuit == id
    ));
    assert!(
        f.a.next_keepalive().is_none(),
        "a keepalive is never answered by another"
    );
    assert_eq!(
        f.a.circuit_info(id).unwrap().unwrap().state,
        CircuitState::Established
    );
    f.close().await;
}

#[tokio::test]
async fn a_local_packet_with_another_source_is_never_admitted() {
    let mut f = NetmapMesh::start().await;
    // B's address as source, sent into A's TUN: not a packet A originates.
    let forged = super::packet::echo(f.address(2), f.address(2), false, 7, 1, b"ping");
    assert_eq!(f.a.outbound(forged).unwrap_err(), Error::Packet);
    assert_eq!(f.a.flow_count(), 0);
    f.close().await;
}

/// SecureNode admission of a source packet on its projection equals the
/// policy's decision for the same flow, across Access variations (the
/// semantic differential that replaced the v5 comparison).
#[tokio::test]
async fn source_admission_equals_the_policy_decision_for_every_service() {
    use meshway_configuration::execution::BusinessProtocol;
    use meshway_policy::compile::netmap::LocalDecision;
    let mut f = NetmapMesh::start().await;
    let [a, _, b] = f.ids();
    let variations: [&[&str]; 5] = [
        &["udp:42424", "tcp:443", "icmp:*"],
        &["udp:*"],
        &["tcp:400-500"],
        &["icmp:*", "udp:1-1000"],
        &[],
    ];
    let mut source = 20_000u16;
    for services in variations {
        f.recompile(|cfg, _| {
            cfg.access = if services.is_empty() {
                vec![]
            } else {
                vec![rule(a, &sel(b), services)]
            }
        });
        f.install_on(0, true).await;
        let compiler = f.content(0).0.compiler().clone();
        let (from, to) = (f.address(0), f.address(2));
        for (protocol, port) in [
            (BusinessProtocol::Udp, Some(42424)),
            (BusinessProtocol::Udp, Some(53)),
            (BusinessProtocol::Tcp, Some(443)),
            (BusinessProtocol::Tcp, Some(22)),
            (BusinessProtocol::IcmpEcho, None),
        ] {
            source += 1;
            let packet = match (protocol, port) {
                (BusinessProtocol::Udp, Some(port)) => {
                    super::packet::udp(from, to, source, port, b"differential")
                }
                (BusinessProtocol::Tcp, Some(port)) => {
                    super::packet::tcp(from, to, (source, port), 1, 0, 0x02)
                }
                _ => super::packet::echo(from, to, false, source, 1, b"differential"),
            };
            let decided = compiler
                .decide(
                    a,
                    to,
                    protocol,
                    port.map(|port| meshway_transport::spec::Port::new(port).unwrap()),
                )
                .unwrap();
            let admitted = matches!(
                f.a.outbound(packet),
                Ok(Outbound::Queued { .. } | Outbound::Send(_) | Outbound::AwaitingAdjacency)
            );
            assert_eq!(
                admitted,
                matches!(decided, LocalDecision::Permitted(_)),
                "{services:?} {protocol:?} {port:?}"
            );
        }
    }
    f.close().await;
}
