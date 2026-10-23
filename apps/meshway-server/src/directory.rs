//! Server filenames and master-key publication, owned by the SQLite worker.
pub(crate) use meshway_private_storage::Error as DirectoryError;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub(crate) struct Directory {
    private: meshway_private_storage::Directory,
    // SQLite reopens its pathname. Keep the checked file and directory alive;
    // shared ancestor checks protect the namespace against other UIDs.
    _database: std::fs::File,
}

const DATABASE: &str = "controller.sqlite";
pub(crate) const LOCK: &str = "controller.lock";
/// Prefix of a migration's unpublished staging files (with their SQLite
/// journals); they are never read, only removed.
const STAGING: &str = "controller.sqlite.migrating.";
/// Database files a migration replaced, newest first, that are kept.
pub(crate) const MIGRATION_BACKUPS: usize = 3;

/// `controller.sqlite.v{version}-{unix ms}`: the database file of schema
/// `version` a migration replaced at that time.
pub(crate) fn backup_name(version: i64, at_ms: i64) -> String {
    format!("{DATABASE}.v{version}-{at_ms}")
}

fn backup_stamp(name: &str) -> Option<(i64, i64)> {
    let (version, stamp) = name
        .strip_prefix(DATABASE)?
        .strip_prefix(".v")?
        .split_once('-')?;
    let digits = |value: &str| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
    if !digits(version) || !digits(stamp) {
        return None;
    }
    Some((version.parse().ok()?, stamp.parse().ok()?))
}

impl Directory {
    pub(crate) fn open(path: &Path) -> Result<Self, DirectoryError> {
        let private = meshway_private_storage::Directory::open(path, LOCK)?;
        // A migration interrupted between naming the old database as a backup
        // and publishing the new one left the old file with a second name; the
        // database is still the old one, and the migration starts again.
        for name in private.names()? {
            if backup_stamp(&name).is_some() && private.remove_link(DATABASE, &name)? {
                tracing::info!(backup = %name, "Removed the backup name of an interrupted database migration");
            }
        }
        for name in [
            "controller.sqlite-wal",
            "controller.sqlite-shm",
            "controller.sqlite-journal",
        ] {
            private.file(name, false)?;
        }
        let database = private.file(DATABASE, true)?.ok_or(DirectoryError::Io)?;
        Ok(Self {
            private,
            _database: database,
        })
    }

    /// A new data directory at `path`; `None` when the name exists already
    /// (an existing directory is never adopted).
    pub(crate) fn create_new(path: &Path) -> Result<Option<Self>, DirectoryError> {
        let Some(private) = meshway_private_storage::Directory::create_new(path, LOCK)? else {
            return Ok(None);
        };
        let database = private.file(DATABASE, true)?.ok_or(DirectoryError::Io)?;
        Ok(Some(Self {
            private,
            _database: database,
        }))
    }

    /// Removes the files a new data directory gets from a backup (database,
    /// its journals and the master key), and nothing else.
    pub(crate) fn remove_written(&self) -> Result<(), DirectoryError> {
        for name in [
            DATABASE,
            "controller.sqlite-journal",
            "controller.sqlite-wal",
            "controller.sqlite-shm",
            "master-key",
            "master-key.pending",
        ] {
            self.private.remove(name)?;
        }
        Ok(())
    }

    pub(crate) fn database_path(&self) -> PathBuf {
        self.private.path().join(DATABASE)
    }

    pub(crate) fn database_len(&self) -> Result<u64, DirectoryError> {
        Ok(self
            ._database
            .metadata()
            .map_err(|_| DirectoryError::Io)?
            .len())
    }

    /// Makes the database file and its name durable.
    pub(crate) fn sync_database(&self) -> Result<(), DirectoryError> {
        self._database.sync_all().map_err(|_| DirectoryError::Io)?;
        self.private.sync()
    }

    pub(crate) fn available_bytes(&self) -> Result<u64, DirectoryError> {
        self.private.available_bytes()
    }

    /// Removes every unpublished migration staging file.
    pub(crate) fn discard_staging(&self) -> Result<(), DirectoryError> {
        for name in self.private.names()? {
            if name.starts_with(STAGING) {
                self.private.remove(&name)?;
            }
        }
        Ok(())
    }

    /// A new, empty owner-only staging file for the database of `version`.
    pub(crate) fn create_staging(&self, version: i64) -> Result<(String, PathBuf), DirectoryError> {
        let name = format!("{STAGING}v{version}");
        self.private.remove(&name)?;
        drop(self.private.file(&name, true)?.ok_or(DirectoryError::Io)?);
        Ok((name.clone(), self.private.path().join(name)))
    }

    pub(crate) fn remove_staging(&self, name: &str) -> Result<(), DirectoryError> {
        debug_assert!(name.starts_with(STAGING));
        self.private.remove(name)
    }

    /// A quiesced database has no journal: no WAL that SQLite could replay
    /// onto another file under the database name. A stale shared-memory index
    /// is removed.
    pub(crate) fn ensure_no_journal(&self) -> Result<(), DirectoryError> {
        for name in ["controller.sqlite-wal", "controller.sqlite-journal"] {
            if self.private.file(name, false)?.is_some() {
                return Err(DirectoryError::Io);
            }
        }
        self.private.remove("controller.sqlite-shm")
    }

    /// Gives the current database file the second name `backup`; it keeps
    /// the old file once [`Self::replace_database`] published the new one.
    /// Interrupted in between, [`Self::open`] removes the second name.
    pub(crate) fn link_backup(&self, backup: &str) -> Result<(), DirectoryError> {
        debug_assert!(backup_stamp(backup).is_some());
        self.private.link(DATABASE, backup)
    }

    /// Atomically replaces the database by the complete staging file
    /// `staging`: the name is always the complete old or the complete new
    /// file. Afterwards only the newest [`MIGRATION_BACKUPS`] backups stay.
    pub(crate) fn replace_database(&mut self, staging: &str) -> Result<(), DirectoryError> {
        debug_assert!(staging.starts_with(STAGING));
        self.private.rename_over(staging, DATABASE)?;
        self._database = self
            .private
            .file(DATABASE, false)?
            .ok_or(DirectoryError::Io)?;
        let mut backups: Vec<_> = self
            .private
            .names()?
            .into_iter()
            .filter_map(|name| backup_stamp(&name).map(|(_, at)| (at, name)))
            .collect();
        backups.sort();
        let excess = backups.len().saturating_sub(MIGRATION_BACKUPS);
        for (_, name) in &backups[..excess] {
            if let Err(error) = self.private.remove(name) {
                tracing::warn!(backup = %name, %error, "Old database backup not removed");
            }
        }
        Ok(())
    }

    pub(crate) fn path(&self) -> &Path {
        self.private.path()
    }

    pub(crate) fn read_key(
        &self,
        pending: bool,
    ) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>, DirectoryError> {
        let name = if pending {
            "master-key.pending"
        } else {
            "master-key"
        };
        let Some(file) = self.private.file(name, false)? else {
            return Ok(None);
        };
        if !pending {
            file.sync_all().map_err(|_| DirectoryError::Io)?;
            self.private.sync()?;
        }
        // Fixed format is 53 bytes; retain an extra byte to detect oversize.
        let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(54));
        file.take(54)
            .read_to_end(&mut bytes)
            .map_err(|_| DirectoryError::Io)?;
        Ok(Some(bytes))
    }

    pub(crate) fn publish_key(&self, bytes: Option<&[u8]>) -> Result<(), DirectoryError> {
        self.private
            .publish_once("master-key", "master-key.pending", bytes)
    }
}
