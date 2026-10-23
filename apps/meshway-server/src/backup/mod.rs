//! The offline backup: a consistent copy of the database together with the
//! master key, in a new private directory that is itself a data directory.
//! The database and its key are every secret of the installation, the netmap
//! signing keys included; a copy of one without the other is not a backup.
//!
//! It holds the source's storage lock throughout (the serving process must
//! stop first) and never writes the source. The copy is proved like a normal
//! opening proves a database: the current schema in full and the copied key
//! bound to it. A failure removes everything it wrote.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::app::AppError;
use crate::directory::Directory;
use crate::secrets;
use crate::settings::Migration;
use crate::store::schema;

/// A completed backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    pub output: PathBuf,
    /// Schema version of the copied database.
    pub schema: i64,
    pub database_bytes: u64,
}

/// Copies the database and master key of `data_dir` into the new directory
/// `output` (0700; its files 0600). A database of an earlier schema version
/// is refused like by every other offline command.
pub async fn backup(data_dir: PathBuf, output: PathBuf) -> Result<Backup, AppError> {
    tokio::task::spawn_blocking(move || run(&data_dir, &output))
        .await
        .map_err(|_| AppError::Worker)?
}

fn run(data_dir: &Path, output: &Path) -> Result<Backup, AppError> {
    let mut source = Directory::open(data_dir)?;
    schema::migrate::prepare(&mut source, Migration::Refuse)?;
    let mut database = open(&source.database_path()).map_err(|_| AppError::Database)?;
    database
        .pragma_update(None, "query_only", true)
        .map_err(|_| AppError::Database)?;
    if schema::identity(&database)? != (schema::APPLICATION_ID, schema::VERSION) {
        tracing::error!("The data directory holds no database to back up");
        return Err(AppError::UnsupportedSchema);
    }
    schema::check_with(&mut database, |connection| {
        secrets::initialize(connection, &source, false).map_err(AppError::from)
    })?;
    let key = source.read_key(false)?.ok_or(AppError::MasterKey)?;

    // Created here or refused: a directory that appears under the name in
    // the meantime is never adopted, so a failure removes only this run's files.
    let target = match Directory::create_new(output) {
        Ok(Some(target)) => target,
        Ok(None) => {
            tracing::error!("Backup output already exists");
            return Err(AppError::BackupOutput);
        }
        Err(error) => {
            tracing::error!(%error, "Backup output directory not created");
            return Err(AppError::BackupOutput);
        }
    };
    let result = (|| {
        let needed = source.database_len()?.saturating_mul(11) / 10;
        let available = target
            .available_bytes()
            .map_err(|_| AppError::BackupOutput)?;
        if available < needed {
            tracing::error!(needed, available, "Not enough free space for the backup");
            return Err(AppError::BackupOutput);
        }
        let path = target.database_path();
        let path = path.to_str().ok_or(AppError::BackupOutput)?;
        // `query_only` also refuses VACUUM INTO, which writes the copy only.
        database
            .pragma_update(None, "query_only", false)
            .map_err(|_| AppError::Database)?;
        database
            .execute("VACUUM INTO ?1", [path])
            .map_err(|error| match error.sqlite_error_code() {
                Some(rusqlite::ErrorCode::DiskFull) => AppError::BackupOutput,
                _ => AppError::Database,
            })?;
        target
            .publish_key(Some(&key))
            .map_err(|_| AppError::BackupOutput)?;
        #[cfg(test)]
        tests::after_copy(output)?;
        let mut copy = open(&target.database_path()).map_err(|_| AppError::Database)?;
        copy.pragma_update(None, "query_only", true)
            .map_err(|_| AppError::Database)?;
        let schema_version = schema::identity(&copy)?.1;
        schema::check_with(&mut copy, |connection| {
            secrets::initialize(connection, &target, false).map_err(AppError::from)
        })
        .inspect_err(|error| tracing::error!(%error, "The backup copy failed its check"))?;
        copy.close().map_err(|_| AppError::Database)?;
        target.sync_database().map_err(|_| AppError::BackupOutput)?;
        Ok(Backup {
            output: output.to_owned(),
            schema: schema_version,
            database_bytes: target.database_len().map_err(|_| AppError::BackupOutput)?,
        })
    })();
    if result.is_err() {
        remove(target, output);
    }
    result
}

/// Removes a backup that did not complete: a partial copy of secrets is not
/// left behind. Only the files this run writes and the directory it created
/// go; anything else found there keeps the directory (and is reported).
fn remove(target: Directory, output: &Path) {
    let removed = target.remove_written();
    drop(target);
    let removed = removed.and_then(|()| {
        std::fs::remove_file(output.join(crate::directory::LOCK))
            .and_then(|()| std::fs::remove_dir(output))
            .map_err(|_| crate::directory::DirectoryError::Io)
    });
    if let Err(error) = removed {
        tracing::error!(%error, output = %output.display(), "Incomplete backup not removed; delete it");
    }
}

fn open(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

#[cfg(test)]
mod tests;
