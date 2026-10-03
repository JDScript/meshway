use crate::directory::{Directory, DirectoryError};
use crate::settings::Settings;
use crate::store::{Executor, ExecutorError, OpenError, Options, schema};

/// Application lifecycle owner. Dropping closes work admission immediately;
/// the SQLite worker retains its process lock until accepted work has drained
/// and the connection has closed. Use [`Self::shutdown`] to await that boundary.
pub struct App {
    executor: Executor,
}

/// Safe diagnostic categories: no filesystem paths, SQL, or credentials.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum AppError {
    #[error("invalid database budget or data directory setting")]
    InvalidSettings,
    #[error("storage ownership, type, or private permissions are invalid")]
    UnsafeStorage,
    #[error("data directory is already in use")]
    AlreadyRunning,
    #[error("storage filesystem operation failed")]
    Filesystem,
    #[error("database application or schema version is unsupported")]
    UnsupportedSchema,
    #[error("database structure does not match the current schema")]
    SchemaStructure,
    #[error("database integrity check failed")]
    StorageIntegrity,
    #[error("database operation failed")]
    Database,
    #[error("database worker failed or stopped")]
    Worker,
    #[error("private storage is unsupported on this platform")]
    UnsupportedPlatform,
}

impl App {
    /// Opens a private, exclusively locked account-storage database.
    /// Existing unknown or damaged databases are rejected, never reset.
    pub async fn open(settings: Settings) -> Result<Self, AppError> {
        let options = Options {
            queue_capacity: settings.database_queue_capacity,
            busy_timeout: settings.database_busy_timeout,
        };
        options.validate().map_err(AppError::from)?;
        // No await between acquiring the lock and transferring it to the
        // executor's worker. Cancellation therefore cannot orphan its owner.
        let directory = Directory::open(&settings.data_dir)?;
        let executor = Executor::open(
            directory.database_path(),
            options,
            directory,
            schema::initialize,
        )
        .await
        .map_err(|error| match error {
            OpenError::Executor(error) => AppError::from(error),
            OpenError::Initialization(error) => AppError::from(error),
        })?;
        Ok(Self { executor })
    }

    /// Runs full, read-only schema, SQLite integrity, and foreign-key checks on
    /// the owning thread against one consistent database snapshot.
    pub async fn check_storage(&self) -> Result<(), AppError> {
        self.executor.handle().execute(schema::check).await??;
        Ok(())
    }

    /// Stops admission, drains accepted operations, and waits for SQLite close.
    /// Cancelling this future still leaves the process lock with the worker.
    pub async fn shutdown(self) -> Result<(), AppError> {
        self.executor.shutdown().await?;
        Ok(())
    }
}

impl From<DirectoryError> for AppError {
    fn from(error: DirectoryError) -> Self {
        match error {
            DirectoryError::InvalidPath => Self::InvalidSettings,
            DirectoryError::UnsafeStorage => Self::UnsafeStorage,
            DirectoryError::InUse => Self::AlreadyRunning,
            DirectoryError::Io => Self::Filesystem,
            #[cfg(not(unix))]
            DirectoryError::UnsupportedPlatform => Self::UnsupportedPlatform,
        }
    }
}

impl From<ExecutorError> for AppError {
    fn from(error: ExecutorError) -> Self {
        match error {
            ExecutorError::InvalidQueueCapacity | ExecutorError::InvalidBusyTimeout => {
                Self::InvalidSettings
            }
            ExecutorError::Sqlite { .. } | ExecutorError::ConnectionConfiguration => Self::Database,
            ExecutorError::ThreadStart
            | ExecutorError::Closed
            | ExecutorError::WorkerPanicked
            | ExecutorError::JoinFailed => Self::Worker,
        }
    }
}

impl From<schema::SchemaError> for AppError {
    fn from(error: schema::SchemaError) -> Self {
        match error {
            schema::SchemaError::Unsupported => Self::UnsupportedSchema,
            schema::SchemaError::Structure => Self::SchemaStructure,
            schema::SchemaError::Integrity => Self::StorageIntegrity,
            schema::SchemaError::Sqlite => Self::Database,
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
