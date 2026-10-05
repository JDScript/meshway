//! Checked private filesystem resources. Callers own formats and lifecycle.
//! Operations are synchronous; asynchronous owners run them on blocking workers.
#![forbid(unsafe_code)]

use std::{
    fs::File,
    path::{Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("storage path must be absolute without parent traversal")]
    InvalidPath,
    #[error("storage ownership, type or private permissions are invalid")]
    UnsafeStorage,
    #[error("private directory is already in use")]
    InUse,
    #[error("private filesystem operation failed")]
    Io,
    #[cfg(not(unix))]
    #[error("private storage is unsupported on this platform")]
    UnsupportedPlatform,
}

/// Lock and checked directory remain alive through the owner's last reference.
/// Files are opened relative to this descriptor, never by concatenating paths.
pub struct Directory {
    path: PathBuf,
    lock_name: String,
    #[cfg(unix)]
    directory: File,
    #[cfg(unix)]
    _lock: File,
}

impl Directory {
    pub fn open(path: &Path, lock_name: &str) -> Result<Self, Error> {
        name(lock_name)?;
        #[cfg(unix)]
        {
            unix::open(path, lock_name)
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(Error::UnsupportedPlatform)
        }
    }

    /// Inspect an existing private owner directory without creating any path or
    /// lock file. Missing directories return None; an existing directory whose
    /// lock is missing or invalid remains an ownership error.
    pub fn open_existing(path: &Path, lock_name: &str) -> Result<Option<Self>, Error> {
        name(lock_name)?;
        #[cfg(unix)]
        {
            unix::open_existing(path, lock_name)
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(Error::UnsupportedPlatform)
        }
    }

    /// For libraries such as SQLite which must reopen checked names themselves.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Create an owner-only regular file, or open an existing checked file.
    pub fn file(&self, file_name: &str, create: bool) -> Result<Option<File>, Error> {
        name(file_name)?;
        #[cfg(unix)]
        {
            unix::file(&self.directory, file_name, create)
        }
        #[cfg(not(unix))]
        {
            let _ = create;
            Err(Error::UnsupportedPlatform)
        }
    }

    pub fn sync(&self) -> Result<(), Error> {
        #[cfg(unix)]
        {
            self.directory.sync_all().map_err(|_| Error::Io)
        }
        #[cfg(not(unix))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }

    /// Publish a new file exactly once. `None` publishes a checked staging file
    /// whose format/completeness the caller has already verified after a crash.
    pub fn publish_once(
        &self,
        final_name: &str,
        staging: &str,
        bytes: Option<&[u8]>,
    ) -> Result<(), Error> {
        name(final_name)?;
        name(staging)?;
        if final_name == staging || final_name == self.lock_name || staging == self.lock_name {
            return Err(Error::InvalidPath);
        }
        #[cfg(unix)]
        {
            unix::publish_once(&self.directory, final_name, staging, bytes)
        }
        #[cfg(not(unix))]
        {
            let _ = bytes;
            Err(Error::UnsupportedPlatform)
        }
    }

    /// Durable atomic replacement using a unique private temporary file.
    /// An error after rename has an unknown durability outcome: reload before
    /// issuing dependent external operations. Existing corrupt files are not read
    /// or reset here; the caller must validate its current record before updates.
    pub fn replace(&self, final_name: &str, bytes: &[u8]) -> Result<(), Error> {
        name(final_name)?;
        if final_name == self.lock_name {
            return Err(Error::InvalidPath);
        }
        #[cfg(unix)]
        {
            unix::replace(&self.directory, final_name, bytes)
        }
        #[cfg(not(unix))]
        {
            let _ = bytes;
            Err(Error::UnsupportedPlatform)
        }
    }
}

fn name(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value.len() > 128
        || matches!(value, "." | "..")
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Err(Error::InvalidPath);
    }
    Ok(())
}

#[cfg(unix)]
mod unix;
