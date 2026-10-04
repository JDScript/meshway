//! Real HTTP enrollment/configuration/discovery feeds production opaque execution
//! snapshots into the plaintext in-process policy core. Peer identity below is
//! trusted test-adapter input: these tests prove neither wire authentication nor
//! end-to-end encryption, installed UDP transport, daemon execution or TUN I/O.
#![cfg(unix)]

mod support;

use meshway_dataplane::{
    Action, AuthenticatedPeer, Direction, Engine, Hop, Ingress, Limits, Packet, PendingAction,
};
use meshway_model::NodeId;
use std::{net::IpAddr, time::Duration};
use support::{Mesh, Node, Server, WAIT, dns, packet};

fn engine(node: &Node) -> Engine {
    Engine::new(
        node.control.execution_snapshot().unwrap(),
        Limits::default(),
    )
    .unwrap()
}

#[tokio::test]
async fn published_mesh_dns_never_turns_relay_or_reply_rights_into_name_visibility() {
    use hickory_proto::rr::RecordType;
    use meshway_dns::{Begin, ExitSelection, Owner, Unavailable};
    let mut mesh = Mesh::start().await;
    let name = |node: &Node| {
        let guard = node.control.execution_snapshot().unwrap();
        let cfg = guard.configuration().unwrap();
        meshway_configuration::node_dns_name(&cfg.hostname, &cfg.dns.settings.domain).unwrap()
    };
    let (a_name, r_name, b_name) = (name(&mesh.a), name(&mesh.r), name(&mesh.b));
    let limits = meshway_dns::Limits {
        max_pending: 4,
        query_timeout: Duration::from_secs(5),
    };
    let mut a_dns = Owner::new(mesh.a.control.execution_snapshot().unwrap(), limits).unwrap();
    assert!(matches!(
        a_dns.begin(dns::query(&b_name, RecordType::A), ExitSelection::NoExit),
        Begin::Answer(_)
    ));
    let lookup = dns::discover(&mut a_dns, &r_name, RecordType::A);
    let response = mesh.a.control.query_dns(&lookup.name).await.unwrap();
    assert!(response.record.is_none());
    assert!(matches!(
        a_dns.complete_discovery(&lookup.ticket, response),
        Begin::Unavailable(Unavailable::NotVisible)
    ));
    let mut b_dns = Owner::new(mesh.b.control.execution_snapshot().unwrap(), limits).unwrap();
    let lookup = dns::discover(&mut b_dns, &a_name, RecordType::A);
    let response = mesh.b.control.query_dns(&lookup.name).await.unwrap();
    assert!(response.record.is_none());
    assert!(matches!(
        b_dns.complete_discovery(&lookup.ticket, response),
        Begin::Unavailable(Unavailable::NotVisible)
    ));
    mesh.close().await;
}

#[tokio::test]
async fn acknowledged_demand_cancellation_fences_old_outputs_before_target_resync() {
    use meshway_protocol::CancelDemandRequest;
    let mut mesh = Mesh::start().await;
    let mut source = engine(&mesh.a);
    let snapshot = mesh.a.control.execution_snapshot().unwrap();
    let pending = source
        .process(local(&mesh), datagram(&mesh, false, false))
        .unwrap();
    let foreign = CancelDemandRequest {
        instance: mesh.b.instance(),
        request_id: mesh.demand,
    };
    assert!(matches!(
        mesh.a.control.cancel_demand(&foreign).await,
        Err(meshway_control::Error::Lifecycle)
    ));
    snapshot.ensure_current().unwrap();
    let request = CancelDemandRequest {
        instance: mesh.a.instance(),
        request_id: mesh.demand,
    };
    mesh.a.control.cancel_demand(&request).await.unwrap();
    tokio::time::timeout(WAIT, pending.invalidated())
        .await
        .unwrap();
    let mut released = false;
    assert!(pending.release(|_| released = true).is_err());
    assert!(!released);
    assert!(mesh.a.control.execution_snapshot().is_err());
    assert!(source.maintain().is_err());
    mesh.a.sync_current().await;
    let current = mesh.a.control.execution_snapshot().unwrap();
    assert!(
        !current
            .configuration()
            .unwrap()
            .permits
            .contains_key(&mesh.demand)
    );
    assert!(snapshot.ensure_current().is_err());
    let mut fresh = engine(&mesh.a);
    assert!(
        fresh
            .process(local(&mesh), datagram(&mesh, false, false))
            .is_err()
    );
    mesh.close().await;
}

fn local(mesh: &Mesh) -> Ingress {
    Ingress::Local {
        demand: mesh.demand,
        path: mesh.path.clone(),
    }
}

fn peer(mesh: &Mesh, sender: NodeId, previous: NodeId) -> Ingress {
    Ingress::Peer {
        demand: mesh.demand,
        path: mesh.path.clone(),
        identity: AuthenticatedPeer {
            origin: mesh.a.id,
            sender,
            previous,
        },
    }
}

fn datagram(mesh: &Mesh, ipv6: bool, reply: bool) -> Packet {
    let address = |node: &Node| -> IpAddr {
        let snapshot = node.control.execution_snapshot().unwrap();
        let addresses = &snapshot.configuration().unwrap().addresses;
        if ipv6 {
            addresses.ipv6.expect("assigned IPv6 address").into()
        } else {
            addresses.ipv4.expect("assigned IPv4 address").into()
        }
    };
    if reply {
        packet::udp(address(&mesh.b), address(&mesh.a), 42424, 50000, b"reply")
    } else {
        packet::udp(address(&mesh.a), address(&mesh.b), 50000, 42424, b"forward")
    }
}

fn release(mesh: &Mesh, pending: PendingAction, hop: Hop, direction: Direction) -> Packet {
    assert_eq!(pending.hop(), hop);
    assert_eq!(pending.route().demand, mesh.demand);
    assert_eq!(pending.route().origin, mesh.a.id);
    assert_eq!(pending.route().original_path, mesh.path);
    assert_eq!(pending.route().direction, direction);
    pending
        .release(|action| match action {
            Action::Forward { packet, .. } | Action::Deliver { packet, .. } => packet,
            Action::Exit { .. } => panic!("Mesh destination must not become an exit"),
        })
        .unwrap()
}

#[tokio::test]
async fn real_http_permit_drives_plaintext_ipv4_ipv6_full_path_and_stateful_replies() {
    let mesh = Mesh::start().await;
    let mut a = engine(&mesh.a);
    let mut r = engine(&mesh.r);
    let mut b = engine(&mesh.b);
    for ipv6 in [false, true] {
        // A genuine permit still cannot authorize a reply without local forward
        // facts. Endpoint, relay and origin all enforce their own flow state.
        assert!(
            b.process(local(&mesh), datagram(&mesh, ipv6, true))
                .is_err()
        );
        assert!(
            r.process(
                peer(&mesh, mesh.b.id, mesh.b.id),
                datagram(&mesh, ipv6, true)
            )
            .is_err()
        );
        assert!(
            a.process(
                peer(&mesh, mesh.b.id, mesh.r.id),
                datagram(&mesh, ipv6, true)
            )
            .is_err()
        );
        // Keeping the declared full path while skipping the relay is rejected.
        assert!(
            b.process(
                peer(&mesh, mesh.a.id, mesh.a.id),
                datagram(&mesh, ipv6, false)
            )
            .is_err()
        );
        // Omitting the mandatory relay from the declared path is also rejected.
        assert!(
            b.process(
                Ingress::Peer {
                    demand: mesh.demand,
                    path: vec![mesh.a.id, mesh.b.id],
                    identity: AuthenticatedPeer {
                        origin: mesh.a.id,
                        sender: mesh.a.id,
                        previous: mesh.a.id,
                    },
                },
                datagram(&mesh, ipv6, false),
            )
            .is_err()
        );
        assert!(
            r.process(
                peer(&mesh, mesh.a.id, mesh.b.id),
                datagram(&mesh, ipv6, false)
            )
            .is_err()
        );
        // This tests trusted-adapter identity consistency, not signature proof.
        assert!(
            b.process(
                peer(&mesh, mesh.r.id, mesh.r.id),
                datagram(&mesh, ipv6, false)
            )
            .is_err()
        );

        let forward = datagram(&mesh, ipv6, false);
        let expected = datagram(&mesh, ipv6, false).into_bytes();
        let forward = release(
            &mesh,
            a.process(local(&mesh), forward).unwrap(),
            Hop::Forward(mesh.r.id),
            Direction::Forward,
        );
        let forward = release(
            &mesh,
            r.process(peer(&mesh, mesh.a.id, mesh.a.id), forward)
                .unwrap(),
            Hop::Forward(mesh.b.id),
            Direction::Forward,
        );
        let delivered = release(
            &mesh,
            b.process(peer(&mesh, mesh.a.id, mesh.r.id), forward)
                .unwrap(),
            Hop::Deliver,
            Direction::Forward,
        );
        assert_eq!(delivered.into_bytes(), expected);

        let reply = datagram(&mesh, ipv6, true);
        let expected = datagram(&mesh, ipv6, true).into_bytes();
        let reply = release(
            &mesh,
            b.process(local(&mesh), reply).unwrap(),
            Hop::Forward(mesh.r.id),
            Direction::Reply,
        );
        assert!(
            r.process(
                peer(&mesh, mesh.a.id, mesh.b.id),
                datagram(&mesh, ipv6, true)
            )
            .is_err()
        );
        assert!(
            a.process(
                peer(&mesh, mesh.b.id, mesh.b.id),
                datagram(&mesh, ipv6, true)
            )
            .is_err()
        );
        let reply = release(
            &mesh,
            r.process(peer(&mesh, mesh.b.id, mesh.b.id), reply).unwrap(),
            Hop::Forward(mesh.a.id),
            Direction::Reply,
        );
        let delivered = release(
            &mesh,
            a.process(peer(&mesh, mesh.b.id, mesh.r.id), reply).unwrap(),
            Hop::Deliver,
            Direction::Reply,
        );
        assert_eq!(delivered.into_bytes(), expected);
    }
    assert_eq!(a.retained_flows(), 2);
    assert_eq!(r.retained_flows(), 2);
    assert_eq!(b.retained_flows(), 2);
    mesh.close().await;
}

#[tokio::test]
async fn real_http_access_revocation_fences_synced_receiver_and_queued_delivery() {
    let mut mesh = Mesh::start().await;
    let mut b = engine(&mesh.b);
    let old = mesh.b.control.execution_snapshot().unwrap();
    let unsynced_origin = mesh.a.control.execution_snapshot().unwrap();
    let queued = b
        .process(
            peer(&mesh, mesh.a.id, mesh.r.id),
            datagram(&mesh, false, false),
        )
        .unwrap();
    assert_eq!(queued.hop(), Hop::Deliver);
    mesh.server
        .configure(&mesh.owner, |input| input.access.clear())
        .await;
    mesh.b.sync_current().await;
    tokio::time::timeout(WAIT, old.invalidated()).await.unwrap();
    tokio::time::timeout(WAIT, queued.invalidated())
        .await
        .unwrap();
    let mut sent = false;
    assert!(queued.release(|_| sent = true).is_err());
    assert!(!sent);
    assert!(
        b.process(
            peer(&mesh, mesh.a.id, mesh.r.id),
            datagram(&mesh, false, false)
        )
        .is_err()
    );
    assert_eq!(b.retained_flows(), 0);
    let fresh = mesh.b.control.execution_snapshot().unwrap();
    assert!(!old.same_execution(&fresh));
    assert!(
        !fresh
            .configuration()
            .unwrap()
            .permits
            .contains_key(&mesh.demand)
    );
    assert!(b.refresh(fresh.clone()).is_err());
    let mut replacement = Engine::new(fresh, Limits::default()).unwrap();
    assert!(
        replacement
            .process(
                peer(&mesh, mesh.a.id, mesh.r.id),
                datagram(&mesh, false, false)
            )
            .is_err()
    );
    // No global instantaneous revocation claim: only B consumed the new target.
    unsynced_origin.ensure_current().unwrap();
    mesh.close().await;
}

#[tokio::test]
async fn real_control_renewal_preserves_flows_but_not_queued_deadlines_and_stop_wakes() {
    let mut mesh = Mesh::start().await;
    let original = mesh.a.control.execution_snapshot().unwrap();
    let original_deadline = original.expires_at();
    let mut a = engine(&mesh.a);
    let queued = a
        .process(local(&mesh), datagram(&mesh, false, false))
        .unwrap();
    mesh.a.control.renew_instance().await.unwrap();
    let renewed = mesh.a.control.execution_snapshot().unwrap();
    assert!(original.same_execution(&renewed));
    assert_eq!(original.reference(), renewed.reference());
    assert_eq!(original.expires_at(), original_deadline);
    assert!(renewed.expires_at() >= original_deadline);
    a.refresh(renewed.clone()).unwrap();
    assert_eq!(a.retained_flows(), 1);
    queued.ensure_current().unwrap();
    assert!(
        a.refresh(mesh.b.control.execution_snapshot().unwrap())
            .is_err()
    );
    assert_eq!(a.retained_flows(), 1);

    let reply = a
        .process(
            peer(&mesh, mesh.b.id, mesh.r.id),
            datagram(&mesh, false, true),
        )
        .unwrap();
    let delivered = release(&mesh, reply, Hop::Deliver, Direction::Reply);
    assert_eq!(
        delivered.into_bytes(),
        datagram(&mesh, false, true).into_bytes()
    );

    // A deliberately short retention lifetime is not a performance threshold.
    // Renewing the engine cannot move a previously queued action's deadline.
    let mut idle = Engine::new(
        original.clone(),
        Limits {
            max_flows: 4,
            idle_timeout: Duration::from_millis(30),
        },
    )
    .unwrap();
    let short = idle
        .process(local(&mesh), datagram(&mesh, false, false))
        .unwrap();
    idle.refresh(renewed).unwrap();
    tokio::time::timeout(WAIT, short.invalidated())
        .await
        .unwrap();
    let mut sent = false;
    assert!(short.release(|_| sent = true).is_err());
    assert!(!sent);
    idle.maintain().unwrap();
    assert_eq!(idle.retained_flows(), 0);

    // Waiters run concurrently with stop and need no status/configuration poll.
    tokio::time::timeout(WAIT, async {
        let (stopped, (), (), ()) = tokio::join!(
            mesh.a.control.stop(),
            original.invalidated(),
            queued.invalidated(),
            a.invalidated(),
        );
        stopped.unwrap();
    })
    .await
    .unwrap();
    assert_eq!(a.retained_flows(), 0);
    assert!(queued.release(|_| sent = true).is_err());
    assert!(!sent);
    mesh.close().await;
}

#[tokio::test]
async fn real_control_dns_discovery_binds_wire_answers_and_preserves_typed_not_visible() {
    use hickory_proto::{
        op::{Message, ResponseCode},
        rr::{RData, RecordType},
    };
    use meshway_dns::{Begin, Owner, ResponseTransport, Unavailable};
    let server = Server::start().await;
    let user = server.owner().await;
    let mut node = server.enroll(&user, "dns-self").await;
    server.configure(&user, |input| input.access.clear()).await;
    // A changed configuration is saved successfully without minting a run
    // permit. The following host helper must explicitly renew that lease.
    node.control.sync_configuration().await.unwrap().unwrap();
    assert!(matches!(
        node.control.execution_snapshot(),
        Err(meshway_control::Error::Expired)
    ));
    let snapshot = node.sync_current().await;
    let configuration = snapshot.configuration().unwrap();
    let name = meshway_configuration::node_dns_name(
        &configuration.hostname,
        &configuration.dns.settings.domain,
    )
    .unwrap();
    assert!(!configuration.dns.mesh_hosts.contains_key(&name));
    let addresses = configuration.addresses.clone();
    let absent = format!("absent.{}", configuration.dns.settings.domain);
    let mut owner = Owner::new(
        snapshot.clone(),
        meshway_dns::Limits {
            max_pending: 4,
            query_timeout: Duration::from_secs(10),
        },
    )
    .unwrap();
    for kind in [RecordType::A, RecordType::AAAA] {
        let request = dns::discover(&mut owner, &name, kind);
        let lease = node.control.status().instance;
        let response = node.control.query_dns(&request.name).await.unwrap();
        assert_eq!(node.control.status().instance, lease);
        let answer = dns::answer(owner.complete_discovery(&request.ticket, response));
        let bytes = answer.encode(ResponseTransport::Udp, 1232).unwrap();
        let message = Message::from_vec(&bytes).unwrap();
        assert_eq!(message.metadata.id, 123);
        assert_eq!(message.metadata.response_code, ResponseCode::NoError);
        assert_eq!(message.queries.len(), 1);
        assert_eq!(message.queries[0], *dns::query(&name, kind).question());
        assert_eq!(message.answers.len(), 1);
        let record = &message.answers[0];
        assert_eq!(record.ttl, 0);
        match (kind, &record.data) {
            (RecordType::A, RData::A(value)) => assert_eq!(Some(value.0), addresses.ipv4),
            (RecordType::AAAA, RData::AAAA(value)) => assert_eq!(Some(value.0), addresses.ipv6),
            other => panic!("unexpected self address: {other:?}"),
        }
        assert!(message.authorities.is_empty());
        // Completion consumes the ticket, while cancellation still cuts an
        // already constructed but unsent answer from that ticket.
        assert!(!owner.cancel(&request.ticket));
        tokio::time::timeout(WAIT, answer.invalidated())
            .await
            .unwrap();
        assert!(answer.encode(ResponseTransport::Udp, 1232).is_err());
    }
    let missing = dns::discover(&mut owner, &absent, RecordType::A);
    let response = node.control.query_dns(&missing.name).await.unwrap();
    assert!(response.record.is_none());
    assert!(matches!(
        owner.complete_discovery(&missing.ticket, response),
        Begin::Unavailable(Unavailable::NotVisible)
    ));

    let wrong = dns::discover(&mut owner, &name, RecordType::A);
    let mut response = node.control.query_dns(&wrong.name).await.unwrap();
    // Corrupt only the typed completion fixture; no fake credential or lease.
    response.source_node_id = NodeId::new(support::request().into_uuid()).unwrap();
    assert!(matches!(
        owner.complete_discovery(&wrong.ticket, response),
        Begin::Unavailable(Unavailable::BindingMismatch)
    ));
    let request = dns::discover(&mut owner, &name, RecordType::A);
    let response = node.control.query_dns(&request.name).await.unwrap();
    let answer = dns::answer(owner.complete_discovery(&request.ticket, response));
    // No outstanding Control operation/task exists here. Losing the unique
    // credential/lease owner must still fence this already completed answer.
    drop(node.control);
    tokio::time::timeout(WAIT, answer.invalidated())
        .await
        .unwrap();
    assert!(answer.ensure_current().is_err());
    assert!(answer.encode(ResponseTransport::Udp, 1232).is_err());
    server.close().await;
}

#[tokio::test]
async fn dns_owner_refresh_replace_drop_and_control_stop_fence_late_real_completions() {
    use hickory_proto::rr::RecordType;
    use meshway_dns::{Begin, Owner, ResponseTransport, Unavailable};
    let server = Server::start().await;
    let user = server.owner().await;
    let mut node = server.enroll(&user, "dns-lifecycle").await;
    server.configure(&user, |input| input.access.clear()).await;
    let original = node.sync_current().await;
    let configuration = original.configuration().unwrap();
    let name = meshway_configuration::node_dns_name(
        &configuration.hostname,
        &configuration.dns.settings.domain,
    )
    .unwrap();
    let limits = meshway_dns::Limits {
        max_pending: 4,
        query_timeout: Duration::from_secs(10),
    };
    let mut owner = Owner::new(original.clone(), limits).unwrap();
    let request = dns::discover(&mut owner, &name, RecordType::A);
    let response = node.control.query_dns(&request.name).await.unwrap();
    let unsent = dns::answer(owner.complete_discovery(&request.ticket, response.clone()));
    let pending = dns::discover(&mut owner, &name, RecordType::A);
    let late_response = node.control.query_dns(&pending.name).await.unwrap();
    node.control.renew_instance().await.unwrap();
    let renewed = node.control.execution_snapshot().unwrap();
    assert!(original.same_execution(&renewed));
    owner.refresh(renewed.clone()).unwrap();
    assert_eq!(owner.pending_len(), 0);
    tokio::time::timeout(WAIT, pending.ticket.cancelled())
        .await
        .unwrap();
    tokio::time::timeout(WAIT, unsent.invalidated())
        .await
        .unwrap();
    assert!(unsent.encode(ResponseTransport::Udp, 1232).is_err());
    assert!(matches!(
        owner.complete_discovery(&pending.ticket, late_response.clone()),
        Begin::Unavailable(Unavailable::InvalidTicket)
    ));

    let request = dns::discover(&mut owner, &name, RecordType::A);
    let unsent = dns::answer(owner.complete_discovery(&request.ticket, response.clone()));
    let pending = dns::discover(&mut owner, &name, RecordType::A);
    owner.replace(renewed.clone()).unwrap();
    tokio::time::timeout(WAIT, pending.ticket.cancelled())
        .await
        .unwrap();
    assert!(unsent.ensure_current().is_err());
    assert!(matches!(
        owner.complete_discovery(&pending.ticket, late_response.clone()),
        Begin::Unavailable(Unavailable::InvalidTicket)
    ));

    let request = dns::discover(&mut owner, &name, RecordType::A);
    let unsent = dns::answer(owner.complete_discovery(&request.ticket, response.clone()));
    drop(owner);
    tokio::time::timeout(WAIT, unsent.invalidated())
        .await
        .unwrap();
    assert!(unsent.encode(ResponseTransport::Udp, 1232).is_err());

    let mut owner = Owner::new(renewed, limits).unwrap();
    let request = dns::discover(&mut owner, &name, RecordType::A);
    let unsent = dns::answer(owner.complete_discovery(&request.ticket, response));
    let pending = dns::discover(&mut owner, &name, RecordType::A);
    tokio::time::timeout(WAIT, async {
        let (stopped, ()) = tokio::join!(node.control.stop(), unsent.invalidated());
        stopped.unwrap();
    })
    .await
    .unwrap();
    assert!(unsent.ensure_current().is_err());
    assert!(unsent.encode(ResponseTransport::Udp, 1232).is_err());
    assert!(matches!(
        owner.complete_discovery(&pending.ticket, late_response),
        Begin::Unavailable(Unavailable::Inactive)
    ));
    node.control.shutdown().await.unwrap();
    server.close().await;
}
