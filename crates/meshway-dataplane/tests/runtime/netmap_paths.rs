//! Path choice on real SecureNodes when A↔B is allowed directly and through
//! R: relaying while the direct path lacks a carrier, unanswered-path pauses
//! and the relay a new circuit would take.
use super::netmap_flow::{delivered, sealed};
use super::support::netmap::{NetmapMesh, rule, sel};
use meshway_dataplane::secure::{Flushed, Outbound};
use std::{collections::BTreeMap, time::Duration};

/// A↔B allowed directly as well as through R: Policy's paths are [A, B]
/// then [A, R, B].
async fn direct_or_relayed() -> NetmapMesh {
    NetmapMesh::with(super::support::netmap::DAY_MS, |cfg, _| {
        cfg.routing.mesh.clear()
    })
    .await
}

#[tokio::test]
async fn a_direct_path_without_a_carrier_relays_and_keeps_wanting_the_direct_adjacency() {
    let mut f = direct_or_relayed().await;
    let [_, r, b] = f.ids();
    let packet = f.mesh_udp(false, 42424);
    let Outbound::Queued { init: Some(init) } = f.a.outbound(packet).unwrap() else {
        panic!("the relayed path starts at once")
    };
    assert_eq!(init.next(), r, "relayed through R");
    let interests: Vec<_> =
        f.a.take_adjacency_interests()
            .into_iter()
            .map(|interest| interest.peer)
            .collect();
    assert_eq!(
        interests,
        [b],
        "the preferred direct adjacency is still wanted"
    );
    let id = f.finish_handshake(init).await;
    let mut flushed = f.a.flush(32);
    let Ok(Flushed::Send(job)) = flushed.remove(0) else {
        panic!("queued packet")
    };
    assert_eq!(job.circuit(), id);
    delivered(&mut f, sealed(job));
    f.close().await;
}

#[tokio::test]
async fn a_new_flow_opens_the_direct_path_once_it_has_a_carrier_instead_of_joining_the_relay() {
    let mut f = direct_or_relayed().await;
    let [a, r, b] = f.ids();
    let packet = f.mesh_udp(false, 42424);
    let Outbound::Queued { init: Some(init) } = f.a.outbound(packet).unwrap() else {
        panic!("the relayed path starts at once")
    };
    assert_eq!(init.next(), r);
    let relayed = f.finish_handshake(init).await;
    let mut flushed = f.a.flush(32);
    let Ok(Flushed::Send(job)) = flushed.remove(0) else {
        panic!("queued packet")
    };
    delivered(&mut f, sealed(job));
    let direct = meshway_dataplane::secure::Carrier {
        dialer: a,
        listener: b,
        listener_key: "tcp".into(),
        protocol: meshway_model::TransportProtocol::NativeTcp,
    };
    f.a.set_carriers(BTreeMap::from([(r, f.ar.carrier.clone()), (b, direct)]))
        .unwrap();
    // The existing flow keeps its relayed circuit.
    let Outbound::Send(job) = f.a.outbound(f.mesh_udp(false, 42424)).unwrap() else {
        panic!("the established flow keeps its path")
    };
    assert_eq!(job.circuit(), relayed);
    drop(job);
    // A new flow ranks the direct path first and does not join the relay.
    let packet = super::packet::udp(f.address(0), f.address(2), 50001, 42424, b"new flow");
    let Outbound::Queued { init: Some(init) } = f.a.outbound(packet).unwrap() else {
        panic!("a new circuit on the direct path")
    };
    assert_eq!(init.next(), b);
    drop(init);
    f.close().await;
}

#[tokio::test]
async fn an_unanswered_path_is_paused_and_the_next_circuit_takes_another_one() {
    let mut f = direct_or_relayed().await;
    let [a, r, b] = f.ids();
    f.rebuild(
        meshway_dataplane::secure::Limits {
            handshake_timeout: Duration::from_millis(100),
            ..Default::default()
        },
        Default::default(),
    );
    // A has a carrier to B, but nothing on it is ever answered.
    let direct = meshway_dataplane::secure::Carrier {
        dialer: a,
        listener: b,
        listener_key: "tcp".into(),
        protocol: meshway_model::TransportProtocol::NativeTcp,
    };
    f.a.set_carriers(BTreeMap::from([(r, f.ar.carrier.clone()), (b, direct)]))
        .unwrap();
    let packet = f.mesh_udp(false, 42424);
    let Outbound::Queued { init: Some(lost) } = f.a.outbound(packet).unwrap() else {
        panic!("a new flow starts its circuit")
    };
    assert_eq!(lost.next(), b, "the direct path comes first");
    drop(lost);
    tokio::time::sleep(Duration::from_millis(150)).await;
    f.a.maintain().unwrap();
    // The direct handshake ended unanswered: the relay carries the next one.
    let packet = f.mesh_udp(false, 42424);
    let Outbound::Queued { init: Some(init) } = f.a.outbound(packet).unwrap() else {
        panic!("a new circuit on another path")
    };
    assert_eq!(init.next(), r);
    f.finish_handshake(init).await;
    // Its pause (one handshake timeout) over, the direct path is tried again.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        f.a.choose_path([[a, b].as_slice(), [a, r, b].as_slice()]),
        Some(0)
    );
    f.close().await;
}

#[tokio::test]
async fn a_path_pause_survives_content_unrelated_to_that_path() {
    let mut f = direct_or_relayed().await;
    let [a, r, b] = f.ids();
    f.rebuild(
        meshway_dataplane::secure::Limits {
            handshake_timeout: Duration::from_millis(400),
            ..Default::default()
        },
        Default::default(),
    );
    let direct = |key: &str| meshway_dataplane::secure::Carrier {
        dialer: a,
        listener: b,
        listener_key: key.into(),
        protocol: meshway_model::TransportProtocol::NativeTcp,
    };
    f.a.set_carriers(BTreeMap::from([
        (r, f.ar.carrier.clone()),
        (b, direct("tcp")),
    ]))
    .unwrap();
    let Outbound::Queued { init: Some(lost) } = f.a.outbound(f.mesh_udp(false, 42424)).unwrap()
    else {
        panic!("a new flow starts its circuit")
    };
    assert_eq!(lost.next(), b);
    drop(lost);
    tokio::time::sleep(Duration::from_millis(450)).await;
    f.a.maintain().unwrap();
    let (direct_path, relayed) = ([a, b], [a, r, b]);
    let paths = [direct_path.as_slice(), relayed.as_slice()];
    assert_eq!(f.a.choose_path(paths), Some(1), "the direct path is paused");
    // Another service in Access changes A's content, not this path's.
    f.publish(|cfg, [a, _, b]| cfg.access.push(rule(a, &sel(b), &["udp:9"])))
        .await;
    assert_eq!(
        f.a.choose_path(paths),
        Some(1),
        "unrelated content keeps it"
    );
    // B's listener changes: the A–B carrier is another one now.
    f.publish(|cfg, [_, _, b]| {
        let node = cfg.nodes.get_mut(&b).unwrap();
        let listener = node.listeners.remove("tcp").unwrap();
        node.listeners.insert("tcp-2".into(), listener);
    })
    .await;
    f.a.set_carriers(BTreeMap::from([
        (r, f.ar.carrier.clone()),
        (b, direct("tcp-2")),
    ]))
    .unwrap();
    assert_eq!(
        f.a.choose_path(paths),
        Some(0),
        "this path's carrier changed"
    );
    f.close().await;
}

#[tokio::test]
async fn a_needed_peer_without_a_circuit_reports_the_relay_a_new_one_would_take() {
    let mut f = direct_or_relayed().await;
    let [a, r, b] = f.ids();
    // A has a channel with R only; nothing is established toward B.
    assert_eq!(f.a.relay_route(b), None, "R reports no channel with B yet");
    f.a.set_peer_channels(BTreeMap::from([(r, [b].into())]));
    assert_eq!(f.a.relay_route(b), Some(vec![a, r, b]));
    // Without a channel to the relay itself nothing reaches B.
    f.a.set_carriers(BTreeMap::new()).unwrap();
    assert_eq!(f.a.relay_route(b), None);
    f.close().await;
}

#[tokio::test]
async fn the_idle_route_is_the_path_a_new_flow_then_takes() {
    let mut f = direct_or_relayed().await;
    let [a, r, b] = f.ids();
    // Only R carries: the shown path is the relay the circuit starts on.
    let shown = f.a.idle_route(b);
    assert_eq!(shown, Some(vec![a, r, b]));
    let Outbound::Queued { init: Some(init) } = f.a.outbound(f.mesh_udp(false, 42424)).unwrap()
    else {
        panic!("a new flow starts its circuit")
    };
    assert_eq!(init.next(), r);
    f.finish_handshake(init).await;
    assert_eq!(f.a.joined_route(b), shown);
    f.close().await;
}

#[tokio::test]
async fn preview_and_joined_route_report_the_circuit_a_new_flow_takes() {
    use meshway_configuration::execution::BusinessProtocol;
    use meshway_dataplane::secure::Preview;
    let mut f = direct_or_relayed().await;
    let [a, r, b] = f.ids();
    let destination = f.address(2);
    let preview = |f: &mut NetmapMesh| {
        f.a.preview(
            destination,
            (BusinessProtocol::Udp, Some(42424.try_into().unwrap())),
        )
        .unwrap()
    };
    let Preview::Mesh {
        path, established, ..
    } = preview(&mut f)
    else {
        panic!("a Mesh decision")
    };
    assert_eq!(
        (path, established),
        (vec![a, r, b], false),
        "the circuit it would open"
    );
    assert!(
        f.a.take_adjacency_interests().is_empty(),
        "a preview wants no adjacency"
    );
    let Outbound::Queued { init: Some(init) } = f.a.outbound(f.mesh_udp(false, 42424)).unwrap()
    else {
        panic!("the relayed path starts at once")
    };
    f.finish_handshake(init).await;
    let Preview::Mesh {
        path, established, ..
    } = preview(&mut f)
    else {
        panic!("a Mesh decision")
    };
    assert_eq!(
        (path, established),
        (vec![a, r, b], true),
        "it joins the relay"
    );
    assert_eq!(f.a.joined_route(b), Some(vec![a, r, b]));
    // With a carrier to B a new flow opens the direct path: the relayed
    // circuit is no longer the one a new flow takes.
    let direct = meshway_dataplane::secure::Carrier {
        dialer: a,
        listener: b,
        listener_key: "tcp".into(),
        protocol: meshway_model::TransportProtocol::NativeTcp,
    };
    f.a.set_carriers(BTreeMap::from([(r, f.ar.carrier.clone()), (b, direct)]))
        .unwrap();
    let Preview::Mesh {
        path, established, ..
    } = preview(&mut f)
    else {
        panic!("a Mesh decision")
    };
    assert_eq!((path, established), (vec![a, b], false));
    assert_eq!(f.a.joined_route(b), None);
    f.close().await;
}

/// A roamed: its direct channel to B is gone while it acquires it again, and
/// R still carries. The established flow keeps its direct circuit and waits
/// for the adjacency; a new flow does not wait with it but goes through R.
#[tokio::test]
async fn a_new_flow_goes_through_the_relay_while_the_direct_circuit_has_no_channel() {
    use meshway_dataplane::secure::Received;
    let mut f = direct_or_relayed().await;
    let [a, r, b] = f.ids();
    let direct = meshway_dataplane::secure::Carrier {
        dialer: a,
        listener: b,
        listener_key: "tcp".into(),
        protocol: meshway_model::TransportProtocol::NativeTcp,
    };
    f.a.set_carriers(BTreeMap::from([
        (r, f.ar.carrier.clone()),
        (b, direct.clone()),
    ]))
    .unwrap();
    f.b.set_carriers(BTreeMap::from([
        (r, f.rb.carrier.clone()),
        (a, direct.clone()),
    ]))
    .unwrap();
    let mut ab = super::support::secure::Link::connect(
        tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap(),
        direct.clone(),
    )
    .await;
    let Outbound::Queued { init: Some(init) } = f.a.outbound(f.mesh_udp(false, 42424)).unwrap()
    else {
        panic!("the direct path starts at once")
    };
    assert_eq!(init.next(), b);
    let circuit = init.circuit();
    ab.dial.send(init).await;
    let first = ab.listen.recv().await;
    let Received::Established {
        response: Some(second),
        ..
    } = f.b.receive(&direct, first).unwrap()
    else {
        panic!("terminal handshake")
    };
    ab.listen.send(second).await;
    let second = ab.dial.recv().await;
    assert!(matches!(
        f.a.receive(&direct, second).unwrap(),
        Received::Established { circuit: source, response: None } if source == circuit
    ));
    let mut flushed = f.a.flush(32);
    let Ok(Flushed::Send(job)) = flushed.remove(0) else {
        panic!("queued packet")
    };
    assert_eq!(job.circuit(), circuit);
    drop(job);
    // A roamed: no channel to B, R still carries.
    f.a.set_carriers(BTreeMap::from([(r, f.ar.carrier.clone())]))
        .unwrap();
    // The established flow keeps its circuit and waits for B's adjacency.
    assert!(matches!(
        f.a.outbound(f.mesh_udp(false, 42424)).unwrap(),
        Outbound::AwaitingAdjacency
    ));
    let packet = super::packet::udp(f.address(0), f.address(2), 50001, 42424, b"new flow");
    let Outbound::Queued { init: Some(init) } = f.a.outbound(packet).unwrap() else {
        panic!("a new circuit through R")
    };
    assert_eq!(init.next(), r);
    drop(init);
    f.close().await;
}
