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

impl Directory {
    pub(crate) fn open(path: &Path) -> Result<Self, DirectoryError> {
        let private = meshway_private_storage::Directory::open(path, "controller.lock")?;
        for name in [
            "controller.sqlite-wal",
            "controller.sqlite-shm",
            "controller.sqlite-journal",
        ] {
            private.file(name, false)?;
        }
        let database = private
            .file("controller.sqlite", true)?
            .ok_or(DirectoryError::Io)?;
        Ok(Self {
            private,
            _database: database,
        })
    }

    pub(crate) fn database_path(&self) -> PathBuf {
        self.private.path().join("controller.sqlite")
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
