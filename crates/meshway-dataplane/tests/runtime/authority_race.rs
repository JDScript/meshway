//! Deterministic publication races at the production validation boundaries.
//! Only a scoped subscriber pauses the synchronous worker. Real Control/Server
//! operations invalidate its opaque snapshot; no clock or authority is forged.
use super::{
    datagram, local, peer,
    support::{Mesh, Node, packet, secure::SecureMesh},
};
use meshway_authority::DataPlaneIdentity;
use meshway_dataplane::{
    Engine, Error as EngineError, PendingAction,
    secure::{Carrier, Error, PendingFrame, Received, SecureNode},
};
use meshway_protocol::RenewDemandRequest;
use std::{
    sync::{Mutex, mpsc},
    time::Duration,
};
use tokio::sync::oneshot;
use tracing::{
    Event, Metadata, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
};

struct Boundary {
    target: &'static str,
    operation: &'static str,
    entered: Mutex<Option<oneshot::Sender<()>>>,
    resume: Mutex<mpsc::Receiver<()>>,
}
impl Subscriber for Boundary {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == self.target
    }
    fn register_callsite(&self, _: &'static Metadata<'static>) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        struct Operation<'a> {
            expected: &'a str,
            matched: bool,
        }
        impl Visit for Operation<'_> {
            fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
            fn record_str(&mut self, field: &Field, value: &str) {
                if field.name() == "operation" && value == self.expected {
                    self.matched = true;
                }
            }
        }
        let mut operation = Operation {
            expected: self.operation,
            matched: false,
        };
        event.record(&mut operation);
        if operation.matched
            && let Some(entered) = self.entered.lock().unwrap().take()
        {
            let _ = entered.send(());
            self.resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .expect("bounded real Control handoff must release worker");
        }
    }
}

fn boundary(
    target: &'static str,
    operation: &'static str,
) -> (Boundary, oneshot::Receiver<()>, mpsc::Sender<()>) {
    let (entered, arrived) = oneshot::channel();
    let (resume, released) = mpsc::channel();
    (
        Boundary {
            target,
            operation,
            entered: Mutex::new(Some(entered)),
            resume: Mutex::new(released),
        },
        arrived,
        resume,
    )
}

async fn identity(node: &mut Node) -> DataPlaneIdentity {
    let snapshot = node.sync_current().await;
    node.control.dataplane_identity(snapshot).unwrap()
}

async fn renew(mesh: &mut Mesh) -> DataPlaneIdentity {
    let lease = mesh.a.control.renew_instance().await.unwrap();
    mesh.a
        .control
        .renew_demand(&RenewDemandRequest {
            instance: mesh.a.instance(),
            request_id: mesh.demand,
            expires_at_ms: lease.expires_at_ms,
        })
        .await
        .unwrap();
    identity(&mut mesh.a).await
}

fn bytes(pending: PendingFrame) -> Vec<u8> {
    pending.release(|bytes| bytes.into_vec()).unwrap()
}
fn delivery(node: &mut SecureNode, carrier: &Carrier, wire: Vec<u8>) -> PendingAction {
    let Received::Packet(packet) = node.receive(carrier, wire).unwrap() else {
        panic!("endpoint packet");
    };
    packet
}
fn forwarded(node: &mut SecureNode, carrier: &Carrier, wire: Vec<u8>) -> Vec<u8> {
    let Received::Forward(packet) = node.receive(carrier, wire).unwrap() else {
        panic!("opaque relay");
    };
    bytes(packet)
}
fn request(fixture: &mut SecureMesh, cid: [u8; 32]) {
    let wire = bytes(
        fixture
            .a
            .send(cid, datagram(&fixture.mesh, false, false))
            .unwrap(),
    );
    let wire = forwarded(&mut fixture.r, &fixture.ar.carrier, wire);
    delivery(&mut fixture.b, &fixture.rb.carrier, wire)
        .release(|_| ())
        .unwrap();
}
fn response(fixture: &mut SecureMesh, cid: [u8; 32]) -> Vec<u8> {
    let wire = bytes(
        fixture
            .b
            .send(cid, datagram(&fixture.mesh, false, true))
            .unwrap(),
    );
    forwarded(&mut fixture.r, &fixture.rb.carrier, wire)
}

#[derive(Clone, Copy)]
enum Operation {
    Receive,
    Send,
    Maintain,
}

async fn live_handoff(operation: Operation) {
    let mut fixture = SecureMesh::start().await;
    let cid = fixture.handshake().await;
    request(&mut fixture, cid);
    let wire = response(&mut fixture, cid);
    let queued_delivery = delivery(&mut fixture.a, &fixture.ar.carrier, wire);
    let queued_frame = fixture
        .a
        .send(cid, datagram(&fixture.mesh, false, false))
        .unwrap();
    let race_wire = response(&mut fixture, cid);
    let old = fixture.mesh.a.control.execution_snapshot().unwrap();
    let packet = datagram(&fixture.mesh, false, false);
    let incoming = fixture.ar.carrier.clone();
    let replay = race_wire.clone();
    let SecureMesh {
        mut mesh,
        mut a,
        mut r,
        mut b,
        ar,
        rb,
    } = fixture;
    let name = match operation {
        Operation::Receive => "receive",
        Operation::Send => "send",
        Operation::Maintain => "maintain",
    };
    let (gate, arrived, resume) = boundary("meshway_dataplane::secure::authority_boundary", name);
    let worker = tokio::task::spawn_blocking(move || {
        let result = tracing::subscriber::with_default(gate, || match operation {
            Operation::Receive => a.receive(&incoming, race_wire).map(|_| ()),
            Operation::Send => a.send(cid, packet).map(|_| ()),
            Operation::Maintain => a.maintain(),
        });
        (a, result)
    });
    tokio::time::timeout(Duration::from_secs(10), arrived)
        .await
        .unwrap()
        .unwrap();
    old.ensure_current().unwrap();
    let fresh = renew(&mut mesh).await;
    assert!(old.ensure_current().is_err());
    assert!(
        queued_frame
            .release(|_| panic!("old ciphertext escaped"))
            .is_err()
    );
    assert!(
        queued_delivery
            .release(|_| panic!("old plaintext escaped"))
            .is_err()
    );
    resume.send(()).unwrap();
    let (mut a, result) = worker.await.unwrap();
    assert_eq!(result, Err(Error::Authority));
    assert!(a.ready_circuits().is_err());
    assert_eq!(a.maintain(), Err(Error::Authority));
    assert_eq!(a.retained_flows(), 1);
    assert!(a.send(cid, datagram(&mesh, false, false)).is_err());
    assert_eq!(a.retained_flows(), 1);
    a.reconcile(fresh).unwrap();
    assert_eq!(a.ready_circuits().unwrap()[0].id, cid);
    match operation {
        Operation::Receive => assert!(matches!(a.receive(&ar.carrier, replay), Err(Error::Replay))),
        Operation::Send | Operation::Maintain => {
            delivery(&mut a, &ar.carrier, replay)
                .release(|_| ())
                .unwrap();
        }
    }
    r.reconcile(identity(&mut mesh.r).await).unwrap();
    b.reconcile(identity(&mut mesh.b).await).unwrap();
    // These nonces must continue after every already-produced/consumed frame.
    let wire = bytes(a.send(cid, datagram(&mesh, false, false)).unwrap());
    let wire = forwarded(&mut r, &ar.carrier, wire);
    delivery(&mut b, &rb.carrier, wire).release(|_| ()).unwrap();
    let wire = bytes(b.send(cid, datagram(&mesh, false, true)).unwrap());
    let wire = forwarded(&mut r, &rb.carrier, wire);
    delivery(&mut a, &ar.carrier, wire).release(|_| ()).unwrap();
    for (index, node) in [&a, &r, &b].into_iter().enumerate() {
        assert_eq!(node.retained_flows(), usize::from(index != 1));
        assert_eq!(node.ready_circuits().unwrap()[0].id, cid);
    }
    SecureMesh {
        mesh,
        a,
        r,
        b,
        ar,
        rb,
    }
    .close()
    .await;
}

#[tokio::test]
async fn receive_postcheck_authority_race_preserves_nonce_and_replay_under_real_successor() {
    live_handoff(Operation::Receive).await;
}
#[tokio::test]
async fn send_postcheck_authority_race_rejects_output_and_preserves_live_cipher() {
    live_handoff(Operation::Send).await;
}
#[tokio::test]
async fn maintenance_authority_race_does_not_retire_circuits_or_clear_flows() {
    live_handoff(Operation::Maintain).await;
}

#[tokio::test]
async fn engine_commit_authority_race_discards_prepared_flow_but_retains_prior_flow() {
    let mut mesh = Mesh::start().await;
    let old = mesh.a.control.execution_snapshot().unwrap();
    let mut engine = Engine::new(old.clone(), Default::default()).unwrap();
    let queued = engine
        .process(local(&mesh), datagram(&mesh, false, false))
        .unwrap();
    let addresses = |node: &Node| {
        node.control
            .execution_snapshot()
            .unwrap()
            .configuration()
            .unwrap()
            .addresses
            .ipv4
            .unwrap()
    };
    let new_packet = packet::udp(
        addresses(&mesh.a).into(),
        addresses(&mesh.b).into(),
        50001,
        42424,
        b"uncommitted",
    );
    let uncommitted_reply = packet::udp(
        addresses(&mesh.b).into(),
        addresses(&mesh.a).into(),
        42424,
        50001,
        b"no state",
    );
    let ingress = local(&mesh);
    let (gate, arrived, resume) = boundary("meshway_dataplane::flow::authority_boundary", "commit");
    let worker = tokio::task::spawn_blocking(move || {
        let result =
            tracing::subscriber::with_default(gate, || engine.process(ingress, new_packet));
        (engine, result)
    });
    tokio::time::timeout(Duration::from_secs(10), arrived)
        .await
        .unwrap()
        .unwrap();
    let fresh = renew(&mut mesh).await;
    assert!(
        queued
            .release(|_| panic!("old engine output escaped"))
            .is_err()
    );
    resume.send(()).unwrap();
    let (mut engine, result) = worker.await.unwrap();
    assert!(matches!(result, Err(EngineError::Authority(_))));
    assert!(engine.maintain().is_err());
    assert_eq!(engine.retained_flows(), 1);
    engine.reconcile(fresh.snapshot().clone()).unwrap();
    assert!(
        engine
            .process(peer(&mesh, mesh.b.id, mesh.r.id), uncommitted_reply)
            .is_err()
    );
    engine
        .process(
            peer(&mesh, mesh.b.id, mesh.r.id),
            datagram(&mesh, false, true),
        )
        .unwrap()
        .release(|_| ())
        .unwrap();
    assert_eq!(engine.retained_flows(), 1);
    mesh.close().await;
}

#[tokio::test]
async fn authority_race_cannot_resurrect_circuit_after_observed_withdrawal_and_regrant() {
    let mut fixture = SecureMesh::start().await;
    let cid = fixture.handshake().await;
    request(&mut fixture, cid);
    let wire = response(&mut fixture, cid);
    let replay = wire.clone();
    let incoming = fixture.ar.carrier.clone();
    let SecureMesh {
        mut mesh,
        mut a,
        r,
        b,
        ar,
        rb,
    } = fixture;
    let (gate, arrived, resume) =
        boundary("meshway_dataplane::secure::authority_boundary", "receive");
    let worker = tokio::task::spawn_blocking(move || {
        let result = tracing::subscriber::with_default(gate, || a.receive(&incoming, wire));
        (a, result)
    });
    tokio::time::timeout(Duration::from_secs(10), arrived)
        .await
        .unwrap()
        .unwrap();
    let mut access = Vec::new();
    mesh.server
        .configure(&mesh.owner, |input| {
            access = input.access.clone();
            input.access.clear();
        })
        .await;
    let withdrawn = identity(&mut mesh.a).await;
    resume.send(()).unwrap();
    let (mut a, result) = worker.await.unwrap();
    assert!(matches!(result, Err(Error::Authority)));
    a.reconcile(withdrawn).unwrap();
    assert!(a.ready_circuits().unwrap().is_empty());
    assert_eq!(a.retained_flows(), 0);
    mesh.server
        .configure(&mesh.owner, |input| input.access = access)
        .await;
    a.reconcile(identity(&mut mesh.a).await).unwrap();
    assert!(a.ready_circuits().unwrap().is_empty());
    assert!(matches!(a.receive(&ar.carrier, replay), Err(Error::State)));
    assert_eq!(a.retained_flows(), 0);
    SecureMesh {
        mesh,
        a,
        r,
        b,
        ar,
        rb,
    }
    .close()
    .await;
}
