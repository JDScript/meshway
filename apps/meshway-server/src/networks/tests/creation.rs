use super::*;

#[tokio::test]
async fn real_creation_isolation_idempotency_sealed_complete_configuration_and_reopen() {
    let _serial = TEST_KDF_LOCK.lock().await;
    let (_temp, path, app) = fixture().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let networks = app.networks();
    assert!(
        networks
            .list(alice.token.expose(), 20, None)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let req = request();
    let first = networks
        .create(alice.token.expose(), "  Personal  ".into(), req)
        .await
        .unwrap();
    assert_eq!(first.name, "Personal");
    assert_eq!(first.role, MembershipRole::Owner);
    assert!(
        first.actions.rename
            && first.actions.read_configuration
            && first.actions.edit_configuration
            && first.actions.manage_members
    );
    assert_eq!(first.current_revision, Revision::MIN);
    assert_eq!(
        networks
            .create(alice.token.expose(), "Personal".into(), req)
            .await
            .unwrap(),
        first
    );
    assert_eq!(
        networks
            .create(alice.token.expose(), "different".into(), req)
            .await,
        Err(Error::RequestConflict)
    );
    let second = networks
        .create(bob.token.expose(), "Personal".into(), req)
        .await
        .unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(
        networks.get(bob.token.expose(), first.id).await,
        Err(Error::NotFound)
    );
    assert_eq!(
        networks.get(alice.token.expose(), second.id).await,
        Err(Error::NotFound)
    );
    assert_eq!(
        networks
            .list(alice.token.expose(), 20, None)
            .await
            .unwrap()
            .items,
        vec![first.clone()]
    );
    let members = networks
        .members(alice.token.expose(), first.id, 20, None)
        .await
        .unwrap();
    assert_eq!(members.items.len(), 1);
    assert_eq!(members.items[0].user_id, alice.session.user.id);
    assert_eq!(members.items[0].role, MembershipRole::Owner);
    let (source, config, bindings, revision, publications) =
        config_bytes(&networks, first.id).await;
    assert_eq!((revision, publications), (1, 0));
    let id_bytes = first.id.as_uuid().as_bytes();
    let ula = format!(
        "fd{:02x}:{:02x}{:02x}:{:02x}{:02x}::",
        id_bytes[0], id_bytes[1], id_bytes[2], id_bytes[3], id_bytes[4]
    )
    .parse::<std::net::Ipv6Addr>()
    .unwrap();
    let expected = new_network(
        AddressPools {
            ipv4: vec!["100.64.0.0/10".into()],
            ipv6: vec![format!("{ula}/48")],
        },
        format!("{}.meshway.internal", first.id.as_uuid().simple()),
    )
    .unwrap();
    let canonical = canonical_json(&expected).unwrap();
    for (bytes, purpose) in [
        (&source, Purpose::Source),
        (&config, Purpose::Configuration),
        (&bindings, Purpose::Bindings),
    ] {
        assert!(
            !bytes
                .windows(canonical.expose().len())
                .any(|w| w == canonical.expose().as_bytes())
        );
        assert!(
            networks
                .sealer
                .open(second.id, Revision::MIN, purpose, bytes)
                .is_err()
        );
        assert!(
            networks
                .sealer
                .open(first.id, Revision::new(2).unwrap(), purpose, bytes)
                .is_err()
        );
    }
    assert!(
        networks
            .sealer
            .open(first.id, Revision::MIN, Purpose::Configuration, &source)
            .is_err()
    );
    let plain = networks
        .sealer
        .open(first.id, Revision::MIN, Purpose::Configuration, &config)
        .unwrap();
    assert!(plain.as_bytes() == canonical.expose().as_bytes());
    let source = networks
        .sealer
        .open(first.id, Revision::MIN, Purpose::Source, &source)
        .unwrap();
    assert!(source.as_bytes() == canonical.expose().as_bytes());
    let bindings = networks
        .sealer
        .open(first.id, Revision::MIN, Purpose::Bindings, &bindings)
        .unwrap();
    let typed: crate::configuration::ConfigurationBindings =
        serde_json::from_slice(bindings.as_bytes()).unwrap();
    assert!(typed.users.is_empty() && typed.listeners.is_empty() && typed.rule_sets.is_empty());
    assert!(expected.nodes.is_empty());
    for filename in ["controller.sqlite", "controller.sqlite-wal"] {
        if let Ok(bytes) = std::fs::read(path.join(filename)) {
            assert!(
                !bytes
                    .windows(canonical.expose().len())
                    .any(|w| w == canonical.expose().as_bytes())
            );
        }
    }
    let counts = networks.handle.execute(|c| c.query_row("SELECT (SELECT count(*) FROM networks), (SELECT count(*) FROM memberships), (SELECT count(*) FROM configuration_versions), (SELECT count(*) FROM activity_events WHERE action = 'network.create')", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))).unwrap()).await.unwrap();
    assert_eq!(counts, (2, 2, 2, 2));
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
    assert!(matches!(
        networks.get(alice.token.expose(), first.id).await,
        Err(Error::Unavailable(_))
    ));
    let app = App::open(Settings::new(&path)).await.unwrap();
    assert_eq!(
        app.networks()
            .get(alice.token.expose(), first.id)
            .await
            .unwrap(),
        first
    );
    assert_eq!(
        app.networks()
            .create(alice.token.expose(), "Personal".into(), req)
            .await
            .unwrap(),
        first
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn final_create_transaction_rechecks_session_after_preparation_and_cancellation_retries_once()
{
    let _serial = TEST_KDF_LOCK.lock().await;
    let (_temp, _path, app) = fixture().await;
    let alice = user(&app, "alice").await;
    let networks = app.networks();
    for mutation in [
        "UPDATE user_sessions SET revoked_at = created_at",
        "UPDATE users SET disabled_at = created_at",
        "UPDATE users SET credential_version = credential_version + 1",
    ] {
        let (started, running) = oneshot::channel();
        let (release, wait) = mpsc::channel();
        let mut gate = Box::pin(networks.handle.execute(move |_| {
            started.send(()).unwrap();
            wait.recv().unwrap();
        }));
        pending(gate.as_mut());
        running.await.unwrap();
        let mut creating =
            Box::pin(networks.create(alice.token.expose(), "must fail".into(), request()));
        pending(creating.as_mut()); // preflight queued before revocation
        let mut revoke = Box::pin(
            networks
                .handle
                .execute(move |c| c.execute_batch(mutation).unwrap()),
        );
        pending(revoke.as_mut());
        release.send(()).unwrap();
        gate.await.unwrap();
        revoke.await.unwrap();
        assert_eq!(creating.await, Err(Error::InvalidSession));
        assert_eq!(
            networks.list(alice.token.expose(), 1, None).await,
            Err(Error::InvalidSession)
        );
        networks.handle.execute(|c| c.execute_batch("UPDATE user_sessions SET revoked_at = NULL; UPDATE users SET disabled_at = NULL, credential_version = 1").unwrap()).await.unwrap();
    }
    // Drive the public API through preflight, then cancel after the actual
    // creating transaction is admitted behind a deterministic worker barrier.
    let req = request();
    let (started, running) = oneshot::channel();
    let (release, wait) = mpsc::channel();
    let mut gate = Box::pin(networks.handle.execute(move |_| {
        started.send(()).unwrap();
        wait.recv().unwrap();
    }));
    pending(gate.as_mut());
    running.await.unwrap();
    let mut creating = Box::pin(networks.create(alice.token.expose(), "recover".into(), req));
    pending(creating.as_mut());
    let (preflight_done, preflight) = oneshot::channel();
    let (finish, hold) = mpsc::channel();
    let mut barrier = Box::pin(networks.handle.execute(move |_| {
        preflight_done.send(()).unwrap();
        hold.recv().unwrap();
    }));
    pending(barrier.as_mut());
    release.send(()).unwrap();
    gate.await.unwrap();
    preflight.await.unwrap();
    pending(creating.as_mut());
    drop(creating);
    finish.send(()).unwrap();
    barrier.await.unwrap();
    let recovered = networks
        .create(alice.token.expose(), "recover".into(), req)
        .await
        .unwrap();
    assert_eq!(
        networks
            .list(alice.token.expose(), 20, None)
            .await
            .unwrap()
            .items,
        vec![recovered]
    );
    networks
        .handle
        .execute(|c| {
            assert_eq!(
                c.query_row(
                    "SELECT count(*) FROM activity_events WHERE action = 'network.create'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            )
        })
        .await
        .unwrap();
    app.shutdown().await.unwrap();
}

#[test]
fn ula_templates_are_canonical_even_with_zero_global_id_segments() {
    let id = NetworkId::new(uuid::Uuid::from_bytes([
        1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
    ]))
    .unwrap();
    assert_eq!(
        configuration::initial(id).unwrap().address_pools.ipv6,
        ["fd01:1::/48"]
    );
}

#[tokio::test]
async fn schema_enforces_network_revision_foreign_keys_and_immutable_complete_versions() {
    let _serial = TEST_KDF_LOCK.lock().await;
    let (_temp, _path, app) = fixture().await;
    let alice = user(&app, "alice").await;
    let networks = app.networks();
    let first = networks
        .create(alice.token.expose(), "first".into(), request())
        .await
        .unwrap();
    let second = networks
        .create(alice.token.expose(), "second".into(), request())
        .await
        .unwrap();
    let (source, config, bindings, _, _) = config_bytes(&networks, second.id).await;
    let reseal = |bytes: &[u8], purpose| {
        let plain = networks
            .sealer
            .open(second.id, Revision::MIN, purpose, bytes)
            .unwrap();
        networks
            .sealer
            .seal(
                second.id,
                Revision::new(2).unwrap(),
                purpose,
                plain.as_bytes(),
            )
            .unwrap()
    };
    let source = reseal(&source, Purpose::Source);
    let config = reseal(&config, Purpose::Configuration);
    let bindings = reseal(&bindings, Purpose::Bindings);
    networks.handle.execute(move |c| {
        for sql in [
            "UPDATE networks SET id = zeroblob(16)",
            "UPDATE networks SET created_by = randomblob(16)",
            "UPDATE networks SET creation_request_id = zeroblob(16)",
            "UPDATE networks SET creation_request_digest = zeroblob(31)",
            "UPDATE networks SET current_revision = 0",
            "UPDATE networks SET publication_sequence = -1",
            "UPDATE networks SET publication_sequence = 9007199254740992",
            "UPDATE networks SET created_at = -1",
            "UPDATE networks SET deleted_at = created_at - 1",
            "UPDATE memberships SET id = zeroblob(16)",
            "UPDATE memberships SET network_id = randomblob(16)",
            "UPDATE memberships SET user_id = randomblob(16)",
            "UPDATE memberships SET role = 'superuser'",
            "UPDATE memberships SET version = 0",
            "UPDATE memberships SET joined_at = -1",
            "UPDATE memberships SET revoked_at = joined_at - 1",
            "UPDATE configuration_versions SET source_jsonc = zeroblob(61)",
            "DELETE FROM configuration_versions",
            "UPDATE activity_events SET account_user_id = actor_user_id WHERE scope = 'network'",
            "UPDATE activity_events SET network_id = NULL WHERE scope = 'network'",
        ] { assert!(c.execute(sql, []).is_err(), "accepted {sql}"); }
        assert!(c.execute("INSERT INTO memberships (id, network_id, user_id, role, version, joined_at) SELECT randomblob(16), network_id, user_id, role, version, joined_at FROM memberships LIMIT 1", []).is_err());
        // A revision existing in another network cannot satisfy this pointer.
        c.execute("INSERT INTO configuration_versions SELECT network_id, 2, ?2, ?3, ?4, created_by, created_at, cause, request_id, NULL, '{}', created_by_email FROM configuration_versions WHERE network_id = ?1", params![second.id.as_uuid().as_bytes().as_slice(), source.as_bytes(), config.as_bytes(), bindings.as_bytes()]).unwrap();
        let tx = c.transaction().unwrap();
        tx.execute("UPDATE networks SET current_revision = 2 WHERE id = ?1", [first.id.as_uuid().as_bytes().as_slice()]).unwrap();
        assert!(tx.commit().is_err());
        assert_eq!(c.query_row("SELECT current_revision FROM networks WHERE id = ?1", [first.id.as_uuid().as_bytes().as_slice()], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert!(c.execute("INSERT INTO configuration_versions SELECT network_id, 3, x'01', configuration, bindings, created_by, created_at, cause, request_id, NULL, '{}', created_by_email FROM configuration_versions LIMIT 1", []).is_err());
    }).await.unwrap();
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}
