use super::*;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use meshway_protocol::netmap::{NetmapSigner, ServerSigningKey, netmap_lease_transcript};

/// The verification a Node performs with the key pinned at enrollment.
fn verifies(
    key: &ServerSigningKey,
    network: NetworkId,
    node: NodeId,
    response: &SyncResponse,
) -> bool {
    let signature = &response.signature;
    let mut bytes = [0; 64];
    if signature.key_id != key.key_id
        || signature.key_generation != key.key_generation
        || URL_SAFE_NO_PAD
            .decode_slice(&signature.signature, &mut bytes)
            .ok()
            != Some(64)
    {
        return false;
    }
    let message = netmap_lease_transcript(
        network,
        node,
        &response.lease,
        NetmapSigner::Server,
        signature.key_id,
        signature.key_generation,
    );
    ed25519_dalek::VerifyingKey::from_bytes(key.public_key.bytes.as_bytes()).is_ok_and(|key| {
        key.verify_strict(&message, &ed25519_dalek::Signature::from_bytes(&bytes))
            .is_ok()
    })
}

async fn stored_key(app: &App) -> ServerSigningKey {
    app.executor
        .handle()
        .read(crate::store::netmap::server_signing_key)
        .await
        .unwrap()
}

#[tokio::test]
async fn every_sync_answer_carries_the_enrolled_servers_signature() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, mut app, network, a, b, ak, _, signer_a, _) = fixture().await;
    app.netmap_timing.hold = Duration::from_millis(100);
    let owner = owner(&app).await;
    let c = super::super::node::enroll_tag_node(
        &app,
        &owner,
        network,
        &ed25519_dalek::SigningKey::from_bytes(&[9; 32]),
        "c",
        [19; 32],
    )
    .await;
    let key = c.server_signing_key.clone();
    assert_eq!(key, stored_key(&app).await);
    assert_eq!(key.public_key.algorithm, KeyAlgorithm::Ed25519);
    let token = session(&app.execution(), network, a, &ak, &signer_a).await;
    let service = app.netmaps();

    let full = service
        .sync(token.token.expose(), holding(None))
        .await
        .unwrap();
    let SyncNetmap::Full { netmap: base } = &full.netmap else {
        panic!("expected full");
    };
    assert_eq!(full.signature.key_id, key.key_id);
    assert!(verifies(&key, network, a, &full));
    let unchanged = service
        .sync(token.token.expose(), holding(Some(base.reference())))
        .await
        .unwrap();
    assert_eq!(unchanged.netmap, SyncNetmap::Unchanged);
    assert!(verifies(&key, network, a, &unchanged));
    assert_ne!(unchanged.signature, full.signature);
    edit(&app, &owner, network, vec![rename(b, "signed")])
        .await
        .unwrap();
    let delta = service
        .sync(token.token.expose(), holding(Some(base.reference())))
        .await
        .unwrap();
    assert!(matches!(delta.netmap, SyncNetmap::Delta { .. }));
    assert!(verifies(&key, network, a, &delta));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn tampering_with_any_signed_field_breaks_the_signature() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, _, signer_a, _) = fixture().await;
    let key = stored_key(&app).await;
    let token = session(&app.execution(), network, a, &ak, &signer_a).await;
    let response = app
        .netmaps()
        .sync(token.token.expose(), holding(None))
        .await
        .unwrap();
    assert!(verifies(&key, network, a, &response));
    // Another network or Node does not accept the answer.
    assert!(!verifies(&key, network, b, &response));
    assert!(!verifies(
        &key,
        b.into_uuid().try_into().unwrap(),
        a,
        &response
    ));
    let tampered: [fn(&mut SyncResponse); 9] = [
        |r| r.lease.issued_at_ms -= 1,
        |r| r.lease.expires_at_ms += 1,
        |r| r.lease.netmap.serial = r.lease.netmap.serial.checked_next().unwrap(),
        |r| r.lease.netmap.revision = r.lease.netmap.revision.checked_next().unwrap(),
        |r| r.lease.netmap.digest = Digest::from_bytes([3; 32]),
        |r| r.signature.key_id = "00000000-0000-4000-8000-0000000000ee".parse().unwrap(),
        |r| r.signature.key_generation = r.signature.key_generation.checked_next().unwrap(),
        |r| {
            let mut bytes = URL_SAFE_NO_PAD.decode(&r.signature.signature).unwrap();
            bytes[0] ^= 1;
            r.signature.signature = URL_SAFE_NO_PAD.encode(bytes);
        },
        |r| r.signature.signature = "A".repeat(86),
    ];
    for (index, tamper) in tampered.into_iter().enumerate() {
        let mut copy = response.clone();
        tamper(&mut copy);
        assert!(!verifies(&key, network, a, &copy), "field {index}");
    }
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_signing_key_survives_a_restart() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (dir, app, network, a, _, ak, _, signer_a, _) = fixture().await;
    let before = stored_key(&app).await;
    app.shutdown().await.unwrap();
    let app = App::open(Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    assert_eq!(stored_key(&app).await, before);
    let token = session(&app.execution(), network, a, &ak, &signer_a).await;
    let response = app
        .netmaps()
        .sync(token.token.expose(), holding(None))
        .await
        .unwrap();
    assert!(verifies(&before, network, a, &response));
    let rows: i64 = app
        .executor
        .handle()
        .read(|tx| Ok(tx.query_row("SELECT count(*) FROM server_signing_keys", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(rows, 1);
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}
