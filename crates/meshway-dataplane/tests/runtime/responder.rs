//! Authenticated malformed HS1 consumes a responder CID, not a business flow.
use super::support::secure::SecureMesh;
use meshway_control::DataPlaneIdentity;
use meshway_dataplane::secure::{CircuitState, Error, Limits, Received, SecureNode};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

// Adversarial candidate-v4 wire fixture, not another codec or admission path.
// Change the Noise tag in a real one-proof Init and have its real origin sign
// that changed transcript. Merely corrupting bytes would only test bad proofs.
fn resign(
    mut bytes: Vec<u8>,
    identity: DataPlaneIdentity,
    listener_key: &str,
    mutate: impl FnOnce(&mut [u8], usize),
) -> Vec<u8> {
    assert_eq!(&bytes[..8], b"MWCIR\0\0\x04");
    let signature = bytes.len() - 64;
    let proof = signature - (16 + 8 + 16 + 16 + 1 + 1 + listener_key.len() + 32);
    assert_eq!(bytes[proof - 1], 1);
    mutate(&mut bytes, proof - 1);
    let mut transcript = Sha256::new();
    transcript.update(b"meshway/circuit/hop-proof/candidate-v4\0");
    transcript.update(&bytes[..proof - 1]);
    transcript.update([0u8]);
    transcript.update(&bytes[proof..signature]);
    let signature_bytes = identity
        .sign_packet(meshway_model::Digest::from_bytes(
            transcript.finalize().into(),
        ))
        .unwrap();
    bytes[signature..].copy_from_slice(&signature_bytes);
    bytes
}

#[tokio::test]
async fn authenticated_responder_failure_retains_tombstone_across_replay_renewal_and_capacity() {
    let mut fixture = SecureMesh::start().await;
    fixture.b = SecureNode::new(
        fixture
            .mesh
            .b
            .control
            .dataplane_identity(fixture.mesh.b.control.execution_snapshot().unwrap())
            .unwrap(),
        Limits {
            max_circuits: 1,
            ..Default::default()
        },
        BTreeMap::from([(fixture.mesh.r.id, fixture.rb.carrier.clone())]),
    )
    .unwrap();
    let init = fixture
        .a
        .start(fixture.mesh.demand, fixture.mesh.path.clone())
        .unwrap();
    let cid = init.circuit();
    let bytes = resign(
        init.release(|bytes| bytes).unwrap(),
        fixture
            .mesh
            .a
            .control
            .dataplane_identity(fixture.mesh.a.control.execution_snapshot().unwrap())
            .unwrap(),
        &fixture.ar.carrier.listener_key,
        |bytes, base_end| {
            assert_eq!(bytes[8], 1);
            bytes[base_end - 1] ^= 1;
        },
    );
    let Received::Forward(forward) = fixture.r.receive(&fixture.ar.carrier, bytes).unwrap() else {
        panic!("valid origin proof must reach the opaque relay");
    };
    let bytes = forward.release(|bytes| bytes).unwrap();
    let mut bad_proof = bytes.clone();
    *bad_proof.last_mut().unwrap() ^= 1;
    assert!(matches!(
        fixture.b.receive(&fixture.rb.carrier, bad_proof),
        Err(Error::Proof)
    ));
    assert_eq!(fixture.b.retained_circuits(), 0);
    assert_eq!(fixture.b.retained_flows(), 0);
    assert!(matches!(
        fixture.b.receive(&fixture.rb.carrier, bytes.clone()),
        Err(Error::Crypto)
    ));
    assert_eq!(fixture.b.retained_circuits(), 1);
    assert_eq!(fixture.b.retained_flows(), 0);
    assert_eq!(
        fixture.b.circuit_info(cid).unwrap().unwrap().state,
        CircuitState::Closed
    );
    assert!(matches!(
        fixture.b.receive(&fixture.rb.carrier, bytes.clone()),
        Err(Error::State)
    ));

    let before = fixture.mesh.b.control.execution_snapshot().unwrap();
    let deadline = before.configuration().unwrap().permits[&fixture.mesh.demand].expires_at_ms;
    fixture.renew().await;
    assert!(
        fixture
            .mesh
            .b
            .control
            .execution_snapshot()
            .unwrap()
            .configuration()
            .unwrap()
            .permits[&fixture.mesh.demand]
            .expires_at_ms
            > deadline
    );
    assert!(before.ensure_current().is_err());
    assert!(matches!(
        fixture.b.receive(&fixture.rb.carrier, bytes),
        Err(Error::State)
    ));
    assert_eq!(
        fixture.b.circuit_info(cid).unwrap().unwrap().state,
        CircuitState::Closed
    );
    assert_eq!(fixture.b.retained_circuits(), 1);
    assert_eq!(fixture.b.retained_flows(), 0);

    let init = fixture
        .a
        .start(fixture.mesh.demand, fixture.mesh.path.clone())
        .unwrap();
    let Received::Forward(forward) = fixture
        .r
        .receive(&fixture.ar.carrier, init.release(|bytes| bytes).unwrap())
        .unwrap()
    else {
        panic!("new valid initiation");
    };
    assert!(matches!(
        fixture
            .b
            .receive(&fixture.rb.carrier, forward.release(|bytes| bytes).unwrap()),
        Err(Error::Capacity)
    ));
    fixture.close().await;
}

#[tokio::test]
async fn signed_terminal_cannot_replace_the_relays_cached_ephemeral() {
    let mut fixture = SecureMesh::start().await;
    let init = fixture
        .a
        .start(fixture.mesh.demand, fixture.mesh.path.clone())
        .unwrap();
    let cid = init.circuit();
    let Received::Forward(init) = fixture
        .r
        .receive(&fixture.ar.carrier, init.release(|bytes| bytes).unwrap())
        .unwrap()
    else {
        panic!("relay init");
    };
    let Received::Established {
        response: Some(response),
        ..
    } = fixture
        .b
        .receive(&fixture.rb.carrier, init.release(|bytes| bytes).unwrap())
        .unwrap()
    else {
        panic!("terminal response");
    };
    let original = response.release(|bytes| bytes).unwrap();
    let replaced = resign(
        original.clone(),
        fixture
            .mesh
            .b
            .control
            .dataplane_identity(fixture.mesh.b.control.execution_snapshot().unwrap())
            .unwrap(),
        &fixture.rb.carrier.listener_key,
        |bytes, base_end| {
            assert_eq!(bytes[8], 2);
            // Full three-node public list ends the response base. Replace R's
            // entry and sign with the real terminal key, leaving its own intact.
            bytes[base_end - 64] ^= 1;
        },
    );
    assert!(matches!(
        fixture.r.receive(&fixture.rb.carrier, replaced),
        Err(Error::Proof)
    ));
    assert_eq!(
        fixture.r.circuit_info(cid).unwrap().unwrap().state,
        CircuitState::Handshaking
    );
    assert_eq!(fixture.r.retained_flows(), 0);
    let Received::Forward(response) = fixture.r.receive(&fixture.rb.carrier, original).unwrap()
    else {
        panic!("original response");
    };
    assert!(matches!(
        fixture
            .a
            .receive(
                &fixture.ar.carrier,
                response.release(|bytes| bytes).unwrap()
            )
            .unwrap(),
        Received::Established { response: None, .. }
    ));
    fixture.close().await;
}
