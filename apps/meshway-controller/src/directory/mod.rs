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
    // Open descriptors pin the checked resources. The lock descriptor is never
    // cloned or unlinked; its last close releases the process lock.
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
}

#[cfg(unix)]
mod unix {
    use std::fs::{File, TryLockError};
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
        for name in parents {
            parent = File::from(
                openat(&parent, *name, DIRECTORY_FLAGS, Mode::empty()).map_err(io_error)?,
            );
        }
        match mkdirat(&parent, *name, Mode::from_raw_mode(0o700)) {
            Ok(()) | Err(Errno::EXIST) => {}
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

    fn io_error(error: Errno) -> DirectoryError {
        match error {
            Errno::LOOP | Errno::NOTDIR => DirectoryError::UnsafeStorage,
            _ => DirectoryError::Io,
        }
    }
}
