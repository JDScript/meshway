use super::*;
use crate::{
    App, Settings,
    accounts::{IssuedSession, Secret, TEST_KDF_LOCK, random_uuid},
    secrets::{CursorScope, Purpose},
};
use meshway_configuration::{AddressPools, canonical_json, new_network};
use meshway_model::{MembershipRole, NetworkActions, Revision, UserId};
use rusqlite::params;
use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::mpsc,
    task::{Context, Poll, Waker},
};
use tokio::sync::oneshot;

fn request() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}
async fn fixture() -> (tempfile::TempDir, PathBuf, App) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let app = App::open(Settings::new(&path)).await.unwrap();
    (temp, path, app)
}
async fn user(app: &App, name: &str) -> IssuedSession {
    app.accounts()
        .register(
            format!("{name}@example.test"),
            Secret::new("valid password".into()),
            request(),
        )
        .await
        .unwrap()
}
fn pending<F: Future>(future: Pin<&mut F>) {
    assert!(matches!(
        future.poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
}
async fn grant(networks: &Networks, network: NetworkId, user: UserId, role: &'static str) {
    networks.handle.execute(move |connection| {
        connection.execute("INSERT INTO memberships (id, network_id, user_id, role, version, joined_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)", params![random_uuid().unwrap().as_bytes().as_slice(), network.as_uuid().as_bytes().as_slice(), user.as_uuid().as_bytes().as_slice(), role, Utc::now().timestamp_millis()]).unwrap();
    }).await.unwrap();
}
async fn config_bytes(networks: &Networks, id: NetworkId) -> (Vec<u8>, Vec<u8>, Vec<u8>, i64, i64) {
    networks.handle.execute(move |connection| connection.query_row("SELECT c.source_jsonc, c.configuration, c.bindings, n.current_revision, n.publication_sequence FROM networks n JOIN configuration_versions c ON c.network_id = n.id AND c.revision = n.current_revision WHERE n.id = ?1", [id.as_uuid().as_bytes().as_slice()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))).unwrap()).await.unwrap()
}

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
        first.actions.rename && first.actions.edit_configuration && first.actions.manage_members
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
        Err(NetworkError::RequestConflict)
    );
    let second = networks
        .create(bob.token.expose(), "Personal".into(), req)
        .await
        .unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(
        networks.get(bob.token.expose(), first.id).await,
        Err(NetworkError::NotFound)
    );
    assert_eq!(
        networks.get(alice.token.expose(), second.id).await,
        Err(NetworkError::NotFound)
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
    let typed: configuration::ConfigurationBindings =
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
    assert_eq!(
        networks.get(alice.token.expose(), first.id).await,
        Err(NetworkError::Unavailable)
    );
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
            edit_configuration: false,
            manage_members: false
        }
    );
    assert_eq!(
        networks
            .members(bob.token.expose(), original.id, 20, None)
            .await,
        Err(NetworkError::PermissionDenied)
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
        Err(NetworkError::PermissionDenied)
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
        Err(NetworkError::NameConflict)
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
    assert_eq!(
        networks
            .create(alice.token.expose(), "rollback".into(), request())
            .await,
        Err(NetworkError::Storage)
    );
    assert_eq!(
        networks
            .rename(
                alice.token.expose(),
                original.id,
                "rollback".into(),
                "after".into(),
                request()
            )
            .await,
        Err(NetworkError::Storage)
    );
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
        Err(NetworkError::NotFound)
    );
    assert_eq!(
        networks.get(alice.token.expose(), original.id).await,
        Err(NetworkError::NotFound)
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
        Err(NetworkError::NotFound)
    );
    app.check_storage().await.unwrap();
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
        assert_eq!(creating.await, Err(NetworkError::InvalidSession));
        assert_eq!(
            networks.list(alice.token.expose(), 1, None).await,
            Err(NetworkError::InvalidSession)
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
        Err(NetworkError::InvalidCursor)
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
        Err(NetworkError::InvalidCursor)
    );
    for bad in [String::new(), "a".repeat(2049), "not/a+cursor=".into()] {
        assert_eq!(
            networks.list(alice.token.expose(), 1, Some(bad)).await,
            Err(NetworkError::InvalidCursor)
        );
    }
    for limit in [0, 201, u16::MAX] {
        assert_eq!(
            networks.list(alice.token.expose(), limit, None).await,
            Err(NetworkError::InvalidLimit)
        );
    }
    let scope = CursorScope::Networks(alice.session.user.id);
    let expired = pagination::Window::new(100, 100, Utc::now().timestamp_millis() - 16 * 60 * 1000)
        .unwrap()
        .encode(&networks.sealer, scope)
        .unwrap();
    assert_eq!(
        networks.list(alice.token.expose(), 1, Some(expired)).await,
        Err(NetworkError::InvalidCursor)
    );
    let future = pagination::Window::new(100, 100, Utc::now().timestamp_millis() + 60_000)
        .unwrap()
        .encode(&networks.sealer, scope)
        .unwrap();
    assert_eq!(
        networks.list(alice.token.expose(), 1, Some(future)).await,
        Err(NetworkError::InvalidCursor)
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
        Err(NetworkError::InvalidSession)
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
        Err(NetworkError::InvalidCursor)
    );
    assert_eq!(
        networks
            .members(bob.token.expose(), one.id, 1, Some(cursor.clone()))
            .await,
        Err(NetworkError::InvalidCursor)
    );
    assert_eq!(
        networks
            .list(alice.token.expose(), 1, Some(cursor.clone()))
            .await,
        Err(NetworkError::InvalidCursor)
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
        Err(NetworkError::PermissionDenied)
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
        Err(NetworkError::PermissionDenied)
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
        Err(NetworkError::NotFound)
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
        Err(NetworkError::NotFound)
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
            Err(NetworkError::InvalidName)
        );
    }
    for (mutation, expected) in [
        (
            "UPDATE memberships SET role = 'member'",
            NetworkError::PermissionDenied,
        ),
        (
            "UPDATE memberships SET revoked_at = joined_at",
            NetworkError::NotFound,
        ),
        (
            "UPDATE user_sessions SET revoked_at = created_at",
            NetworkError::InvalidSession,
        ),
        (
            "UPDATE users SET disabled_at = created_at",
            NetworkError::InvalidSession,
        ),
        (
            "UPDATE users SET credential_version = credential_version + 1",
            NetworkError::InvalidSession,
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
        c.execute("INSERT INTO configuration_versions SELECT network_id, 2, ?2, ?3, ?4, created_by, created_at, cause, request_id, NULL, '{}' FROM configuration_versions WHERE network_id = ?1", params![second.id.as_uuid().as_bytes().as_slice(), source.as_bytes(), config.as_bytes(), bindings.as_bytes()]).unwrap();
        let tx = c.transaction().unwrap();
        tx.execute("UPDATE networks SET current_revision = 2 WHERE id = ?1", [first.id.as_uuid().as_bytes().as_slice()]).unwrap();
        assert!(tx.commit().is_err());
        assert_eq!(c.query_row("SELECT current_revision FROM networks WHERE id = ?1", [first.id.as_uuid().as_bytes().as_slice()], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert!(c.execute("INSERT INTO configuration_versions SELECT network_id, 3, x'01', configuration, bindings, created_by, created_at, cause, request_id, NULL, '{}' FROM configuration_versions LIMIT 1", []).is_err());
    }).await.unwrap();
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}
