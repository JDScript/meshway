use std::{
    sync::{Arc, Barrier},
    time::Duration,
};

use rusqlite::{Connection, TransactionBehavior, params};
use tempfile::TempDir;

use super::*;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const NOW: i64 = 1000;
const OLD: &str = "original password secret";
const NEW: &str = "replacement password secret";

struct Fixture {
    _dir: TempDir,
    db: Connection,
    path: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.sqlite");
        let mut db = connect(&path);
        // The entire actual v9 schema, not a substitute networks table.
        db.execute_batch(include_str!("../store/schema_v9.sql"))
            .unwrap();
        for (id, initial) in [(A, 1), (B, 0)] {
            db.execute(
                "INSERT INTO networks VALUES (?1,'2026-10-01',?2)",
                params![id, initial],
            )
            .unwrap();
        }
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        tx.execute_batch(SCHEMA_FRAGMENT).unwrap();
        tx.commit().unwrap();
        Self {
            _dir: dir,
            db,
            path,
        }
    }

    fn network(&self, id: &str) -> AccountNetwork {
        Accounts::new(&self.db).network(id).unwrap()
    }

    fn transaction<T>(
        &mut self,
        f: impl FnOnce(&mut rusqlite::Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let result = f(&mut tx);
        // Deliberately commit even after domain errors: savepoint must have
        // undone all its own partial writes.
        tx.commit().unwrap();
        result
    }

    fn invite(
        &mut self,
        net: &AccountNetwork,
        email: &Email,
        role: Role,
        ttl: i64,
    ) -> IssuedSecret {
        self.transaction(|tx| create_invitation(tx, &bootstrap(), net, email, role, NOW, ttl))
            .unwrap()
    }

    async fn register(
        &mut self,
        worker: &PasswordWorker,
        net: &AccountNetwork,
        email: &Email,
        role: Role,
    ) -> Membership {
        let invite = self.invite(net, email, role, 100);
        let intent = Accounts::new(&self.db)
            .prepare_registration(invite.secret.expose(), email, NOW)
            .unwrap();
        let hash = worker.hash(password(OLD)).await.unwrap();
        self.transaction(|tx| register_invited(tx, intent, hash, NOW))
            .unwrap()
    }

    async fn login(&mut self, worker: &PasswordWorker, email: &Email) -> IssuedSecret {
        let proof = worker
            .verify(
                Accounts::new(&self.db).credential(email).unwrap(),
                password(OLD),
            )
            .await
            .unwrap();
        self.transaction(|tx| finish_login(tx, proof, NOW, 100))
            .unwrap()
    }
}

fn connect(path: &std::path::Path) -> Connection {
    let db = Connection::open(path).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    db.busy_timeout(Duration::from_secs(10)).unwrap();
    db
}

// Test-only proof factory; production can only prove the real Store bearer.
fn bootstrap() -> Principal {
    Principal(PrincipalKind::Bootstrap)
}
fn email(value: &str) -> Email {
    Email::parse(value).unwrap()
}
fn password(value: &str) -> Password {
    Password::new(value.to_owned()).unwrap()
}
async fn worker() -> PasswordWorker {
    PasswordWorker::new(2).await.unwrap()
}

#[test]
fn normalized_email_and_input_budget() {
    assert_eq!(email(" Alice@Example.COM "), email("alice@example.com"));
    assert_ne!(email("alice+work@example.com"), email("alice@example.com"));
    for invalid in ["a@@b", "x\n@y", ".a@x", "a@-x", "a..b@x", "x@é.com"] {
        assert!(Email::parse(invalid).is_err());
    }
    assert!(Password::new("short".into()).is_err());
    assert!(Password::new("a".repeat(1025)).is_err());
}

#[test]
fn explicit_schema_rollback_and_actual_v9_fk() {
    let mut db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    db.execute_batch(include_str!("../store/schema_v9.sql"))
        .unwrap();
    let tx = db.transaction().unwrap();
    tx.execute_batch(SCHEMA_FRAGMENT).unwrap();
    tx.rollback().unwrap();
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='account_users')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!exists);
    db.execute_batch(SCHEMA_FRAGMENT).unwrap();
    assert!(db.execute("INSERT INTO account_invitations VALUES ('i',zeroblob(32),'missing','a@b','member',NULL,2000,NULL,NULL,NULL)",[]).is_err());
    let version: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 0);
    assert!(Accounts::new(&db).network("missing").is_err());
}

#[tokio::test]
async fn salted_argon2_only_and_wrong_password_rejected() {
    let worker = worker().await;
    let first = worker.hash(password(OLD)).await.unwrap();
    let second = worker.hash(password(OLD)).await.unwrap();
    assert_ne!(first.0.as_str(), second.0.as_str());
    assert!(first.0.starts_with("$argon2id$v=19$m=65536,t=3,p=1$"));
    let mut f = Fixture::new();
    let net = f.network(A);
    let e = email("Alice@Example.com");
    let m = f.register(&worker, &net, &e, Role::Member).await;
    let user = Accounts::new(&f.db).user(&m.user).unwrap();
    assert!(!user.email_verified);
    assert_eq!(user.email, e);
    let phc: String =
        f.db.query_row("SELECT password_phc FROM account_users", [], |r| r.get(0))
            .unwrap();
    assert!(!phc.contains(OLD));
    assert!(matches!(
        worker
            .verify(Accounts::new(&f.db).credential(&e).unwrap(), password(NEW))
            .await,
        Err(AccountError::Unauthorized)
    ));
    assert!(matches!(
        worker.verify(None, password(OLD)).await,
        Err(AccountError::Unauthorized)
    ));
    let session = f.login(&worker, &e).await;
    let stored: Vec<u8> =
        f.db.query_row("SELECT token_hash FROM account_sessions", [], |r| r.get(0))
            .unwrap();
    assert_ne!(stored, session.secret.expose().as_bytes());
    assert_eq!(stored.len(), 32);
}

#[tokio::test]
async fn invite_expiry_revocation_replay_and_binding() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let b = f.network(B);
    let e = email("a@example.com");
    let invite = f.invite(&a, &e, Role::Member, 10);
    assert!(
        Accounts::new(&f.db)
            .prepare_registration(invite.secret.expose(), &e, NOW + 10)
            .is_err()
    );
    assert!(
        Accounts::new(&f.db)
            .prepare_registration(invite.secret.expose(), &email("other@example.com"), NOW)
            .is_err()
    );
    assert!(
        f.transaction(|tx| revoke_invitation(tx, &bootstrap(), &b, invite.secret.expose(), NOW))
            .is_err()
    );
    let intent = Accounts::new(&f.db)
        .prepare_registration(invite.secret.expose(), &e, NOW)
        .unwrap();
    let hash = worker.hash(password(OLD)).await.unwrap();
    f.transaction(|tx| revoke_invitation(tx, &bootstrap(), &a, invite.secret.expose(), NOW))
        .unwrap();
    assert!(
        f.transaction(|tx| register_invited(tx, intent, hash, NOW))
            .is_err()
    );
    assert_eq!(
        f.db.query_row("SELECT COUNT(*) FROM account_users", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let invite = f.invite(&a, &e, Role::Member, 100);
    let first = Accounts::new(&f.db)
        .prepare_registration(invite.secret.expose(), &e, NOW)
        .unwrap();
    let replay = Accounts::new(&f.db)
        .prepare_registration(invite.secret.expose(), &e, NOW)
        .unwrap();
    let h1 = worker.hash(password(OLD)).await.unwrap();
    let h2 = worker.hash(password(NEW)).await.unwrap();
    f.transaction(|tx| register_invited(tx, first, h1, NOW))
        .unwrap();
    assert!(
        f.transaction(|tx| register_invited(tx, replay, h2, NOW))
            .is_err()
    );
    assert!(
        f.db.execute(
            "UPDATE account_invitations SET network_id=?1 WHERE consumed_at IS NOT NULL",
            [B]
        )
        .is_err()
    );
}

#[tokio::test]
async fn existing_user_accepts_only_after_login_and_keeps_password() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let b = f.network(B);
    let e = email("a@example.com");
    let original = f.register(&worker, &a, &e, Role::Member).await;
    let other = f
        .register(&worker, &a, &email("other@example.com"), Role::Member)
        .await;
    let invite = f.invite(&b, &e, Role::Admin, 100);
    assert!(
        Accounts::new(&f.db)
            .prepare_registration(invite.secret.expose(), &e, NOW)
            .is_err()
    );
    assert!(
        f.transaction(|tx| accept_invitation(tx, &bootstrap(), invite.secret.expose(), NOW))
            .is_err()
    );
    let wrong = f.login(&worker, &email("other@example.com")).await;
    let wrong = Accounts::new(&f.db)
        .authenticate(wrong.secret.expose(), NOW)
        .unwrap();
    assert!(
        f.transaction(|tx| accept_invitation(tx, &wrong, invite.secret.expose(), NOW))
            .is_err()
    );
    let before: String =
        f.db.query_row(
            "SELECT password_phc FROM account_users WHERE user_id=?1",
            [&original.user.0],
            |r| r.get(0),
        )
        .unwrap();
    let session = f.login(&worker, &e).await;
    let principal = Accounts::new(&f.db)
        .authenticate(session.secret.expose(), NOW)
        .unwrap();
    let joined = f
        .transaction(|tx| accept_invitation(tx, &principal, invite.secret.expose(), NOW))
        .unwrap();
    assert_eq!(joined.user, original.user);
    assert_ne!(joined.user, other.user);
    assert_eq!(joined.network, b);
    assert_eq!(
        Accounts::new(&f.db)
            .memberships(&principal, NOW)
            .unwrap()
            .len(),
        2
    );
    assert!(
        f.transaction(|tx| accept_invitation(tx, &principal, invite.secret.expose(), NOW))
            .is_err()
    );
    let after: String =
        f.db.query_row(
            "SELECT password_phc FROM account_users WHERE user_id=?1",
            [&original.user.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
    f.login(&worker, &e).await;
}

#[tokio::test]
async fn network_roles_do_not_grant_instance_or_other_network_permission() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let b = f.network(B);
    let e = email("owner@example.com");
    let owner = f.register(&worker, &a, &e, Role::Owner).await;
    let session = f.login(&worker, &e).await;
    let p = Accounts::new(&f.db)
        .authenticate(session.secret.expose(), NOW)
        .unwrap();
    assert!(
        f.transaction(|tx| create_invitation(
            tx,
            &p,
            &a,
            &email("new@example.com"),
            Role::Owner,
            NOW,
            100
        ))
        .is_ok()
    );
    assert!(matches!(
        f.transaction(|tx| create_invitation(
            tx,
            &p,
            &b,
            &email("new@example.com"),
            Role::Member,
            NOW,
            100
        )),
        Err(AccountError::Forbidden)
    ));
    assert!(matches!(
        f.transaction(|tx| issue_reset(tx, &p, &owner.user, NOW, 100)),
        Err(AccountError::Forbidden)
    ));
    assert!(
        f.transaction(|tx| claim_first_owner(
            tx,
            &p,
            &b,
            &e,
            HashedPassword(zeroize::Zeroizing::new(String::new())),
            NOW
        ))
        .is_err()
    );
    assert!(matches!(
        f.transaction(|tx| set_membership(tx, &p, &a, &owner.user, None, NOW)),
        Err(AccountError::LastOwner)
    ));
    assert!(matches!(
        f.transaction(|tx| set_membership(
            tx,
            &bootstrap(),
            &a,
            &owner.user,
            Some(Role::Member),
            NOW
        )),
        Err(AccountError::LastOwner)
    ));
}

#[tokio::test]
async fn admin_grant_ceiling_and_inviter_demotion() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let e = email("admin@example.com");
    let admin = f.register(&worker, &a, &e, Role::Admin).await;
    let session = f.login(&worker, &e).await;
    let p = Accounts::new(&f.db)
        .authenticate(session.secret.expose(), NOW)
        .unwrap();
    let target = email("target@example.com");
    for role in [Role::Admin, Role::Owner] {
        assert!(
            f.transaction(|tx| create_invitation(tx, &p, &a, &target, role, NOW, 100))
                .is_err()
        );
    }
    let invite = f
        .transaction(|tx| create_invitation(tx, &p, &a, &target, Role::Member, NOW, 100))
        .unwrap();
    f.transaction(|tx| set_membership(tx, &bootstrap(), &a, &admin.user, Some(Role::Member), NOW))
        .unwrap();
    assert!(
        Accounts::new(&f.db)
            .prepare_registration(invite.secret.expose(), &target, NOW)
            .is_err()
    );
}

#[tokio::test]
async fn reset_invalidates_sessions_other_resets_and_stale_login_proof() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let e = email("a@example.com");
    let member = f.register(&worker, &a, &e, Role::Member).await;
    let s = f.login(&worker, &e).await;
    let p = Accounts::new(&f.db)
        .authenticate(s.secret.expose(), NOW)
        .unwrap();
    // Snapshot before reset, CPU verification deliberately finishes after reset.
    let snapshot = Accounts::new(&f.db).credential(&e).unwrap();
    let reset = f
        .transaction(|tx| issue_reset(tx, &bootstrap(), &member.user, NOW, 100))
        .unwrap();
    let other = f
        .transaction(|tx| issue_reset(tx, &bootstrap(), &member.user, NOW, 100))
        .unwrap();
    let intent = Accounts::new(&f.db)
        .prepare_reset(reset.secret.expose(), NOW)
        .unwrap();
    let replay = Accounts::new(&f.db)
        .prepare_reset(reset.secret.expose(), NOW)
        .unwrap();
    let hash = worker.hash(password(NEW)).await.unwrap();
    f.transaction(|tx| redeem_reset(tx, intent, hash, NOW))
        .unwrap();
    let stale = worker.verify(snapshot, password(OLD)).await.unwrap();
    assert!(matches!(
        f.transaction(|tx| finish_login(tx, stale, NOW, 100)),
        Err(AccountError::Unauthorized)
    ));
    assert!(
        Accounts::new(&f.db)
            .authenticate(s.secret.expose(), NOW)
            .is_err()
    );
    assert!(Accounts::new(&f.db).memberships(&p, NOW).is_err());
    assert!(
        Accounts::new(&f.db)
            .prepare_reset(other.secret.expose(), NOW)
            .is_err()
    );
    let hash = worker.hash(password(OLD)).await.unwrap();
    assert!(
        f.transaction(|tx| redeem_reset(tx, replay, hash, NOW))
            .is_err()
    );
    assert!(
        worker
            .verify(Accounts::new(&f.db).credential(&e).unwrap(), password(OLD))
            .await
            .is_err()
    );
    let proof = worker
        .verify(Accounts::new(&f.db).credential(&e).unwrap(), password(NEW))
        .await
        .unwrap();
    f.transaction(|tx| finish_login(tx, proof, NOW, 100))
        .unwrap();
}

#[tokio::test]
async fn reset_expiry_revoke_and_secret_class_separation() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let e = email("a@example.com");
    let m = f.register(&worker, &a, &e, Role::Member).await;
    let reset = f
        .transaction(|tx| issue_reset(tx, &bootstrap(), &m.user, NOW, 10))
        .unwrap();
    assert!(
        Accounts::new(&f.db)
            .prepare_reset(reset.secret.expose(), NOW + 10)
            .is_err()
    );
    let invite = f.invite(&a, &email("b@example.com"), Role::Member, 100);
    let session = f.login(&worker, &e).await;
    for secret in [
        invite.secret.expose(),
        reset.secret.expose(),
        session.secret.expose(),
    ] {
        if secret != session.secret.expose() {
            assert!(Accounts::new(&f.db).authenticate(secret, NOW).is_err());
        }
        if secret != reset.secret.expose() {
            assert!(Accounts::new(&f.db).prepare_reset(secret, NOW).is_err());
        }
        if secret != invite.secret.expose() {
            assert!(
                Accounts::new(&f.db)
                    .prepare_registration(secret, &e, NOW)
                    .is_err()
            );
        }
    }
    let node = meshway_protocol::local::credential().unwrap();
    assert!(Accounts::new(&f.db).authenticate(&node, NOW).is_err());
    assert!(
        f.transaction(|tx| create_invitation(
            tx,
            &Principal::node(),
            &a,
            &e,
            Role::Member,
            NOW,
            100
        ))
        .is_err()
    );
    f.transaction(|tx| revoke_reset(tx, &bootstrap(), reset.secret.expose(), NOW))
        .unwrap();
    assert!(
        Accounts::new(&f.db)
            .prepare_reset(reset.secret.expose(), NOW)
            .is_err()
    );
}

#[tokio::test]
async fn logout_expiry_and_password_change() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let e = email("a@example.com");
    f.register(&worker, &a, &e, Role::Member).await;
    let session = f.login(&worker, &e).await;
    assert!(
        Accounts::new(&f.db)
            .authenticate(session.secret.expose(), NOW + 100)
            .is_err()
    );
    let p = Accounts::new(&f.db)
        .authenticate(session.secret.expose(), NOW)
        .unwrap();
    f.transaction(|tx| logout(tx, &p, NOW)).unwrap();
    assert!(
        Accounts::new(&f.db)
            .authenticate(session.secret.expose(), NOW)
            .is_err()
    );
    let session = f.login(&worker, &e).await;
    let p = Accounts::new(&f.db)
        .authenticate(session.secret.expose(), NOW)
        .unwrap();
    let proof = worker
        .verify(Accounts::new(&f.db).credential(&e).unwrap(), password(OLD))
        .await
        .unwrap();
    let hash = worker.hash(password(NEW)).await.unwrap();
    f.transaction(|tx| change_password(tx, &p, proof, hash, NOW))
        .unwrap();
    assert!(
        Accounts::new(&f.db)
            .authenticate(session.secret.expose(), NOW)
            .is_err()
    );
    assert!(
        worker
            .verify(Accounts::new(&f.db).credential(&e).unwrap(), password(OLD))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn failed_membership_insert_rolls_back_account_and_invite() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let e = email("a@example.com");
    let invite = f.invite(&a, &e, Role::Member, 100);
    let intent = Accounts::new(&f.db)
        .prepare_registration(invite.secret.expose(), &e, NOW)
        .unwrap();
    let hash = worker.hash(password(OLD)).await.unwrap();
    f.db.execute_batch("CREATE TRIGGER fail_join BEFORE INSERT ON account_memberships BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
    assert!(
        f.transaction(|tx| register_invited(tx, intent, hash, NOW))
            .is_err()
    );
    assert_eq!(
        f.db.query_row("SELECT COUNT(*) FROM account_users", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.db.query_row("SELECT COUNT(*) FROM account_memberships", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(
        Accounts::new(&f.db)
            .prepare_registration(invite.secret.expose(), &e, NOW)
            .is_ok()
    );
}

#[tokio::test]
async fn two_connections_consume_invitation_exactly_once() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let e = email("a@example.com");
    let invite = f.invite(&a, &e, Role::Member, 100);
    let intent1 = Accounts::new(&f.db)
        .prepare_registration(invite.secret.expose(), &e, NOW)
        .unwrap();
    let intent2 = Accounts::new(&f.db)
        .prepare_registration(invite.secret.expose(), &e, NOW)
        .unwrap();
    let h1 = worker.hash(password(OLD)).await.unwrap();
    let h2 = worker.hash(password(OLD)).await.unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles = [(intent1, h1), (intent2, h2)]
        .into_iter()
        .map(|(intent, hash)| {
            let barrier = barrier.clone();
            let path = f.path.clone();
            std::thread::spawn(move || {
                let mut db = connect(&path);
                barrier.wait();
                let mut tx = db
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .unwrap();
                let result = register_invited(&mut tx, intent, hash, NOW);
                tx.commit().unwrap();
                result.is_ok()
            })
        })
        .collect::<Vec<_>>();
    let successes = handles
        .into_iter()
        .filter_map(|h| h.join().unwrap().then_some(()))
        .count();
    assert_eq!(successes, 1);
    for table in ["account_users", "account_memberships"] {
        assert_eq!(
            f.db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn bootstrap_claim_requires_real_proof_and_never_replaces_user() {
    let dir = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = crate::store::Store::open(dir.path(), &crate::NetworkOptions::default()).unwrap();
    let token = std::fs::read_to_string(dir.path().join("admin.token")).unwrap();
    assert!(Principal::prove_bootstrap(&store, "wrong").is_err());
    let proof = Principal::prove_bootstrap(&store, token.trim()).unwrap();
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let b = f.network(B);
    let e = email("real@example.com");
    let hash = worker.hash(password(OLD)).await.unwrap();
    let owner = f
        .transaction(|tx| claim_first_owner(tx, &proof, &a, &e, hash, NOW))
        .unwrap();
    let hash = worker.hash(password(NEW)).await.unwrap();
    assert!(matches!(
        f.transaction(|tx| claim_first_owner(tx, &proof, &b, &e, hash, NOW)),
        Err(AccountError::Conflict)
    ));
    let s = f.login(&worker, &e).await;
    let p = Accounts::new(&f.db)
        .authenticate(s.secret.expose(), NOW)
        .unwrap();
    let second = f
        .transaction(|tx| claim_existing_owner(tx, &proof, &p, &b, NOW))
        .unwrap();
    assert_eq!(owner.user, second.user);
    assert_eq!(second.role, Role::Owner);
    assert_eq!(Accounts::new(&f.db).memberships(&p, NOW).unwrap().len(), 2);
}

#[tokio::test]
async fn kdf_limit_rejects_without_queueing() {
    let worker = PasswordWorker::new(1).await.unwrap();
    let permit = worker.budget.clone().try_acquire_owned().unwrap();
    assert!(matches!(
        worker.hash(password(OLD)).await,
        Err(AccountError::Busy)
    ));
    assert!(matches!(
        worker.verify(None, password(OLD)).await,
        Err(AccountError::Busy)
    ));
    drop(permit);
    assert!(PasswordWorker::new(0).await.is_err());
    assert!(PasswordWorker::new(9).await.is_err());
    worker.hash(password(OLD)).await.unwrap();
}

#[tokio::test]
async fn browser_session_selects_only_member_network_and_device_owner() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let b = f.network(B);
    let e = email("member@example.com");
    let member = f.register(&worker, &a, &e, Role::Member).await;
    let s = f.login(&worker, &e).await;
    let p = Accounts::new(&f.db)
        .authenticate(s.secret.expose(), NOW)
        .unwrap();
    let repo = Accounts::new(&f.db);
    assert!(
        repo.authorize_network(&p, &a, NetworkPermission::Read, NOW)
            .is_ok()
    );
    assert!(
        repo.authorize_network(&p, &a, NetworkPermission::Manage, NOW)
            .is_err()
    );
    assert!(
        repo.authorize_network(&p, &b, NetworkPermission::Read, NOW)
            .is_err()
    );
    let subject: DeviceAuthorizationSubject = repo.authorize_device(&p, &a, NOW).unwrap();
    assert_eq!(subject.owner(), &member);
    assert_eq!(subject.binding().user, member.user);
    assert_eq!(subject.binding().network, a);
    assert!(repo.authorize_device(&p, &b, NOW).is_err());
    assert!(repo.authorize_device(&bootstrap(), &a, NOW).is_err());
    assert!(repo.authorize_device(&Principal::node(), &a, NOW).is_err());
    assert!(repo.revalidate_device(subject.binding(), &a, NOW).is_ok());
    let tampered = DeviceAuthorizationBinding {
        network: a.clone(),
        user: UserId("missing-user".into()),
        membership_id: subject.binding().membership_id.clone(),
        session_digest: subject.binding().session_digest.clone(),
        credential_version: subject.binding().credential_version,
    };
    assert!(repo.revalidate_device(&tampered, &a, NOW).is_err());
    assert!(repo.revalidate_device(subject.binding(), &b, NOW).is_err());
    assert!(
        repo.revalidate_device(subject.binding(), &a, NOW + 100)
            .is_err()
    );
}

#[tokio::test]
async fn device_approval_rechecks_logout_reset_and_membership_loss() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let e = email("member@example.com");
    let member = f.register(&worker, &a, &e, Role::Member).await;
    let s = f.login(&worker, &e).await;
    let p = Accounts::new(&f.db)
        .authenticate(s.secret.expose(), NOW)
        .unwrap();
    let approval = Accounts::new(&f.db).authorize_device(&p, &a, NOW).unwrap();
    f.transaction(|tx| logout(tx, &p, NOW)).unwrap();
    assert!(
        Accounts::new(&f.db)
            .revalidate_device(approval.binding(), &a, NOW)
            .is_err()
    );
    let s = f.login(&worker, &e).await;
    let p = Accounts::new(&f.db)
        .authenticate(s.secret.expose(), NOW)
        .unwrap();
    let approval = Accounts::new(&f.db).authorize_device(&p, &a, NOW).unwrap();
    let reset = f
        .transaction(|tx| issue_reset(tx, &bootstrap(), &member.user, NOW, 100))
        .unwrap();
    let intent = Accounts::new(&f.db)
        .prepare_reset(reset.secret.expose(), NOW)
        .unwrap();
    let hash = worker.hash(password(OLD)).await.unwrap();
    f.transaction(|tx| redeem_reset(tx, intent, hash, NOW))
        .unwrap();
    assert!(
        Accounts::new(&f.db)
            .revalidate_device(approval.binding(), &a, NOW)
            .is_err()
    );
    let s = f.login(&worker, &e).await;
    let p = Accounts::new(&f.db)
        .authenticate(s.secret.expose(), NOW)
        .unwrap();
    let approval = Accounts::new(&f.db).authorize_device(&p, &a, NOW).unwrap();
    f.transaction(|tx| set_membership(tx, &bootstrap(), &a, &member.user, None, NOW))
        .unwrap();
    assert!(
        Accounts::new(&f.db)
            .authenticate(s.secret.expose(), NOW)
            .is_ok()
    );
    assert!(
        Accounts::new(&f.db)
            .revalidate_device(approval.binding(), &a, NOW)
            .is_err()
    );
    // Rejoining never resurrects an approval made before membership removal.
    let invitation = f.invite(&a, &e, Role::Member, 100);
    let rejoined = f
        .transaction(|tx| accept_invitation(tx, &p, invitation.secret.expose(), NOW))
        .unwrap();
    assert_ne!(rejoined.id, member.id);
    let revoked: Option<i64> =
        f.db.query_row(
            "SELECT revoked_at FROM account_memberships WHERE membership_id=?1",
            [&member.id.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(revoked, Some(NOW));
    assert!(
        f.db.execute(
            "UPDATE account_memberships SET revoked_at=NULL WHERE membership_id=?1",
            [&member.id.0]
        )
        .is_err()
    );
    assert!(
        Accounts::new(&f.db)
            .revalidate_device(approval.binding(), &a, NOW)
            .is_err()
    );
    assert!(Accounts::new(&f.db).authorize_device(&p, &a, NOW).is_ok());
}

#[tokio::test]
async fn owner_transfer_and_user_session_revocation_are_scoped() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let owner_email = email("owner@example.com");
    let member_email = email("member@example.com");
    let owner = f.register(&worker, &a, &owner_email, Role::Owner).await;
    let member = f.register(&worker, &a, &member_email, Role::Member).await;
    let so = f.login(&worker, &owner_email).await;
    let sm = f.login(&worker, &member_email).await;
    let p = Accounts::new(&f.db)
        .authenticate(so.secret.expose(), NOW)
        .unwrap();
    assert!(matches!(
        f.transaction(|tx| revoke_user_sessions(tx, &p, &member.user, NOW)),
        Err(AccountError::Forbidden)
    ));
    f.transaction(|tx| {
        set_membership(tx, &p, &a, &member.user, Some(Role::Owner), NOW)?;
        set_membership(tx, &p, &a, &owner.user, Some(Role::Member), NOW)
    })
    .unwrap();
    assert_eq!(
        Accounts::new(&f.db).role(&a, &member.user).unwrap(),
        Some(Role::Owner)
    );
    assert_eq!(
        Accounts::new(&f.db).role(&a, &owner.user).unwrap(),
        Some(Role::Member)
    );
    f.transaction(|tx| revoke_user_sessions(tx, &bootstrap(), &member.user, NOW))
        .unwrap();
    assert!(
        Accounts::new(&f.db)
            .authenticate(sm.secret.expose(), NOW)
            .is_err()
    );
    assert!(
        Accounts::new(&f.db)
            .authenticate(so.secret.expose(), NOW)
            .is_ok()
    );
}

#[tokio::test]
async fn invitation_consumption_failure_rolls_back_membership_and_account() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let e = email("member@example.com");
    let invite = f.invite(&a, &e, Role::Member, 100);
    let intent = Accounts::new(&f.db)
        .prepare_registration(invite.secret.expose(), &e, NOW)
        .unwrap();
    let hash = worker.hash(password(OLD)).await.unwrap();
    f.db.execute_batch("CREATE TRIGGER fail_consume BEFORE UPDATE OF consumed_at ON account_invitations BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
    assert!(
        f.transaction(|tx| register_invited(tx, intent, hash, NOW))
            .is_err()
    );
    for table in ["account_users", "account_memberships"] {
        assert_eq!(
            f.db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert!(
        Accounts::new(&f.db)
            .prepare_registration(invite.secret.expose(), &e, NOW)
            .is_ok()
    );
}

#[tokio::test]
async fn reset_storage_failure_preserves_password_session_and_unused_token() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let e = email("member@example.com");
    let member = f.register(&worker, &a, &e, Role::Member).await;
    let session = f.login(&worker, &e).await;
    let reset = f
        .transaction(|tx| issue_reset(tx, &bootstrap(), &member.user, NOW, 100))
        .unwrap();
    let intent = Accounts::new(&f.db)
        .prepare_reset(reset.secret.expose(), NOW)
        .unwrap();
    let hash = worker.hash(password(NEW)).await.unwrap();
    f.db.execute_batch("CREATE TRIGGER fail_session_revoke BEFORE UPDATE ON account_sessions BEGIN SELECT RAISE(ABORT,'SECRET_SENTINEL'); END;").unwrap();
    let result = f.transaction(|tx| redeem_reset(tx, intent, hash, NOW));
    assert!(matches!(result, Err(AccountError::Storage)));
    let err = result.err().unwrap();
    assert!(!format!("{err:?}: {err}").contains("SECRET_SENTINEL"));
    assert!(
        Accounts::new(&f.db)
            .authenticate(session.secret.expose(), NOW)
            .is_ok()
    );
    assert!(
        Accounts::new(&f.db)
            .prepare_reset(reset.secret.expose(), NOW)
            .is_ok()
    );
    assert!(
        worker
            .verify(Accounts::new(&f.db).credential(&e).unwrap(), password(OLD))
            .await
            .is_ok()
    );
    assert!(
        worker
            .verify(Accounts::new(&f.db).credential(&e).unwrap(), password(NEW))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn two_connections_consume_reset_exactly_once() {
    let worker = worker().await;
    let mut f = Fixture::new();
    let a = f.network(A);
    let e = email("member@example.com");
    let member = f.register(&worker, &a, &e, Role::Member).await;
    let reset = f
        .transaction(|tx| issue_reset(tx, &bootstrap(), &member.user, NOW, 100))
        .unwrap();
    let i1 = Accounts::new(&f.db)
        .prepare_reset(reset.secret.expose(), NOW)
        .unwrap();
    let i2 = Accounts::new(&f.db)
        .prepare_reset(reset.secret.expose(), NOW)
        .unwrap();
    let h1 = worker.hash(password(NEW)).await.unwrap();
    let h2 = worker.hash(password(NEW)).await.unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles = [(i1, h1), (i2, h2)]
        .into_iter()
        .map(|(intent, hash)| {
            let barrier = barrier.clone();
            let path = f.path.clone();
            std::thread::spawn(move || {
                let mut db = connect(&path);
                barrier.wait();
                let mut tx = db
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .unwrap();
                let result = redeem_reset(&mut tx, intent, hash, NOW);
                tx.commit().unwrap();
                result.is_ok()
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        handles
            .into_iter()
            .filter_map(|h| h.join().unwrap().then_some(()))
            .count(),
        1
    );
    let version: i64 =
        f.db.query_row(
            "SELECT credential_version FROM account_users WHERE user_id=?1",
            [&member.user.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(version, 2);
}
