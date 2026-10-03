//! Private filesystem resources whose lifetime is owned by the SQLite worker.

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub(crate) enum DirectoryError {
    #[error("data directory must be an absolute path without parent traversal")]
    InvalidPath,
    #[error("storage ownership, type, or private permissions are invalid")]
    UnsafeStorage,
    #[error("data directory is already in use")]
    InUse,
    #[error("storage filesystem operation failed")]
    Io,
    #[cfg(not(unix))]
    #[error("private storage is unsupported on this platform")]
    UnsupportedPlatform,
}

pub(crate) struct Directory {
    database_path: PathBuf,
    // Descriptors retain the checked resources, but do not bind SQLite's later
    // pathname lookup. Ancestor checks below protect that namespace against
    // other UIDs. The lock is never cloned/unlinked; its last close releases it.
    #[cfg(unix)]
    _directory: std::fs::File,
    #[cfg(unix)]
    _database: std::fs::File,
    #[cfg(unix)]
    _lock: std::fs::File,
}

impl Directory {
    pub(crate) fn open(path: &Path) -> Result<Self, DirectoryError> {
        #[cfg(unix)]
        {
            unix::open(path)
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(DirectoryError::UnsupportedPlatform)
        }
    }

    pub(crate) fn database_path(&self) -> PathBuf {
        self.database_path.clone()
    }

    pub(crate) fn read_key(
        &self,
        pending: bool,
    ) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>, DirectoryError> {
        #[cfg(unix)]
        {
            unix::read_key(&self._directory, pending)
        }
        #[cfg(not(unix))]
        {
            let _ = pending;
            Err(DirectoryError::UnsupportedPlatform)
        }
    }

    /// `None` publishes a checked, complete staging file from interrupted first
    /// initialization. Neither existing final nor staging files are overwritten.
    pub(crate) fn publish_key(&self, bytes: Option<&[u8]>) -> Result<(), DirectoryError> {
        #[cfg(unix)]
        {
            unix::publish_key(&self._directory, bytes)
        }
        #[cfg(not(unix))]
        {
            let _ = bytes;
            Err(DirectoryError::UnsupportedPlatform)
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::fs::{File, TryLockError};
    use std::io::{Read, Write};
    use std::os::unix::fs::MetadataExt;
    use std::path::{Component, Path};

    use rustix::fs::{Mode, OFlags, mkdirat, open as open_directory, openat};
    use rustix::io::Errno;

    use super::{Directory, DirectoryError};

    const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);
    const FILE_FLAGS: OFlags = OFlags::RDWR
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC)
        .union(OFlags::NONBLOCK);

    const KEY: &str = "master-key";
    const PENDING_KEY: &str = "master-key.pending";

    pub(super) fn read_key(
        directory: &File,
        pending: bool,
    ) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>, DirectoryError> {
        let name = if pending { PENDING_KEY } else { KEY };
        let fd = match openat(directory, name, FILE_FLAGS, Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return Ok(None),
            Err(error) => return Err(io_error(error)),
        };
        let file = File::from(fd);
        check(&file, false)?;
        // A previous first boot may have stopped after rename but before the
        // directory sync. A retry must establish durability before SQL binding.
        if !pending {
            file.sync_all().map_err(|_| DirectoryError::Io)?;
            directory.sync_all().map_err(|_| DirectoryError::Io)?;
        }
        // Fixed key format is 53 bytes. Bound reads even for hostile large files.
        // Preallocate the entire bound: growing a Vec could leave key bytes in
        // a deallocated allocation before the final buffer gets zeroized.
        let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(54));
        file.take(54)
            .read_to_end(&mut bytes)
            .map_err(|_| DirectoryError::Io)?;
        Ok(Some(bytes))
    }

    pub(super) fn publish_key(
        directory: &File,
        bytes: Option<&[u8]>,
    ) -> Result<(), DirectoryError> {
        let flags = if bytes.is_some() {
            FILE_FLAGS | OFlags::CREATE | OFlags::EXCL
        } else {
            FILE_FLAGS
        };
        let mut file = File::from(
            openat(directory, PENDING_KEY, flags, Mode::from_raw_mode(0o600)).map_err(io_error)?,
        );
        check(&file, false)?;
        if let Some(bytes) = bytes {
            file.write_all(bytes).map_err(|_| DirectoryError::Io)?;
        }
        file.sync_all().map_err(|_| DirectoryError::Io)?;
        rustix::fs::renameat_with(
            directory,
            PENDING_KEY,
            directory,
            KEY,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(io_error)?;
        directory.sync_all().map_err(|_| DirectoryError::Io)?;
        Ok(())
    }

    pub(super) fn open(path: &Path) -> Result<Directory, DirectoryError> {
        if !path.is_absolute() {
            return Err(DirectoryError::InvalidPath);
        }
        let mut names = Vec::new();
        for component in path.components() {
            match component {
                Component::RootDir | Component::CurDir => {}
                Component::Normal(name) => names.push(name),
                _ => return Err(DirectoryError::InvalidPath),
            }
        }
        let Some((name, parents)) = names.split_last() else {
            return Err(DirectoryError::InvalidPath);
        };
        let mut parent =
            File::from(open_directory("/", DIRECTORY_FLAGS, Mode::empty()).map_err(io_error)?);
        check_ancestor(&parent)?;
        for name in parents {
            parent = File::from(
                openat(&parent, *name, DIRECTORY_FLAGS, Mode::empty()).map_err(io_error)?,
            );
            check_ancestor(&parent)?;
        }
        match mkdirat(&parent, *name, Mode::from_raw_mode(0o700)) {
            Ok(()) => parent.sync_all().map_err(|_| DirectoryError::Io)?,
            Err(Errno::EXIST) => {}
            Err(error) => return Err(io_error(error)),
        }
        let directory =
            File::from(openat(&parent, *name, DIRECTORY_FLAGS, Mode::empty()).map_err(io_error)?);
        check(&directory, true)?;

        let lock = private_file(&directory, "controller.lock")?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(DirectoryError::InUse),
            Err(TryLockError::Error(_)) => return Err(DirectoryError::Io),
        }
        // SQLite opens these names itself. Reject hostile/stale objects before
        // SQLite can read them; the enclosing directory is owner-only.
        for name in [
            "controller.sqlite-wal",
            "controller.sqlite-shm",
            "controller.sqlite-journal",
        ] {
            match openat(&directory, name, FILE_FLAGS, Mode::empty()) {
                Ok(fd) => check(&File::from(fd), false)?,
                Err(Errno::NOENT) => {}
                Err(error) => return Err(io_error(error)),
            }
        }
        let database = private_file(&directory, "controller.sqlite")?;
        Ok(Directory {
            database_path: path.join("controller.sqlite"),
            _directory: directory,
            _database: database,
            _lock: lock,
        })
    }

    fn private_file(directory: &File, name: &str) -> Result<File, DirectoryError> {
        let fd = match openat(
            directory,
            name,
            FILE_FLAGS | OFlags::CREATE | OFlags::EXCL,
            Mode::from_raw_mode(0o600),
        ) {
            Ok(fd) => fd,
            Err(Errno::EXIST) => {
                openat(directory, name, FILE_FLAGS, Mode::empty()).map_err(io_error)?
            }
            Err(error) => return Err(io_error(error)),
        };
        let file = File::from(fd);
        check(&file, false)?;
        Ok(file)
    }

    fn check(file: &File, directory: bool) -> Result<(), DirectoryError> {
        let metadata = file.metadata().map_err(|_| DirectoryError::Io)?;
        let kind_ok = if directory {
            metadata.is_dir()
        } else {
            metadata.is_file() && metadata.nlink() == 1
        };
        let mode = if directory { 0o700 } else { 0o600 };
        if !kind_ok
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o7777 != mode
        {
            return Err(DirectoryError::UnsafeStorage);
        }
        Ok(())
    }

    fn check_ancestor(directory: &File) -> Result<(), DirectoryError> {
        let metadata = directory.metadata().map_err(|_| DirectoryError::Io)?;
        let owner = metadata.uid();
        let trusted_owner = owner == 0 || owner == rustix::process::geteuid().as_raw();
        let writable_by_others = metadata.mode() & 0o022 != 0;
        let sticky = metadata.mode() & 0o1000 != 0;
        // Root/euid-owned sticky temp directories are allowed: other UIDs may
        // create names but cannot rename our or root's children. Every next
        // child must pass its own ownership check before we descend or create.
        // This does not defend against root or an attacker with our own UID,
        // who already controls the process/private storage trust boundary.
        if !metadata.is_dir() || !trusted_owner || (writable_by_others && !sticky) {
            return Err(DirectoryError::UnsafeStorage);
        }
        Ok(())
    }

    fn io_error(error: Errno) -> DirectoryError {
        match error {
            Errno::LOOP | Errno::NOTDIR => DirectoryError::UnsafeStorage,
            _ => DirectoryError::Io,
        }
    }
}
