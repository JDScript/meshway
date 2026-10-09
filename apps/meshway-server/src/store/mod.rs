//! A bounded, single-connection SQLite executor.
//!
//! The application supplies a private database path and a schema initializer.
//! Initializers and operations run only on the dedicated thread. Operations may
//! return a use-case-specific `Result`; transaction commit/rollback is their
//! responsibility, and the executor never retries them.
//!
//! Queue insertion is the acceptance boundary. Cancellation before insertion
//! cancels the operation; cancellation afterwards only discards its reply.
//! Capacity limits waiting operations (one additional operation may be running).
//! Callers should also bound request concurrency and captured input sizes.

use std::collections::VecDeque;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rusqlite::{Connection, Transaction, TransactionBehavior};
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};

use crate::effect::{Effect, EffectSink};

pub(crate) mod accounts;
pub(crate) mod authorization;
pub(crate) mod configuration;
pub(crate) mod enrollment;
pub(crate) mod execution;
pub(crate) mod history;
pub(crate) mod keys;
pub(crate) mod netmap;
pub(crate) mod networks;
pub(crate) mod observations;
pub(crate) mod resources;
pub(crate) mod schema;

/// Execution infrastructure failures contain no SQL, paths, or SQLite messages.
#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ExecutorError {
    #[error("database queue capacity must be between 1 and 4096")]
    InvalidQueueCapacity,
    #[error("database busy timeout must be between 1 millisecond and 60 seconds")]
    InvalidBusyTimeout,
    #[error("database worker could not be started")]
    ThreadStart,
    #[error("database operation failed (SQLite code {code}, extended code {extended_code})")]
    Sqlite { code: i32, extended_code: i32 },
    #[error("database connection configuration failed")]
    ConnectionConfiguration,
    #[error("database executor is closed")]
    Closed,
    #[error("database worker panicked")]
    WorkerPanicked,
    #[error("database worker join task failed")]
    JoinFailed,
}

impl From<rusqlite::Error> for ExecutorError {
    fn from(error: rusqlite::Error) -> Self {
        match error {
            rusqlite::Error::SqliteFailure(error, _) => Self::Sqlite {
                code: error.extended_code & 0xff,
                extended_code: error.extended_code,
            },
            _ => Self::ConnectionConfiguration,
        }
    }
}

/// The schema owner retains its typed error; executor formatting never prints it.
#[derive(Error)]
pub(crate) enum OpenError<E> {
    #[error(transparent)]
    Executor(#[from] ExecutorError),
    #[error("database initialization rejected")]
    Initialization(E),
}

impl<E> fmt::Debug for OpenError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Executor(error) => formatter.debug_tuple("Executor").field(error).finish(),
            Self::Initialization(_) => formatter.write_str("Initialization(<redacted>)"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Options {
    pub(crate) queue_capacity: usize,
    pub(crate) busy_timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            queue_capacity: 64,
            busy_timeout: Duration::from_secs(5),
        }
    }
}

impl Options {
    pub(crate) fn validate(self) -> Result<(), ExecutorError> {
        if !(1..=4096).contains(&self.queue_capacity) {
            return Err(ExecutorError::InvalidQueueCapacity);
        }
        if !(Duration::from_millis(1)..=Duration::from_secs(60)).contains(&self.busy_timeout) {
            return Err(ExecutorError::InvalidBusyTimeout);
        }
        Ok(())
    }
}

/// The application lifecycle owner. Dropping closes admission and detaches the
/// thread; normal application shutdown must call `shutdown` to drain and join.
pub(crate) struct Executor {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub(crate) struct Handle {
    shared: Arc<Shared>,
}

impl Executor {
    /// [`Self::open_with_effects`] without a runtime: committed effects are
    /// discarded. Only storage-level tests need this.
    #[cfg(test)]
    pub(crate) async fn open<F, E, G, T>(
        path: PathBuf,
        options: Options,
        lifetime_guard: G,
        initialize: F,
    ) -> Result<(Self, T), OpenError<E>>
    where
        F: FnOnce(&mut Connection, &G) -> Result<T, E> + Send + 'static,
        E: Send + 'static,
        G: Send + 'static,
        T: Send + 'static,
    {
        Self::open_with_effects(
            path,
            options,
            lifetime_guard,
            Weak::<crate::effect::NoEffects>::new(),
            initialize,
        )
        .await
    }

    /// Opens/configures SQLite and invokes `initialize` on its owning thread.
    /// Initialization failure closes the connection and joins the thread. It
    /// never deletes, resets, or retries a database; initialization atomicity is
    /// the schema owner's responsibility (normally a SQLite transaction).
    /// The initializer borrows the worker-owned lifetime guard and returns its
    /// startup value through the same bounded one-shot as startup success. An
    /// abandoned reply drops that value; the guard remains until worker exit.
    /// `effects` receives every committed transaction's effects on the worker
    /// thread for as long as its owner keeps it alive.
    pub(crate) async fn open_with_effects<F, E, G, T>(
        path: PathBuf,
        options: Options,
        lifetime_guard: G,
        effects: Weak<dyn EffectSink>,
        initialize: F,
    ) -> Result<(Self, T), OpenError<E>>
    where
        F: FnOnce(&mut Connection, &G) -> Result<T, E> + Send + 'static,
        E: Send + 'static,
        G: Send + 'static,
        T: Send + 'static,
    {
        options.validate()?;
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                accepting: true,
                jobs: VecDeque::new(),
            }),
            ready: Condvar::new(),
            capacity: Arc::new(Semaphore::new(options.queue_capacity)),
            effects,
        });
        let (initialized, initialization) = oneshot::channel();
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("meshway-sqlite".into())
            .spawn(move || {
                // Declared before all worker resources: released only after the
                // connection and abandoned jobs have been dropped, including
                // initialization cancellation, shutdown cancellation and panic.
                let _lifetime_guard = lifetime_guard;
                // Unwinding is terminal: close admission and drop queued replies.
                // Do not catch a panic and reuse a possibly inconsistent connection.
                let _exit = WorkerExit(Arc::clone(&worker_shared));
                let connection = open_connection(path, options).map_err(OpenError::Executor);
                let mut connection = match connection {
                    Ok(connection) => connection,
                    Err(error) => {
                        let _ = initialized.send(Err(error));
                        return;
                    }
                };
                let value = match initialize(&mut connection, &_lifetime_guard) {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = initialized.send(Err(OpenError::Initialization(error)));
                        return;
                    }
                };
                if let Err(error) = enable_wal(&connection) {
                    let _ = initialized.send(Err(OpenError::Executor(error)));
                    return;
                }
                let _ = initialized.send(Ok(value));
                while let Some(job) = worker_shared.next() {
                    // Free the waiting slot before executing; no queue lock is
                    // held while running SQL or delivering the one-shot reply.
                    drop(job.slot);
                    (job.operation)(&mut connection);
                }
            })
            .map_err(|_| ExecutorError::ThreadStart)?;
        let owner = Self {
            shared,
            worker: Some(worker),
        };
        match initialization.await {
            Ok(Ok(value)) => Ok((owner, value)),
            result => {
                // Also join on startup failure; cancellation drops the owner and
                // closes admission without blocking an async runtime thread.
                owner.shutdown().await?;
                Err(match result {
                    Ok(Err(error)) => error,
                    Err(_) => OpenError::Executor(ExecutorError::Closed),
                    Ok(Ok(_)) => unreachable!(),
                })
            }
        }
    }

    pub(crate) fn handle(&self) -> Handle {
        Handle {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Closes admission, drains accepted work, then joins off the async runtime.
    /// Once polled, cancelling this future does not cancel the blocking join.
    /// Operations must be finite: shutdown does not interrupt running SQL/code.
    pub(crate) async fn shutdown(mut self) -> Result<(), ExecutorError> {
        self.shared.close();
        if let Some(worker) = self.worker.take() {
            tokio::task::spawn_blocking(move || worker.join())
                .await
                .map_err(|_| ExecutorError::JoinFailed)?
                .map_err(|_| ExecutorError::WorkerPanicked)?;
        }
        Ok(())
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        self.shared.close();
        // Dropping std::thread::JoinHandle detaches rather than blocking.
    }
}

impl Handle {
    /// Returns a typed operation result, or `Closed` if it was not accepted or
    /// the worker exited without a reply. A closed reply does not prove rollback;
    /// use-case idempotency/recovery must resolve unknown commit outcomes.
    /// The executor does not inspect or format use-case results/errors.
    pub(crate) async fn execute<F, T>(&self, operation: F) -> Result<T, ExecutorError>
    where
        F: FnOnce(&mut Connection) -> T + Send + 'static,
        T: Send + 'static,
    {
        let slot = Arc::clone(&self.shared.capacity)
            .acquire_owned()
            .await
            .map_err(|_| ExecutorError::Closed)?;
        let (reply, response) = oneshot::channel();
        let job = Job {
            slot,
            operation: Box::new(move |connection| {
                let result = operation(connection);
                let _ = reply.send(result);
            }),
        };
        {
            let mut queue = self.shared.queue();
            if !queue.accepting {
                return Err(ExecutorError::Closed);
            }
            queue.jobs.push_back(job);
        }
        self.shared.ready.notify_one();
        response.await.map_err(|_| ExecutorError::Closed)
    }

    /// One transaction, one closure. The closure returns its result together
    /// with the effects the commit should have; they are applied in order on
    /// this thread only after COMMIT succeeded, so a rollback publishes nothing
    /// and a cancelled HTTP waiter cannot separate a commit from its effects.
    pub(crate) async fn transact<F, T>(
        &self,
        behavior: TransactionBehavior,
        operation: F,
    ) -> Result<T, crate::error::Error>
    where
        F: FnOnce(&Transaction<'_>) -> Result<(T, Vec<Effect>), crate::error::Error>
            + Send
            + 'static,
        T: Send + 'static,
    {
        let effects = Weak::clone(&self.shared.effects);
        self.execute(move |connection| {
            let tx = connection.transaction_with_behavior(behavior)?;
            let (value, pending) = operation(&tx)?;
            tx.commit()?;
            if let Some(sink) = effects.upgrade() {
                for effect in pending {
                    sink.apply(effect);
                }
            }
            Ok(value)
        })
        .await?
    }

    /// A deferred transaction that produces no effects: consistent reads.
    pub(crate) async fn read<F, T>(&self, operation: F) -> Result<T, crate::error::Error>
    where
        F: FnOnce(&Transaction<'_>) -> Result<T, crate::error::Error> + Send + 'static,
        T: Send + 'static,
    {
        self.transact(TransactionBehavior::Deferred, |tx| {
            operation(tx).map(|value| (value, Vec::new()))
        })
        .await
    }

    /// An immediate write transaction whose commit has no runtime effect.
    pub(crate) async fn write<F, T>(&self, operation: F) -> Result<T, crate::error::Error>
    where
        F: FnOnce(&Transaction<'_>) -> Result<T, crate::error::Error> + Send + 'static,
        T: Send + 'static,
    {
        self.transact(TransactionBehavior::Immediate, |tx| {
            operation(tx).map(|value| (value, Vec::new()))
        })
        .await
    }
}

fn open_connection(path: PathBuf, options: Options) -> Result<Connection, ExecutorError> {
    let connection = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    connection.busy_timeout(options.busy_timeout)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    let foreign_keys: bool =
        connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    let synchronous: i64 = connection.pragma_query_value(None, "synchronous", |row| row.get(0))?;
    if !foreign_keys || synchronous != 2 {
        return Err(ExecutorError::ConnectionConfiguration);
    }
    Ok(connection)
}

// Only change persistent journal mode after the schema owner accepts the file.
fn enable_wal(connection: &Connection) -> Result<(), ExecutorError> {
    let journal_mode: String =
        connection.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(ExecutorError::ConnectionConfiguration);
    }
    Ok(())
}

struct Job {
    operation: Box<dyn FnOnce(&mut Connection) + Send>,
    slot: OwnedSemaphorePermit,
}

struct Queue {
    accepting: bool,
    jobs: VecDeque<Job>,
}

struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
    capacity: Arc<Semaphore>,
    /// The application owns the runtime; the executor only delivers to it.
    /// After the owner dropped it, committed effects have no audience.
    effects: Weak<dyn EffectSink>,
}

impl Shared {
    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn close(&self) {
        self.queue().accepting = false;
        self.capacity.close();
        self.ready.notify_all();
    }

    fn next(&self) -> Option<Job> {
        let mut queue = self.queue();
        loop {
            if let Some(job) = queue.jobs.pop_front() {
                return Some(job);
            }
            if !queue.accepting {
                return None;
            }
            queue = self
                .ready
                .wait(queue)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

struct WorkerExit(Arc<Shared>);

impl Drop for WorkerExit {
    fn drop(&mut self) {
        self.0.close();
        let abandoned = std::mem::take(&mut self.0.queue().jobs);
        // Drop captured inputs and reply senders outside the queue lock. Each
        // completion has its own non-blocking one-shot; no completion queue can
        // backpressure worker exit, even if every caller has been cancelled.
        drop(abandoned);
    }
}

#[cfg(test)]
mod tests;
