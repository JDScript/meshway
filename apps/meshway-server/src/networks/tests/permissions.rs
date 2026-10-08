use super::*;

#[tokio::test]
async fn current_roles_creator_revocation_rename_cas_and_transactional_activity() {
    let _serial = TEST_KDF_LOCK.lock().await;
    let (_temp, _path, app) = fixture().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let networks = app.networks();
    let req = request();
    let original = networks
        .create(alice.token.expose(), "before".into(), req)
        .await
        .unwrap();
    grant(&networks, original.id, bob.session.user.id, "member").await;
    let member = networks.get(bob.token.expose(), original.id).await.unwrap();
    assert_eq!(
        member.actions,
        NetworkActions {
            rename: false,
            read_configuration: false,
            edit_configuration: false,
            manage_members: false,
            join_link_roles: vec![],
        }
    );
    assert_eq!(
        networks
            .members(bob.token.expose(), original.id, 20, None)
            .await,
        Err(Error::PermissionDenied)
    );
    assert_eq!(
        networks
            .rename(
                bob.token.expose(),
                original.id,
                "after".into(),
                "before".into(),
                request()
            )
            .await,
        Err(Error::PermissionDenied)
    );
    let saved = config_bytes(&networks, original.id).await;
    let renamed = networks
        .rename(
            alice.token.expose(),
            original.id,
            "after".into(),
            "before".into(),
            request(),
        )
        .await
        .unwrap();
    assert_eq!(renamed.name, "after");
    assert_eq!(renamed.current_revision, Revision::MIN);
    assert_eq!(config_bytes(&networks, original.id).await, saved);
    assert_eq!(
        networks
            .rename(
                alice.token.expose(),
                original.id,
                "stale".into(),
                "before".into(),
                request()
            )
            .await,
        Err(Error::NameConflict)
    );
    assert_eq!(
        networks
            .create(alice.token.expose(), "before".into(), req)
            .await
            .unwrap()
            .name,
        "after"
    );
    networks.handle.execute(|c| c.execute_batch("CREATE TRIGGER fail_network_activity BEFORE INSERT ON activity_events WHEN NEW.scope = 'network' BEGIN SELECT RAISE(ABORT, 'test failure'); END").unwrap()).await.unwrap();
    assert!(matches!(
        networks
            .create(alice.token.expose(), "rollback".into(), request())
            .await,
        Err(Error::Storage(_))
    ));
    assert!(matches!(
        networks
            .rename(
                alice.token.expose(),
                original.id,
                "rollback".into(),
                "after".into(),
                request()
            )
            .await,
        Err(Error::Storage(_))
    ));
    networks
        .handle
        .execute(|c| {
            assert_eq!(
                c.query_row("SELECT count(*) FROM networks", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                c.query_row("SELECT count(*) FROM configuration_versions", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                c.query_row("SELECT count(*) FROM memberships", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                2
            );
            assert_eq!(
                c.query_row(
                    "SELECT count(*) FROM activity_events WHERE action = 'network.rename'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
            c.execute_batch("DROP TRIGGER fail_network_activity")
                .unwrap();
        })
        .await
        .unwrap();
    assert_eq!(
        networks
            .get(alice.token.expose(), original.id)
            .await
            .unwrap()
            .name,
        "after"
    );
    let alice_id = alice.session.user.id;
    networks
        .handle
        .execute(move |c| {
            c.execute(
                "UPDATE memberships SET revoked_at = joined_at WHERE user_id = ?1",
                [alice_id.as_uuid().as_bytes().as_slice()],
            )
            .unwrap();
        })
        .await
        .unwrap();
    assert_eq!(
        networks
            .create(alice.token.expose(), "before".into(), req)
            .await,
        Err(Error::NotFound)
    );
    assert_eq!(
        networks.get(alice.token.expose(), original.id).await,
        Err(Error::NotFound)
    );
    assert!(
        networks
            .list(alice.token.expose(), 20, None)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    // Rejoining supplies a new membership; an old creator does not recover Owner.
    grant(&networks, original.id, alice.session.user.id, "member").await;
    let retry = networks
        .create(alice.token.expose(), "before".into(), req)
        .await
        .unwrap();
    assert_eq!(retry.role, MembershipRole::Member);
    assert_ne!(retry.membership_id, original.membership_id);
    assert!(!retry.actions.rename);
    networks
        .handle
        .execute(|c| {
            c.execute("UPDATE networks SET deleted_at = created_at", [])
                .unwrap();
        })
        .await
        .unwrap();
    assert_eq!(
        networks
            .create(alice.token.expose(), "before".into(), req)
            .await,
        Err(Error::NotFound)
    );
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn network_pages_bound_creations_membership_admission_and_opaque_cursor_scope() {
    let _serial = TEST_KDF_LOCK.lock().await;
    let (_temp, path, app) = fixture().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let networks = app.networks();
    let one = networks
        .create(alice.token.expose(), "one".into(), request())
        .await
        .unwrap();
    let hidden = networks
        .create(bob.token.expose(), "hidden".into(), request())
        .await
        .unwrap();
    let two = networks
        .create(alice.token.expose(), "two".into(), request())
        .await
        .unwrap();
    let three = networks
        .create(alice.token.expose(), "three".into(), request())
        .await
        .unwrap();
    let first = networks.list(alice.token.expose(), 1, None).await.unwrap();
    assert_eq!(first.items, vec![one]);
    let cursor = first.next_cursor.clone().unwrap();
    assert!(
        cursor
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    );
    assert_eq!(
        networks
            .list(bob.token.expose(), 1, Some(cursor.clone()))
            .await,
        Err(Error::InvalidCursor)
    );
    let mut tampered = cursor.clone().into_bytes();
    tampered[10] = if tampered[10] == b'A' { b'B' } else { b'A' };
    assert_eq!(
        networks
            .list(
                alice.token.expose(),
                1,
                Some(String::from_utf8(tampered).unwrap())
            )
            .await,
        Err(Error::InvalidCursor)
    );
    for bad in [String::new(), "a".repeat(2049), "not/a+cursor=".into()] {
        assert_eq!(
            networks.list(alice.token.expose(), 1, Some(bad)).await,
            Err(Error::InvalidCursor)
        );
    }
    for limit in [0, 201, u16::MAX] {
        assert_eq!(
            networks.list(alice.token.expose(), limit, None).await,
            Err(Error::InvalidLimit)
        );
    }
    let scope = CursorScope::Networks(alice.session.user.id);
    let expired = pagination::Window::new(100, 100, Utc::now().timestamp_millis() - 16 * 60 * 1000)
        .unwrap()
        .encode(&networks.sealer, scope)
        .unwrap();
    assert_eq!(
        networks.list(alice.token.expose(), 1, Some(expired)).await,
        Err(Error::InvalidCursor)
    );
    let future = pagination::Window::new(100, 100, Utc::now().timestamp_millis() + 60_000)
        .unwrap()
        .encode(&networks.sealer, scope)
        .unwrap();
    assert_eq!(
        networks.list(alice.token.expose(), 1, Some(future)).await,
        Err(Error::InvalidCursor)
    );
    networks
        .create(alice.token.expose(), "later".into(), request())
        .await
        .unwrap();
    grant(&networks, hidden.id, alice.session.user.id, "member").await;
    // Metadata is current, while admission and ordering keep the first boundary.
    networks
        .rename(
            alice.token.expose(),
            two.id,
            "renamed".into(),
            "two".into(),
            request(),
        )
        .await
        .unwrap();
    let second = networks
        .list(alice.token.expose(), 1, Some(cursor.clone()))
        .await
        .unwrap();
    assert_eq!(second.items[0].id, two.id);
    assert_eq!(second.items[0].name, "renamed");
    assert_eq!(second.as_of, first.as_of);
    let last_cursor = second.next_cursor.unwrap();
    // Master key recovery preserves a still-valid cursor across a real restart.
    app.shutdown().await.unwrap();
    let app = App::open(Settings::new(&path)).await.unwrap();
    let third = app
        .networks()
        .list(alice.token.expose(), 200, Some(last_cursor))
        .await
        .unwrap();
    assert_eq!(third.items, vec![three]);
    assert!(third.next_cursor.is_none());
    assert_eq!(third.as_of, first.as_of);
    app.accounts()
        .logout(alice.token.expose(), request())
        .await
        .unwrap();
    assert_eq!(
        app.networks()
            .list(alice.token.expose(), 1, Some(cursor))
            .await,
        Err(Error::InvalidSession)
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn members_pages_are_admin_only_bounded_and_recheck_revocation() {
    let _serial = TEST_KDF_LOCK.lock().await;
    let (_temp, _path, app) = fixture().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let carol = user(&app, "carol").await;
    let networks = app.networks();
    let one = networks
        .create(alice.token.expose(), "one".into(), request())
        .await
        .unwrap();
    let two = networks
        .create(alice.token.expose(), "two".into(), request())
        .await
        .unwrap();
    grant(&networks, one.id, bob.session.user.id, "admin").await;
    let first = networks
        .members(alice.token.expose(), one.id, 1, None)
        .await
        .unwrap();
    assert_eq!(first.items[0].user_id, alice.session.user.id);
    let cursor = first.next_cursor.unwrap();
    assert_eq!(
        networks
            .members(alice.token.expose(), two.id, 1, Some(cursor.clone()))
            .await,
        Err(Error::InvalidCursor)
    );
    assert_eq!(
        networks
            .members(bob.token.expose(), one.id, 1, Some(cursor.clone()))
            .await,
        Err(Error::InvalidCursor)
    );
    assert_eq!(
        networks
            .list(alice.token.expose(), 1, Some(cursor.clone()))
            .await,
        Err(Error::InvalidCursor)
    );
    grant(&networks, one.id, carol.session.user.id, "member").await;
    let second = networks
        .members(alice.token.expose(), one.id, 1, Some(cursor.clone()))
        .await
        .unwrap();
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].user_id, bob.session.user.id);
    assert_eq!(second.items[0].role, MembershipRole::Admin);
    assert!(second.next_cursor.is_none());
    assert_eq!(first.as_of, second.as_of);
    assert_eq!(
        networks
            .members(bob.token.expose(), one.id, 200, None)
            .await
            .unwrap()
            .items
            .len(),
        3
    );
    assert_eq!(
        networks
            .members(carol.token.expose(), one.id, 200, None)
            .await,
        Err(Error::PermissionDenied)
    );
    let before_rejoin = networks
        .members(alice.token.expose(), one.id, 1, None)
        .await
        .unwrap();
    let carol_id = carol.session.user.id;
    networks
        .handle
        .execute(move |c| {
            c.execute(
                "DELETE FROM memberships WHERE user_id = ?1",
                [carol_id.as_uuid().as_bytes().as_slice()],
            )
            .unwrap();
        })
        .await
        .unwrap();
    grant(&networks, one.id, carol_id, "member").await;
    let after_rejoin = networks
        .members(alice.token.expose(), one.id, 200, before_rejoin.next_cursor)
        .await
        .unwrap();
    assert_eq!(
        after_rejoin
            .items
            .iter()
            .map(|member| member.user_id)
            .collect::<Vec<_>>(),
        vec![bob.session.user.id]
    );
    let alice_id = alice.session.user.id;
    networks
        .handle
        .execute(move |c| {
            c.execute(
                "UPDATE memberships SET role = 'member', version = version + 1 WHERE user_id = ?1",
                [alice_id.as_uuid().as_bytes().as_slice()],
            )
            .unwrap();
        })
        .await
        .unwrap();
    assert_eq!(
        networks
            .members(alice.token.expose(), one.id, 1, Some(cursor.clone()))
            .await,
        Err(Error::PermissionDenied)
    );
    networks
        .handle
        .execute(move |c| {
            c.execute(
                "UPDATE memberships SET revoked_at = joined_at WHERE user_id = ?1",
                [alice_id.as_uuid().as_bytes().as_slice()],
            )
            .unwrap();
        })
        .await
        .unwrap();
    assert_eq!(
        networks
            .members(alice.token.expose(), one.id, 1, Some(cursor))
            .await,
        Err(Error::NotFound)
    );
    assert_eq!(
        networks
            .rename(
                alice.token.expose(),
                one.id,
                "no".into(),
                "one".into(),
                request()
            )
            .await,
        Err(Error::NotFound)
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn final_metadata_transaction_rechecks_membership_and_account_and_input_is_bounded() {
    let _serial = TEST_KDF_LOCK.lock().await;
    let (_temp, _path, app) = fixture().await;
    let alice = user(&app, "alice").await;
    let networks = app.networks();
    let network = networks
        .create(alice.token.expose(), "one".into(), request())
        .await
        .unwrap();
    for name in [
        "".into(),
        "  ".into(),
        "a\nb".into(),
        "a\0b".into(),
        "a".repeat(129),
        "🦀".repeat(257),
    ] {
        assert_eq!(
            networks.create(alice.token.expose(), name, request()).await,
            Err(Error::InvalidName)
        );
    }
    for (mutation, expected) in [
        (
            "UPDATE memberships SET role = 'member'",
            Error::PermissionDenied,
        ),
        (
            "UPDATE memberships SET revoked_at = joined_at",
            Error::NotFound,
        ),
        (
            "UPDATE user_sessions SET revoked_at = created_at",
            Error::InvalidSession,
        ),
        (
            "UPDATE users SET disabled_at = created_at",
            Error::InvalidSession,
        ),
        (
            "UPDATE users SET credential_version = credential_version + 1",
            Error::InvalidSession,
        ),
    ] {
        let (started, running) = oneshot::channel();
        let (release, wait) = mpsc::channel();
        let mut revoke = Box::pin(networks.handle.execute(move |c| {
            started.send(()).unwrap();
            wait.recv().unwrap();
            c.execute_batch(mutation).unwrap();
        }));
        pending(revoke.as_mut());
        running.await.unwrap();
        let mut rename = Box::pin(networks.rename(
            alice.token.expose(),
            network.id,
            "changed".into(),
            "one".into(),
            request(),
        ));
        pending(rename.as_mut());
        release.send(()).unwrap();
        revoke.await.unwrap();
        assert_eq!(rename.await, Err(expected));
        networks.handle.execute(|c| c.execute_batch("UPDATE memberships SET role = 'owner', revoked_at = NULL; UPDATE user_sessions SET revoked_at = NULL; UPDATE users SET disabled_at = NULL, credential_version = 1").unwrap()).await.unwrap();
        assert_eq!(
            networks
                .get(alice.token.expose(), network.id)
                .await
                .unwrap()
                .name,
            "one"
        );
    }
    app.shutdown().await.unwrap();
}
