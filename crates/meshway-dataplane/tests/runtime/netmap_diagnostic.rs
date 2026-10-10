//! `meshway ping` on netmap snapshots: decided locally with `icmpEcho`, run
//! on observable paths only, never waiting for a Server round trip.
use super::support::netmap::{NetmapMesh, rule, sel};
use meshway_dataplane::secure::{
    CircuitGrant, Error, PendingFrame, Received, Target, Update, diagnostic_paths,
};
use meshway_model::{Digest, EventId};
use std::time::{Duration, Instant};

fn wire(frame: PendingFrame) -> Vec<u8> {
    frame.release(|bytes| bytes.into_vec()).unwrap()
}

fn measurement() -> EventId {
    EventId::new(super::support::request().into_uuid()).unwrap()
}

async fn ping(f: &mut NetmapMesh) -> [u8; 32] {
    let b = f.ids()[2];
    let init = f.a.start_diagnostic(b, None).unwrap().unwrap();
    let id = f.finish_handshake(init).await;
    let info = f.a.diagnostic_circuit(b).unwrap();
    assert_eq!(info.id, id);
    assert_eq!(
        info.grant,
        CircuitGrant {
            purpose: meshway_dataplane::Purpose::Diagnostic,
            target: Target::Mesh { node: b },
        }
    );
    id
}

fn probe(f: &mut NetmapMesh, id: [u8; 32]) -> Result<(), Error> {
    let mid = measurement();
    let challenge = Digest::from_bytes([7; 32]);
    let request = f.a.probe(
        id,
        mid,
        challenge,
        Instant::now() + Duration::from_secs(5),
        &f.ar.carrier.clone(),
    )?;
    let Received::Forward(request) = f.r.receive(&f.ar.carrier, wire(request))? else {
        panic!("relay")
    };
    let Received::ProbeRequest { response, .. } = f.b.receive(&f.rb.carrier, wire(request))? else {
        panic!("terminal probe")
    };
    let Received::Forward(response) = f.r.receive(&f.rb.carrier, wire(response))? else {
        panic!("relay")
    };
    let Received::ProbeReply {
        circuit,
        measurement_id,
        challenge: returned,
    } = f.a.receive(&f.ar.carrier, wire(response))?
    else {
        panic!("probe reply")
    };
    assert_eq!((circuit, measurement_id, returned), (id, mid, challenge));
    Ok(())
}

#[tokio::test]
async fn netmap_ping_is_local_and_runs_on_the_observable_path() {
    let mut f = NetmapMesh::start().await;
    let calls = f.sync_calls();
    let [_, _, b] = f.ids();
    let snapshot = f.nodes[0].identity().snapshot().clone();
    assert_eq!(
        diagnostic_paths(&snapshot, b, None).unwrap(),
        vec![f.path()]
    );
    let id = ping(&mut f).await;
    probe(&mut f, id).unwrap();
    // Diagnostic circuits never carry business packets.
    let packet = f.mesh_udp(false, 42424);
    assert!(f.a.send(id, packet).is_err());
    // Same-content renewal keeps the diagnostic circuit and its cipher.
    assert_eq!(f.renew().await, [Update::Renewed; 3]);
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    probe(&mut f, id).unwrap();
    assert_eq!(
        f.sync_calls(),
        calls + 3,
        "only the explicit renewals called Control"
    );
    f.close().await;
}

#[tokio::test]
async fn netmap_ping_denied_without_icmp_and_revocation_ends_the_circuit() {
    let mut f = NetmapMesh::start().await;
    let id = ping(&mut f).await;
    let updates = f
        .publish(|cfg, [a, _, b]| cfg.access[0] = rule(a, &sel(b), &["udp:42424"]))
        .await;
    for update in [updates[0], updates[2]] {
        assert!(
            matches!(
                update,
                Update::Updated {
                    retired_circuits: 1,
                    ..
                }
            ),
            "{update:?}"
        );
    }
    assert!(f.a.circuit_info(id).unwrap().is_none());
    let b = f.ids()[2];
    assert!(matches!(f.a.start_diagnostic(b, None), Err(Error::Policy)));
    let snapshot = f.nodes[0].identity().snapshot().clone();
    assert!(matches!(
        diagnostic_paths(&snapshot, b, None),
        Err(Error::Policy)
    ));
    f.close().await;
}

#[tokio::test]
async fn netmap_ping_without_observable_path_is_unsupported_not_denied() {
    let mut f = NetmapMesh::start().await;
    let r = f.ids()[1];
    f.inventory
        .get_mut(&r)
        .unwrap()
        .capabilities
        .remove("feature:path-observation");
    f.publish(|_, _| {}).await;
    let b = f.ids()[2];
    assert!(matches!(
        f.a.start_diagnostic(b, None),
        Err(Error::Unsupported)
    ));
    let snapshot = f.nodes[0].identity().snapshot().clone();
    assert!(matches!(
        diagnostic_paths(&snapshot, b, None),
        Err(Error::Unsupported)
    ));
    // An explicit path outside the decision is denied, not unsupported.
    let [a, _, b] = f.ids();
    assert!(matches!(
        diagnostic_paths(&snapshot, b, Some(&[a, b])),
        Err(Error::Policy)
    ));
    f.close().await;
}
