use super::*;

#[tokio::test]
async fn enrollment_and_commits_store_serials_in_their_own_transaction() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ..) = fixture().await;
    let owner = owner(&app).await;
    let (a0, b0) = (
        current(&app, network, a).await.unwrap(),
        current(&app, network, b).await.unwrap(),
    );
    let seen_by_a = load(&app, network, a).await;
    assert!(
        seen_by_a
            .netmap
            .projection
            .configuration
            .nodes
            .contains_key(&b)
    );
    assert_eq!(a0.current.revision, seen_by_a.netmap.revision);

    let revision = edit(&app, &owner, network, vec![rename(b, "renamed")])
        .await
        .unwrap();
    let (a1, b1) = (
        current(&app, network, a).await.unwrap(),
        current(&app, network, b).await.unwrap(),
    );
    assert_eq!(b1.current.serial.get(), b0.current.serial.get() + 1);
    assert_eq!(b1.current.revision, revision);
    assert_eq!(b1.previous, Some(b0.current));
    // a sees b's declaration, so its projection changed too.
    assert_eq!(a1.current.serial.get(), a0.current.serial.get() + 1);
    assert_eq!(
        load(&app, network, a)
            .await
            .netmap
            .projection
            .configuration
            .nodes[&b]
            .hostname,
        "renamed"
    );

    // A failed netmap write rolls back the whole version.
    app.executor
        .handle()
        .execute(|db| {
            db.execute_batch(
                "CREATE TRIGGER fail_netmap BEFORE INSERT ON node_netmaps BEGIN SELECT RAISE(ABORT,'injected'); END;",
            )
        })
        .await
        .unwrap()
        .unwrap();
    let stored = rows(&app, network).await;
    assert!(
        edit(&app, &owner, network, vec![rename(b, "again")])
            .await
            .is_err()
    );
    assert_eq!(super::revision(&app, &owner, network).await, revision);
    assert_eq!(rows(&app, network).await, stored);
    app.executor
        .handle()
        .execute(|db| db.execute_batch("DROP TRIGGER fail_netmap;"))
        .await
        .unwrap()
        .unwrap();
    // Only the current serial and its delta base are kept.
    edit(&app, &owner, network, vec![rename(b, "third")])
        .await
        .unwrap();
    let kept: Vec<i64> = rows(&app, network)
        .await
        .into_iter()
        .filter(|row| row.0 == b.as_uuid().as_bytes())
        .map(|row| row.1)
        .collect();
    assert_eq!(
        kept,
        [
            b1.current.serial.get() as i64,
            b1.current.serial.get() as i64 + 1
        ]
    );
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn unrelated_commits_and_instance_lifecycle_never_write_a_serial() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, _, ak, _, signer_a, _) = fixture().await;
    let owner = owner(&app).await;
    let before = rows(&app, network).await;
    comment_only_edit(&app, &owner, network).await;
    assert_eq!(rows(&app, network).await, before);

    let service = app.execution();
    let token = session(&service, network, a, &ak, &signer_a).await;
    // The first acquire authenticates capabilities: an inventory change.
    let lease = acquire(&app, &token).await;
    let after_capabilities = rows(&app, network).await;
    assert_ne!(after_capabilities, before);
    let instance = InstanceRequest {
        instance_id: lease.instance_id,
        generation: lease.generation,
    };
    service
        .renew(token.token.expose(), instance.clone())
        .await
        .unwrap();
    service
        .release(token.token.expose(), instance)
        .await
        .unwrap();
    // A replacement process with the same capabilities is not an input change.
    acquire(&app, &token).await;
    assert_eq!(rows(&app, network).await, after_capabilities);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn revocations_recompile_every_referencing_node() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, _, bk, ..) = fixture().await;
    let owner = owner(&app).await;

    let before = serial(current(&app, network, a).await);
    edit(
        &app,
        &owner,
        network,
        vec![ConfigurationOperation::ReplaceAccess { access: vec![] }],
    )
    .await
    .unwrap();
    assert_eq!(serial(current(&app, network, a).await), before + 1);
    let netmap = load(&app, network, a).await;
    assert!(netmap.netmap.projection.configuration.access.is_empty());

    let before = serial(current(&app, network, a).await);
    app.execution()
        .revoke_key(
            &owner,
            network,
            b,
            meshway_protocol::node::RevokeNodeKeyRequest {
                key_id: bk.dataplane.id,
                expected_generation: bk.dataplane.generation,
            },
        )
        .await
        .unwrap();
    // b has no trusted keys: withdrawn, its serial high-water kept.
    assert_eq!(current(&app, network, b).await, None);
    assert!(
        rows(&app, network)
            .await
            .iter()
            .any(|row| row.0 == b.as_uuid().as_bytes() && row.3)
    );
    assert_eq!(serial(current(&app, network, a).await), before + 1);
    assert!(
        !load(&app, network, a)
            .await
            .netmap
            .projection
            .inventory
            .contains_key(&b)
    );
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn member_removal_withdraws_user_nodes_and_recompiles_their_peers() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, tag, ..) = fixture().await;
    let owner = owner(&app).await;
    let member = app
        .accounts()
        .register(
            "member@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let link = app
        .networks()
        .create_join_link(&owner, network, MembershipRole::Member, 600, rid())
        .await
        .unwrap();
    app.networks()
        .redeem_join_link(member.token.expose(), link.secret.unwrap().expose(), rid())
        .await
        .unwrap();
    let signer = ed25519_dalek::SigningKey::from_bytes(&[42; 32]);
    let user = super::super::node::enroll_user_node(
        &app,
        member.token.expose(),
        network,
        &signer,
        [52; 32],
    )
    .await
    .node_id;
    assert!(current(&app, network, user).await.is_some());
    let before = serial(current(&app, network, tag).await);
    assert!(
        load(&app, network, tag)
            .await
            .netmap
            .projection
            .inventory
            .contains_key(&user)
    );
    let impact = app
        .networks()
        .removal_impact(member.token.expose(), network, None, rid())
        .await
        .unwrap();
    app.networks()
        .remove_member(
            member.token.expose(),
            network,
            None,
            impact.version,
            impact.revision,
            rid(),
        )
        .await
        .unwrap();
    assert_eq!(current(&app, network, user).await, None);
    assert_eq!(serial(current(&app, network, tag).await), before + 1);
    assert!(
        !load(&app, network, tag)
            .await
            .netmap
            .projection
            .inventory
            .contains_key(&user)
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_netmap_above_the_safety_bound_rejects_the_version() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, ..) = fixture().await;
    let owner = owner(&app).await;
    let before = (
        super::revision(&app, &owner, network).await,
        rows(&app, network).await,
    );
    // Tag definitions are projected to every Node in full.
    let tags_owners = (0..90)
        .map(|index| (format!("tag:{index}-{}", "x".repeat(100_000)), vec![]))
        .chain([("tag:test".to_owned(), vec![])])
        .collect::<BTreeMap<_, _>>();
    let error = edit(
        &app,
        &owner,
        network,
        vec![ConfigurationOperation::ReplaceTagsOwners { tags_owners }],
    )
    .await
    .unwrap_err();
    let crate::error::Error::InvalidConfiguration(ConfigurationError::Invalid(issues)) = error
    else {
        panic!("{error:?}");
    };
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, IssueCode::NetmapLimit);
    assert_eq!(
        issues[0].params["maxBytes"],
        meshway_model::netmap::MAX_NETMAP_BYTES.to_string()
    );
    assert_eq!(
        (
            super::revision(&app, &owner, network).await,
            rows(&app, network).await
        ),
        before
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_node_triggered_netmap_above_the_bound_is_unavailable_not_a_configuration_error() {
    use meshway_model::netmap::MAX_NETMAP_BYTES;
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let owner = owner(&app).await;
    let size = |netmap: CompiledNetmap| netmap.netmap.encoded_len().unwrap();
    let base = size(load(&app, network, a).await).max(size(load(&app, network, b).await));
    let capabilities: BTreeMap<String, meshway_model::state::Capability> = (0..32)
        .map(|index| {
            (
                format!("feature:{index:02}{}", "y".repeat(52)),
                meshway_model::state::Capability::Feature { version: 1 },
            )
        })
        .collect();
    // a's netmap carries its capabilities twice (self keys and inventory), b's
    // once. One more tag adds its name plus 6 bytes to every projection: fill
    // both so that only a crosses the bound.
    let margin = serde_json::to_vec(&capabilities).unwrap().len() * 3 / 2;
    let name = format!(
        "tag:{}",
        "x".repeat(MAX_NETMAP_BYTES - base - margin - 6 - 4)
    );
    edit(
        &app,
        &owner,
        network,
        vec![ConfigurationOperation::ReplaceTagsOwners {
            tags_owners: BTreeMap::from([("tag:test".into(), vec![]), (name, vec![])]),
        }],
    )
    .await
    .unwrap();
    assert!(size(load(&app, network, a).await) <= MAX_NETMAP_BYTES);

    // a authenticates 32 long capabilities: its own netmap carries them twice
    // (self keys and inventory) and crosses the bound. Acquire still succeeds.
    let token = session(&app.execution(), network, a, &ak, &signer_a).await;
    app.execution()
        .acquire(
            token.token.expose(),
            AcquireInstanceRequest {
                instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                capabilities,
            },
        )
        .await
        .unwrap();
    // Only a is excluded; b is compiled without a, which it no longer trusts.
    assert_eq!(current(&app, network, a).await, None);
    assert!(
        !load(&app, network, b)
            .await
            .netmap
            .projection
            .inventory
            .contains_key(&a)
    );
    let error = app
        .netmaps()
        .sync(token.token.expose(), holding(None))
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            crate::error::Error::Unavailable(crate::error::Unavailability::Netmap)
        ),
        "{error:?}"
    );
    let b_token = session(&app.execution(), network, b, &bk, &signer_b).await;
    assert!(
        app.netmaps()
            .sync(b_token.token.expose(), holding(None))
            .await
            .is_ok()
    );

    // A version that fits again restores both netmaps.
    edit(
        &app,
        &owner,
        network,
        vec![ConfigurationOperation::ReplaceTagsOwners {
            tags_owners: BTreeMap::from([("tag:test".into(), vec![])]),
        }],
    )
    .await
    .unwrap();
    assert!(current(&app, network, a).await.is_some());
    assert!(
        app.netmaps()
            .sync(token.token.expose(), holding(None))
            .await
            .is_ok()
    );
    app.shutdown().await.unwrap();
}
