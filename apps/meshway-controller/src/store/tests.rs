use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc;
use std::task::{Context, Poll, Waker};

use super::*;

fn temporary_directory() -> tempfile::TempDir {
    // SQLite NOFOLLOW rejects symlinks anywhere in the path, including the
    // macOS system temp alias /var -> /private/var.
    tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
}

fn assert_pending<F: Future>(future: Pin<&mut F>) {
    assert!(matches!(
        future.poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
}

async fn database(path: PathBuf, queue_capacity: usize) -> Executor {
    Executor::open(
        path,
        Options {
            queue_capacity,
            ..Options::default()
        },
        (),
        |connection, _| {
            connection.execute_batch(
                "CREATE TABLE IF NOT EXISTS entries (id INTEGER PRIMARY KEY, value TEXT NOT NULL)",
            )
        },
    )
    .await
    .unwrap()
    .0
}

async fn count(handle: &Handle) -> i64 {
    handle
        .execute(|connection| {
            connection.query_row("SELECT count(*) FROM entries", [], |row| row.get(0))
        })
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn connection_settings_transactions_and_persistent_reopen() {
    let directory = temporary_directory();
    let path = directory.path().join("controller.sqlite");
    let caller_thread = thread::current().id();
    let (owner, ()) = Executor::open(
        path.clone(),
        Options::default(),
        (),
        move |connection, _| {
            assert_ne!(thread::current().id(), caller_thread);
            let foreign_keys: bool =
                connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
            let synchronous: i64 =
                connection.pragma_query_value(None, "synchronous", |row| row.get(0))?;
            let timeout: i64 =
                connection.pragma_query_value(None, "busy_timeout", |row| row.get(0))?;
            assert!(foreign_keys);
            assert_eq!(synchronous, 2);
            assert_eq!(timeout, 5000);
            connection.execute_batch(
                "CREATE TABLE entries (id INTEGER PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE children (parent INTEGER REFERENCES entries(id));",
            )
        },
    )
    .await
    .unwrap();
    let handle = owner.handle();
    let journal: String = handle
        .execute(|connection| connection.pragma_query_value(None, "journal_mode", |row| row.get(0)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(journal, "wal");
    handle
        .execute(|connection| -> rusqlite::Result<()> {
            let transaction = connection.transaction()?;
            transaction.execute("INSERT INTO entries VALUES (1, 'committed')", [])?;
            transaction.commit()
        })
        .await
        .unwrap()
        .unwrap();
    let rejected = handle
        .execute(|connection| -> rusqlite::Result<()> {
            let transaction = connection.transaction()?;
            transaction.execute("INSERT INTO entries VALUES (2, 'rollback')", [])?;
            transaction.execute("INSERT INTO children VALUES (999)", [])?;
            transaction.commit()
        })
        .await
        .unwrap();
    assert!(matches!(rejected, Err(rusqlite::Error::SqliteFailure(..))));
    assert_eq!(count(&handle).await, 1);
    owner.shutdown().await.unwrap();
    assert_eq!(handle.execute(|_| ()).await, Err(ExecutorError::Closed));

    let reopened = database(path, 1).await;
    assert_eq!(count(&reopened.handle()).await, 1);
    reopened.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_accepted_transaction_still_commits() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let directory = temporary_directory();
        let owner = database(directory.path().join("controller.sqlite"), 1).await;
        let handle = owner.handle();
        let (started, running) = oneshot::channel();
        let (release, wait) = mpsc::channel();
        let mut write = Box::pin(handle.execute(move |connection| -> rusqlite::Result<()> {
            let transaction = connection.transaction()?;
            transaction.execute("INSERT INTO entries VALUES (1, 'survives cancel')", [])?;
            started.send(()).unwrap();
            wait.recv().unwrap();
            transaction.commit()
        }));
        assert_pending(write.as_mut());
        running.await.unwrap();
        drop(write);
        release.send(()).unwrap();
        assert_eq!(count(&handle).await, 1);
        owner.shutdown().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bounded_admission_shutdown_drains_cancelled_queue_and_joins() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let directory = temporary_directory();
        let path = directory.path().join("controller.sqlite");
        let owner = database(path.clone(), 1).await;
        let handle = owner.handle();
        let (started, running) = oneshot::channel();
        let (release, wait) = mpsc::channel();
        let mut first = Box::pin(handle.execute(move |_| {
            started.send(()).unwrap();
            wait.recv().unwrap();
        }));
        assert_pending(first.as_mut());
        running.await.unwrap();

        let mut accepted = Box::pin(handle.execute(|connection| {
            connection
                .execute("INSERT INTO entries VALUES (1, 'queued')", [])
                .unwrap();
        }));
        assert_pending(accepted.as_mut());
        assert_eq!(handle.shared.queue().jobs.len(), 1);
        // This captures an observable side effect but cannot enter the full queue.
        let mut waiting = Box::pin(handle.execute(|connection| {
            connection
                .execute("INSERT INTO entries VALUES (2, 'not accepted')", [])
                .unwrap();
        }));
        assert_pending(waiting.as_mut());
        assert_eq!(handle.shared.queue().jobs.len(), 1);
        drop(accepted);

        let mut shutdown = Box::pin(owner.shutdown());
        assert_pending(shutdown.as_mut());
        assert_eq!(waiting.await, Err(ExecutorError::Closed));
        assert_eq!(handle.execute(|_| ()).await, Err(ExecutorError::Closed));
        assert_pending(shutdown.as_mut());
        release.send(()).unwrap();
        first.await.unwrap();
        shutdown.await.unwrap();

        let reopened = database(path, 1).await;
        assert_eq!(count(&reopened.handle()).await, 1);
        reopened.shutdown().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancellation_before_admission_never_runs_operation() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let directory = temporary_directory();
        let owner = database(directory.path().join("controller.sqlite"), 1).await;
        let handle = owner.handle();
        let (started, running) = oneshot::channel();
        let (release, wait) = mpsc::channel();
        let mut first = Box::pin(handle.execute(move |_| {
            started.send(()).unwrap();
            wait.recv().unwrap();
        }));
        assert_pending(first.as_mut());
        running.await.unwrap();
        let mut queued = Box::pin(handle.execute(|_| ()));
        assert_pending(queued.as_mut());
        let mut cancelled = Box::pin(handle.execute(|connection| {
            connection
                .execute("INSERT INTO entries VALUES (1, 'must not run')", [])
                .unwrap();
        }));
        assert_pending(cancelled.as_mut());
        drop(cancelled);
        release.send(()).unwrap();
        first.await.unwrap();
        queued.await.unwrap();
        assert_eq!(count(&handle).await, 0);
        owner.shutdown().await.unwrap();
    })
    .await
    .unwrap();
}

#[derive(Debug, PartialEq, Eq)]
enum SchemaError {
    Rejected,
}

#[tokio::test]
async fn initialization_rejection_preserves_existing_database() {
    let directory = temporary_directory();
    let path = directory.path().join("controller.sqlite");
    let owner = database(path.clone(), 1).await;
    owner
        .handle()
        .execute(|connection| {
            connection
                .execute("INSERT INTO entries VALUES (1, 'original')", [])
                .unwrap()
        })
        .await
        .unwrap();
    owner.shutdown().await.unwrap();

    let result = Executor::open(path.clone(), Options::default(), (), |connection, _| {
        let transaction = connection.transaction().unwrap();
        transaction.execute("DELETE FROM entries", []).unwrap();
        Err::<(), _>(SchemaError::Rejected)
    })
    .await;
    assert!(matches!(
        result,
        Err(OpenError::Initialization(SchemaError::Rejected))
    ));
    let reopened = database(path, 1).await;
    assert_eq!(count(&reopened.handle()).await, 1);
    reopened.shutdown().await.unwrap();
}

#[tokio::test]
async fn worker_panic_closes_running_queued_waiting_and_future_calls() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let directory = temporary_directory();
        let path = directory.path().join("controller.sqlite");
        let owner = database(path.clone(), 1).await;
        let handle = owner.handle();
        let (started, running) = oneshot::channel();
        let (release, wait) = mpsc::channel();
        let mut panicking = Box::pin(handle.execute(move |connection| {
            let transaction = connection.transaction().unwrap();
            transaction
                .execute("INSERT INTO entries VALUES (1, 'rollback on panic')", [])
                .unwrap();
            started.send(()).unwrap();
            wait.recv().unwrap();
            panic!("test worker failure");
        }));
        assert_pending(panicking.as_mut());
        running.await.unwrap();
        let mut queued = Box::pin(handle.execute(|_| panic!("queued work must not run")));
        assert_pending(queued.as_mut());
        let mut waiting = Box::pin(handle.execute(|_| panic!("waiting work must not run")));
        assert_pending(waiting.as_mut());
        release.send(()).unwrap();
        assert_eq!(panicking.await, Err(ExecutorError::Closed));
        assert_eq!(queued.await, Err(ExecutorError::Closed));
        assert_eq!(waiting.await, Err(ExecutorError::Closed));
        assert_eq!(handle.execute(|_| ()).await, Err(ExecutorError::Closed));
        assert_eq!(owner.shutdown().await, Err(ExecutorError::WorkerPanicked));

        let reopened = database(path, 1).await;
        assert_eq!(count(&reopened.handle()).await, 0);
        reopened.shutdown().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn initialization_panic_joins_and_reports_failure() {
    let directory = temporary_directory();
    let result = Executor::open(
        directory.path().join("controller.sqlite"),
        Options::default(),
        (),
        |_, _| -> Result<(), SchemaError> { panic!("test initialization failure") },
    )
    .await;
    assert!(matches!(
        result,
        Err(OpenError::Executor(ExecutorError::WorkerPanicked))
    ));
}

#[tokio::test]
async fn owner_drop_does_not_block_runtime_and_still_drains() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let directory = temporary_directory();
        let owner = database(directory.path().join("controller.sqlite"), 1).await;
        let handle = owner.handle();
        let (started, running) = oneshot::channel();
        let (release, wait) = mpsc::channel();
        let mut first = Box::pin(handle.execute(move |_| {
            started.send(()).unwrap();
            wait.recv().unwrap();
        }));
        assert_pending(first.as_mut());
        running.await.unwrap();
        let mut queued = Box::pin(handle.execute(|_| 42));
        assert_pending(queued.as_mut());
        // Current-thread runtime: a blocking Drop would deadlock before release.
        drop(owner);
        assert_eq!(handle.execute(|_| ()).await, Err(ExecutorError::Closed));
        release.send(()).unwrap();
        first.await.unwrap();
        assert_eq!(queued.await.unwrap(), 42);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn invalid_options_and_open_failure_are_explicit_and_redacted() {
    let directory = temporary_directory();
    let path = directory.path().join("must-not-be-created.sqlite");
    for queue_capacity in [0, 4097, usize::MAX] {
        let result = Executor::open(
            path.clone(),
            Options {
                queue_capacity,
                ..Options::default()
            },
            (),
            |_, _| Ok::<_, SchemaError>(()),
        )
        .await;
        assert!(matches!(
            result,
            Err(OpenError::Executor(ExecutorError::InvalidQueueCapacity))
        ));
    }
    for busy_timeout in [Duration::ZERO, Duration::from_nanos(1), Duration::MAX] {
        let result = Executor::open(
            path.clone(),
            Options {
                busy_timeout,
                ..Options::default()
            },
            (),
            |_, _| Ok::<_, SchemaError>(()),
        )
        .await;
        assert!(matches!(
            result,
            Err(OpenError::Executor(ExecutorError::InvalidBusyTimeout))
        ));
    }
    assert!(!path.exists());
    let result = Executor::open(
        directory.path().to_owned(),
        Options::default(),
        (),
        |_, _| Ok::<_, SchemaError>(()),
    )
    .await;
    assert!(matches!(
        result,
        Err(OpenError::Executor(ExecutorError::Sqlite { .. }))
    ));
    let error = OpenError::Initialization("secret SQL and material");
    assert_eq!(error.to_string(), "database initialization rejected");
    assert_eq!(format!("{error:?}"), "Initialization(<redacted>)");
    let sqlite = ExecutorError::from(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(1),
        Some("secret SQL and material".into()),
    ));
    assert!(!format!("{sqlite} {sqlite:?}").contains("secret"));
}
