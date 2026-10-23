use std::fs;
use std::future::Future;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::mpsc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use rusqlite::Connection;
use tokio::sync::oneshot;

use super::*;

fn location() -> (tempfile::TempDir, PathBuf) {
    let parent = tempfile::tempdir().unwrap();
    // macOS may expose the temp parent through /var -> /private/var. The
    // configured storage path itself must not traverse symlinks.
    let path = parent.path().canonicalize().unwrap().join("data");
    (parent, path)
}

fn assert_pending<F: Future>(future: Pin<&mut F>) {
    assert!(matches!(
        future.poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
}

async fn reopen(path: &Path) -> App {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match App::open(Settings::new(path)).await {
                Ok(app) => break app,
                Err(AppError::AlreadyRunning) => tokio::time::sleep(Duration::from_millis(1)).await,
                Err(error) => panic!("reopening failed: {error}"),
            }
        }
    })
    .await
    .unwrap()
}

async fn assert_locked(path: &Path) {
    assert!(matches!(
        App::open(Settings::new(path)).await,
        Err(AppError::AlreadyRunning)
    ));
}

#[tokio::test]
async fn empty_directory_private_files_exclusion_shutdown_and_reopen() {
    let (_parent, path) = location();
    let app = App::open(Settings::new(&path)).await.unwrap();
    app.check_storage().await.unwrap();
    // The App owns runtime shutdown, not every temporary projection/provider.
    let state = app.user_state_service().unwrap();
    let provider = Arc::downgrade(&state.provider);
    drop(state);
    assert!(provider.upgrade().is_none());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o700
    );
    for name in [
        "controller.lock",
        "master-key",
        "controller.sqlite",
        "controller.sqlite-wal",
        "controller.sqlite-shm",
    ] {
        let file = path.join(name);
        if file.exists() {
            assert_eq!(
                fs::metadata(file).unwrap().permissions().mode() & 0o7777,
                0o600
            );
        }
    }
    assert_locked(&path).await;
    app.shutdown().await.unwrap();
    fs::write(path.join("controller.lock"), "lock marker preserved").unwrap();
    let app = App::open(Settings::new(&path)).await.unwrap();
    assert_eq!(
        fs::read_to_string(path.join("controller.lock")).unwrap(),
        "lock marker preserved"
    );
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn bad_settings_do_not_create_storage() {
    let (_parent, path) = location();
    let mut settings = Settings::new(&path);
    settings.database_queue_capacity = 0;
    assert!(matches!(
        App::open(settings).await,
        Err(AppError::InvalidSettings)
    ));
    assert!(!path.exists());
    assert!(matches!(
        App::open(Settings::new("relative/data")).await,
        Err(AppError::InvalidDataDirectory)
    ));
}

#[tokio::test]
async fn existing_directory_permissions_and_symlinks_are_not_taken_over() {
    let (parent, path) = location();
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        App::open(Settings::new(&path)).await,
        Err(AppError::UnsafeStorage)
    ));
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(!path.join("controller.lock").exists());
    fs::remove_dir(&path).unwrap();
    symlink(parent.path(), &path).unwrap();
    assert!(matches!(
        App::open(Settings::new(&path)).await,
        Err(AppError::UnsafeStorage)
    ));
}

#[tokio::test]
async fn writable_nonsticky_ancestors_are_rejected_before_creating_storage() {
    for mode in [0o770, 0o777] {
        let (_parent, ancestor) = location();
        fs::create_dir(&ancestor).unwrap();
        let owned = ancestor.join("owned");
        fs::create_dir(&owned).unwrap();
        fs::set_permissions(&owned, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&ancestor, fs::Permissions::from_mode(mode)).unwrap();
        // Check both the immediate parent and an earlier ancestor. A later
        // private directory cannot make an untrusted namespace safe.
        for path in [ancestor.join("data"), owned.join("data")] {
            assert!(matches!(
                App::open(Settings::new(&path)).await,
                Err(AppError::UnsafeStorage)
            ));
            assert!(!path.exists());
        }
        assert_eq!(
            fs::metadata(&ancestor).unwrap().permissions().mode() & 0o7777,
            mode
        );
        assert_eq!(fs::read_dir(&owned).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn trusted_sticky_ancestor_preserves_private_storage_and_locking() {
    let (_parent, ancestor) = location();
    fs::create_dir(&ancestor).unwrap();
    fs::set_permissions(&ancestor, fs::Permissions::from_mode(0o1777)).unwrap();
    let path = ancestor.join("data");
    let app = App::open(Settings::new(&path)).await.unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o700
    );
    assert_locked(&path).await;
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
    let app = App::open(Settings::new(&path)).await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn storage_files_reject_symlinks_hardlinks_directories_and_wide_permissions() {
    for name in [
        "master-key",
        "master-key.pending",
        "controller.lock",
        "controller.sqlite",
        "controller.sqlite-wal",
        "controller.sqlite-shm",
        "controller.sqlite-journal",
    ] {
        for kind in ["symlink", "hardlink", "directory", "wide"] {
            let (parent, path) = location();
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            let marker = parent.path().join("untouched");
            fs::write(&marker, "preserve me").unwrap();
            fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).unwrap();
            let file = path.join(name);
            match kind {
                "symlink" => symlink(&marker, &file).unwrap(),
                "hardlink" => fs::hard_link(&marker, &file).unwrap(),
                "directory" => fs::create_dir(&file).unwrap(),
                "wide" => {
                    fs::write(&file, "preserve me").unwrap();
                    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
                }
                _ => unreachable!(),
            }
            assert!(
                App::open(Settings::new(&path)).await.is_err(),
                "accepted {name}/{kind}"
            );
            assert_eq!(fs::read_to_string(marker).unwrap(), "preserve me");
            // Failure must not leave a detached worker holding the lock.
            fs::remove_dir_all(&file)
                .or_else(|_| fs::remove_file(&file))
                .unwrap();
            let app = App::open(Settings::new(&path)).await.unwrap();
            app.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn key_reopen_missing_invalid_and_mismatched_never_reset_storage() {
    use crate::secrets::Purpose;
    use meshway_model::{NetworkId, Revision};
    let (_parent, path) = location();
    let app = App::open(Settings::new(&path)).await.unwrap();
    let network = NetworkId::new(uuid::Uuid::from_bytes([1; 16])).unwrap();
    let ciphertext = app
        .sealer()
        .seal(network, Revision::MIN, Purpose::Source, b"private source")
        .unwrap();
    app.shutdown().await.unwrap();
    let key_path = path.join("master-key");
    let key = zeroize::Zeroizing::new(fs::read(&key_path).unwrap());
    let app = App::open(Settings::new(&path)).await.unwrap();
    assert!(
        app.sealer()
            .open(
                network,
                Revision::MIN,
                Purpose::Source,
                ciphertext.as_bytes()
            )
            .unwrap()
            .as_bytes()
            == b"private source"
    );
    app.shutdown().await.unwrap();
    let db_path = path.join("controller.sqlite");
    let before = fs::read(&db_path).unwrap();
    for kind in ["missing", "truncated", "secret", "id", "version"] {
        let mut bad = zeroize::Zeroizing::new(key.to_vec());
        match kind {
            "missing" => {
                fs::remove_file(&key_path).unwrap();
            }
            "truncated" => {
                bad.truncate(20);
                fs::write(&key_path, &*bad).unwrap();
            }
            "secret" => {
                bad[21] ^= 1;
                fs::write(&key_path, &*bad).unwrap();
            }
            "id" => {
                bad[5] ^= 1;
                fs::write(&key_path, &*bad).unwrap();
            }
            "version" => {
                bad[4] = 2;
                fs::write(&key_path, &*bad).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            App::open(Settings::new(&path)).await,
            Err(AppError::MasterKey)
        ));
        assert!(fs::read(&db_path).unwrap() == before);
        if kind == "missing" {
            assert!(!key_path.exists());
        } else {
            assert!(fs::read(&key_path).unwrap() == *bad);
        }
        fs::write(&key_path, &*key).unwrap();
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let app = App::open(Settings::new(&path)).await.unwrap();
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_first_initialization_and_pending_publication_reuse_same_key() {
    for pending in [false, true] {
        let (_parent, path) = location();
        let directory = Directory::open(&path).unwrap();
        let result = Executor::open(
            directory.database_path(),
            Options::default(),
            directory,
            |connection, directory| -> Result<(), AppError> {
                schema::initialize(connection, |connection, fresh| {
                    let _sealer = secrets::initialize(connection, directory, fresh)?;
                    // Schema and key binding must roll back; the durably written
                    // key remains recoverable, never replaced by a retry.
                    Err(AppError::Database)
                })
            },
        )
        .await;
        assert!(matches!(
            result,
            Err(OpenError::Initialization(AppError::Database))
        ));
        let key = zeroize::Zeroizing::new(fs::read(path.join("master-key")).unwrap());
        if pending {
            fs::rename(path.join("master-key"), path.join("master-key.pending")).unwrap();
        }
        let app = App::open(Settings::new(&path)).await.unwrap();
        assert!(fs::read(path.join("master-key")).unwrap() == *key);
        assert!(!path.join("master-key.pending").exists());
        app.check_storage().await.unwrap();
        app.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn incomplete_staged_key_is_preserved_and_publication_never_overwrites() {
    let (_parent, path) = location();
    let directory = Directory::open(&path).unwrap();
    // An interrupted partial write is diagnosed, not silently replaced.
    let pending = path.join("master-key.pending");
    fs::write(&pending, b"incomplete private key").unwrap();
    fs::set_permissions(&pending, fs::Permissions::from_mode(0o600)).unwrap();
    drop(directory);
    assert!(matches!(
        App::open(Settings::new(&path)).await,
        Err(AppError::MasterKey)
    ));
    assert_eq!(fs::read(&pending).unwrap(), b"incomplete private key");
    assert!(!path.join("master-key").exists());
    // Explicit test cleanup models operator resolution, not startup repair.
    fs::remove_file(&pending).unwrap();
    let app = App::open(Settings::new(&path)).await.unwrap();
    app.shutdown().await.unwrap();
    let key = zeroize::Zeroizing::new(fs::read(path.join("master-key")).unwrap());
    let directory = Directory::open(&path).unwrap();
    assert!(directory.publish_key(Some(b"must not replace")).is_err());
    assert!(fs::read(path.join("master-key")).unwrap() == *key);
    assert_eq!(fs::read(&pending).unwrap(), b"must not replace");
}

#[tokio::test]
async fn binding_damage_is_rejected_even_without_any_configuration_rows() {
    for mutation in [
        "DROP TRIGGER master_key_binding_no_delete; DELETE FROM master_key_binding; CREATE TRIGGER master_key_binding_no_delete BEFORE DELETE ON master_key_binding BEGIN SELECT RAISE(ABORT, 'immutable master key binding'); END;",
        "DROP TRIGGER master_key_binding_no_update; UPDATE master_key_binding SET key_check = zeroblob(61); CREATE TRIGGER master_key_binding_no_update BEFORE UPDATE ON master_key_binding BEGIN SELECT RAISE(ABORT, 'immutable master key binding'); END;",
    ] {
        let (_parent, path) = location();
        let app = App::open(Settings::new(&path)).await.unwrap();
        let key = zeroize::Zeroizing::new(fs::read(path.join("master-key")).unwrap());
        app.executor
            .handle()
            .execute(move |connection| connection.execute_batch(mutation).unwrap())
            .await
            .unwrap();
        assert!(app.check_storage().await.is_err());
        app.shutdown().await.unwrap();
        assert!(App::open(Settings::new(&path)).await.is_err());
        assert!(fs::read(path.join("master-key")).unwrap() == *key);
    }
}

#[tokio::test]
async fn rejected_database_is_not_replaced_or_reset() {
    for corrupt in [false, true] {
        let (_parent, path) = location();
        let directory = Directory::open(&path).unwrap();
        let database_path = directory.database_path();
        if corrupt {
            fs::write(&database_path, "not a sqlite database: preserved").unwrap();
        } else {
            let connection = Connection::open(&database_path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE keep (value TEXT); INSERT INTO keep VALUES ('original')",
                )
                .unwrap();
        }
        drop(directory);
        let before = fs::read(&database_path).unwrap();
        let error = App::open(Settings::new(&path)).await.err().unwrap();
        if corrupt {
            // Not a database: proven, so a service manager does not restart into it.
            assert_eq!(error, AppError::StorageIntegrity);
            assert!(error.is_persistent());
        }
        assert_eq!(fs::read(&database_path).unwrap(), before);
        assert!(!path.join("master-key").exists());
        // Rejection did not leave a lock owner alive.
        drop(Directory::open(&path).unwrap());
    }
}

/// A database whose pages are damaged is a persistent block (exit 78), not a
/// transient failure that a service manager restarts into forever.
#[tokio::test]
async fn damaged_database_pages_are_a_persistent_block() {
    let (_parent, path) = location();
    App::open(Settings::new(&path))
        .await
        .unwrap()
        .shutdown()
        .await
        .unwrap();
    let database_path = path.join("controller.sqlite");
    let mut bytes = fs::read(&database_path).unwrap();
    let page = u16::from_be_bytes([bytes[16], bytes[17]]) as usize;
    assert!(bytes.len() >= page * 4);
    // Every page after the first (the schema's tables and indexes) is garbage.
    for byte in &mut bytes[page..] {
        *byte = 0xa5;
    }
    fs::write(&database_path, &bytes).unwrap();
    let error = App::open(Settings::new(&path)).await.err().unwrap();
    assert_eq!(error, AppError::StorageIntegrity, "{error}");
    assert!(error.is_persistent());
    assert_eq!(fs::read(&database_path).unwrap(), bytes);
}

#[tokio::test]
async fn storage_diagnostic_reports_schema_damage_without_repair() {
    let (_parent, path) = location();
    let app = App::open(Settings::new(&path)).await.unwrap();
    app.executor
        .handle()
        .execute(|connection| {
            connection
                .execute_batch("DROP INDEX user_sessions_user_id")
                .unwrap();
        })
        .await
        .unwrap();
    assert_eq!(
        app.check_storage().await,
        Err(AppError::SchemaStructure(
            "missing index user_sessions_user_id".into()
        ))
    );
    app.shutdown().await.unwrap();
    assert!(matches!(
        App::open(Settings::new(&path)).await,
        Err(AppError::SchemaStructure(detail)) if detail == "missing index user_sessions_user_id"
    ));
    drop(Directory::open(&path).unwrap());
}

#[tokio::test]
async fn app_drop_and_cancelled_shutdown_hold_lock_through_accepted_commit() {
    for shutdown in [false, true] {
        let (_parent, path) = location();
        let app = App::open(Settings::new(&path)).await.unwrap();
        let handle = app.executor.handle();
        let (started, running) = oneshot::channel();
        let (release, wait) = mpsc::channel();
        let mut write = Box::pin(handle.execute(move |connection| {
            let transaction = connection.transaction().unwrap();
            transaction.execute("INSERT INTO users VALUES (x'00000000000000000000000000000001', 'user@example.test', '$argon2id$v=19$test', 1, 100, NULL)", []).unwrap();
            started.send(()).unwrap();
            wait.recv().unwrap();
            transaction.commit().unwrap();
        }));
        assert_pending(write.as_mut());
        running.await.unwrap();
        drop(write); // Accepted work outlives its cancelled caller.
        if shutdown {
            let mut closing = Box::pin(app.shutdown());
            assert_pending(closing.as_mut());
            drop(closing);
        } else {
            drop(app);
        }
        assert_locked(&path).await;
        assert_eq!(handle.execute(|_| ()).await, Err(ExecutorError::Closed));
        release.send(()).unwrap();
        let reopened = reopen(&path).await;
        let count: i64 = reopened
            .executor
            .handle()
            .execute(|connection| {
                connection
                    .query_row("SELECT count(*) FROM users", [], |r| r.get(0))
                    .unwrap()
            })
            .await
            .unwrap();
        assert_eq!(count, 1);
        reopened.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn cancelling_app_open_keeps_lock_while_sqlite_open_is_blocked() {
    let (_parent, path) = location();
    let app = App::open(Settings::new(&path)).await.unwrap();
    app.shutdown().await.unwrap();

    // Hold a real database lock so the worker cannot finish initialization.
    // DELETE mode makes BEGIN EXCLUSIVE also exclude readers during startup.
    let blocker = Connection::open(path.join("controller.sqlite")).unwrap();
    blocker
        .execute_batch("PRAGMA journal_mode = DELETE; BEGIN EXCLUSIVE")
        .unwrap();
    // The directory preparation stage has completed. Exercise cancellation at
    // the exact worker-transfer boundary used by App::open, without racing a
    // lock probe against the separate blocking filesystem task.
    let directory = Directory::open(&path).unwrap();
    let mut opening = Box::pin(App::open_directory(
        directory,
        Options::default(),
        crate::accounts::Options::new(true, Duration::from_secs(86400)).unwrap(),
        crate::resources::downloads::Downloads::new(),
        None,
        Default::default(),
        startup::Progress::none(),
    ));
    assert_pending(opening.as_mut());
    drop(opening);
    assert_locked(&path).await;
    blocker.execute_batch("ROLLBACK").unwrap();
    drop(blocker);
    let app = reopen(&path).await;
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_initialization_keeps_worker_owned_lock_until_connection_closes() {
    let (_parent, path) = location();
    let directory = Directory::open(&path).unwrap();
    let (started, running) = oneshot::channel();
    let (release, wait) = mpsc::channel();
    let mut opening = Box::pin(Executor::open(
        directory.database_path(),
        Options::default(),
        directory,
        move |connection, directory| {
            started.send(()).unwrap();
            wait.recv().unwrap();
            schema::initialize(connection, |connection, fresh| {
                secrets::initialize(connection, directory, fresh).map_err(AppError::from)
            })
        },
    ));
    assert_pending(opening.as_mut());
    running.await.unwrap();
    drop(opening);
    assert_locked(&path).await;
    release.send(()).unwrap();
    let app = reopen(&path).await;
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_startup_waiting_for_filesystem_budget_creates_nothing() {
    let (_parent, path) = location();
    let permits = DIRECTORY_PREPARATIONS
        .acquire_many(MAX_DIRECTORY_PREPARATIONS)
        .await
        .unwrap();
    let mut opening = Box::pin(App::open(Settings::new(&path)));
    assert_pending(opening.as_mut());
    drop(opening);
    assert!(!path.exists());
    drop(permits);

    let app = App::open(Settings::new(&path)).await.unwrap();
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_first_binding_retains_key_and_lock_until_worker_commit() {
    let (_parent, path) = location();
    let directory = Directory::open(&path).unwrap();
    let (started, running) = oneshot::channel();
    let (release, wait) = mpsc::channel();
    let mut opening = Box::pin(Executor::open(
        directory.database_path(),
        Options::default(),
        directory,
        move |connection, directory| {
            schema::initialize(
                connection,
                |connection, fresh| -> Result<Sealer, AppError> {
                    let sealer = secrets::initialize(connection, directory, fresh)?;
                    started.send(()).unwrap();
                    wait.recv().unwrap();
                    Ok(sealer)
                },
            )
        },
    ));
    assert_pending(opening.as_mut());
    running.await.unwrap();
    let key = zeroize::Zeroizing::new(fs::read(path.join("master-key")).unwrap());
    drop(opening);
    assert_locked(&path).await;
    release.send(()).unwrap();
    let app = reopen(&path).await;
    assert!(fs::read(path.join("master-key")).unwrap() == *key);
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}
