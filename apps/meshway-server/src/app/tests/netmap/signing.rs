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

/// The endorsement check a Node performs with the key it pinned.
fn endorses(
    pinned: &ServerSigningKey,
    network: NetworkId,
    rotation: &meshway_protocol::NetmapKeyRotation,
) -> bool {
    let message = meshway_protocol::netmap_key_rotation_transcript(
        network,
        pinned.key_id,
        pinned.key_generation,
        &rotation.next,
        rotation.not_before_ms,
    );
    let mut bytes = [0; 64];
    URL_SAFE_NO_PAD
        .decode_slice(&rotation.endorsement, &mut bytes)
        .ok()
        == Some(64)
        && ed25519_dalek::VerifyingKey::from_bytes(pinned.public_key.bytes.as_bytes()).is_ok_and(
            |key| {
                key.verify_strict(&message, &ed25519_dalek::Signature::from_bytes(&bytes))
                    .is_ok()
            },
        )
}

fn with_signature(
    response: &SyncResponse,
    signature: &meshway_protocol::NetmapSignature,
) -> SyncResponse {
    SyncResponse {
        signature: signature.clone(),
        ..response.clone()
    }
}

/// Simulates the end of the open window by moving the stored `retired_at`.
async fn end_window(app: &App) {
    app.executor
        .handle()
        .write(|tx| {
            let now = crate::store::execution::now()?;
            tx.execute(
                "UPDATE server_signing_keys SET retired_at=max(created_at,?1-1) WHERE retired_at>?1",
                [now],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

fn pinned(generation: Option<Version>, netmap: Option<NetmapRef>) -> SyncRequest {
    SyncRequest {
        server_key_generation: generation,
        ..holding(netmap)
    }
}

#[tokio::test]
async fn a_rotation_window_endorses_the_successor_and_keeps_signing_with_the_old_key() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, mut app, network, a, b, ak, _, signer_a, _) = fixture().await;
    app.netmap_timing.hold = Duration::from_millis(100);
    let old = stored_key(&app).await;
    let token = session(&app.execution(), network, a, &ak, &signer_a).await;
    let first = app
        .netmaps()
        .sync(token.token.expose(), holding(None))
        .await
        .unwrap();
    assert_eq!(first.key_rotation, None);

    let rotated = app
        .rotate_netmap_key(Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(rotated.previous, old);
    assert_eq!(rotated.next.key_generation, Version::new(2).unwrap());
    assert_ne!(rotated.next.key_id, old.key_id);
    let new = stored_key(&app).await;
    assert_eq!(new, rotated.next, "enrollment now hands out the successor");
    // One window at a time.
    assert_eq!(
        app.rotate_netmap_key(Duration::from_secs(60)).await,
        Err(crate::error::Error::StateConflict)
    );
    assert_eq!(
        app.rotate_netmap_key(Duration::from_secs(59)).await,
        Err(crate::error::Error::InvalidRequest)
    );

    let SyncNetmap::Full { netmap } = &first.netmap else {
        panic!("expected full");
    };
    let during = app
        .netmaps()
        .sync(
            token.token.expose(),
            pinned(Some(old.key_generation), Some(netmap.reference())),
        )
        .await
        .unwrap();
    assert!(
        verifies(&old, network, a, &during),
        "still signed by the old key"
    );
    assert!(!verifies(&new, network, a, &during));
    let rotation = during
        .key_rotation
        .clone()
        .expect("handover during the window");
    assert_eq!(rotation.next, new);
    assert!(endorses(&old, network, &rotation));
    assert!(
        !endorses(&new, network, &rotation),
        "only the replaced key endorses"
    );
    let other: NetworkId = b.into_uuid().try_into().unwrap();
    assert!(!endorses(&old, other, &rotation), "bound to the network");
    let mut later = rotation.clone();
    later.not_before_ms += 1;
    assert!(!endorses(&old, network, &later), "bound to notBefore");
    assert!(
        verifies(
            &new,
            network,
            a,
            &with_signature(&during, &rotation.lease_signature)
        ),
        "the successor signs the same lease"
    );
    assert!(rotation.not_before_ms <= during.lease.issued_at_ms);

    // The inventory says which key each Node reported pinned.
    let owner = owner(&app).await;
    let status = |app: &App| {
        let execution = app.execution();
        let owner = owner.clone();
        async move {
            execution
                .inventory(&owner, network)
                .await
                .unwrap()
                .items
                .into_iter()
                .find(|item| item.id == a)
                .unwrap()
                .server_key
        }
    };
    use meshway_protocol::{PinnedServerKeyStatus as S, PinnedServerKeyView as V};
    assert_eq!(
        status(&app).await,
        Some(V {
            key_generation: old.key_generation,
            status: S::Rotating
        })
    );
    app.netmaps()
        .sync(
            token.token.expose(),
            pinned(Some(new.key_generation), Some(netmap.reference())),
        )
        .await
        .unwrap();
    assert_eq!(
        status(&app).await,
        Some(V {
            key_generation: new.key_generation,
            status: S::Current
        })
    );

    // After the window the successor signs and no handover is offered.
    end_window(&app).await;
    let after = app
        .netmaps()
        .sync(
            token.token.expose(),
            pinned(Some(old.key_generation), Some(netmap.reference())),
        )
        .await
        .unwrap();
    assert_eq!(after.key_rotation, None);
    assert!(verifies(&new, network, a, &after));
    assert!(!verifies(&old, network, a, &after));
    assert_eq!(
        status(&app).await,
        Some(V {
            key_generation: old.key_generation,
            status: S::KeyMismatch
        })
    );
    // The replaced key stays stored for audit; a new rotation may start.
    let third = app
        .rotate_netmap_key(Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(third.previous, new);
    assert_eq!(third.next.key_generation, Version::new(3).unwrap());
    let rows: i64 = app
        .executor
        .handle()
        .read(|tx| Ok(tx.query_row("SELECT count(*) FROM server_signing_keys", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(rows, 3);
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_open_rotation_window_survives_a_restart() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (dir, app, network, a, _, ak, _, signer_a, _) = fixture().await;
    let old = stored_key(&app).await;
    let rotated = app
        .rotate_netmap_key(Duration::from_secs(3600))
        .await
        .unwrap();
    app.shutdown().await.unwrap();
    let app = App::open(Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    assert_eq!(stored_key(&app).await, rotated.next);
    assert_eq!(
        app.rotate_netmap_key(Duration::from_secs(60)).await,
        Err(crate::error::Error::StateConflict),
        "the window is stored, not process state"
    );
    let token = session(&app.execution(), network, a, &ak, &signer_a).await;
    let response = app
        .netmaps()
        .sync(token.token.expose(), holding(None))
        .await
        .unwrap();
    assert!(verifies(&old, network, a, &response));
    let rotation = response.key_rotation.expect("window still open");
    assert_eq!(rotation.next, rotated.next);
    assert!(endorses(&old, network, &rotation));
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}
