//! Checked private filesystem resources. Callers own formats and lifecycle.
//! Filesystem operations are synchronous; accepted writes can be joined by async owners.
#![forbid(unsafe_code)]

mod writes;
pub use writes::Writes;

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

    /// Durably move `source` to a new `<source>.stale-<nanoseconds>` archive,
    /// then keep only the newest `keep` archives of `source`: older ones are
    /// removed, so repeated archival (one per boot) stays bounded. Returns the
    /// new archive's name.
    pub fn archive_stale(&self, source: &str, keep: usize) -> Result<String, Error> {
        name(source)?;
        if source == self.lock_name || keep == 0 {
            return Err(Error::InvalidPath);
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::Io)?
            .as_nanos();
        let archive = format!("{source}{STALE}{stamp}");
        self.publish_once(&archive, source, None)?;
        #[cfg(unix)]
        {
            unix::prune_stale(&self.directory, source, keep)?;
        }
        Ok(archive)
    }

    /// Durably remove a checked owner-only regular file; an absent name is
    /// already removed.
    pub fn remove(&self, file_name: &str) -> Result<(), Error> {
        name(file_name)?;
        if file_name == self.lock_name {
            return Err(Error::InvalidPath);
        }
        #[cfg(unix)]
        {
            unix::remove(&self.directory, file_name)
        }
        #[cfg(not(unix))]
        {
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

/// Immutable content: owner read-only (0400) regular files published by
/// atomic rename and never written in place, so a reader may map them.
impl Directory {
    /// Durably publish `bytes` as an owner read-only file. An existing file
    /// of that name is replaced by rename; readers keep their old inode.
    pub fn publish_read_only(&self, final_name: &str, bytes: &[u8]) -> Result<(), Error> {
        name(final_name)?;
        if final_name == self.lock_name {
            return Err(Error::InvalidPath);
        }
        #[cfg(unix)]
        {
            unix::publish_read_only(&self.directory, final_name, bytes)
        }
        #[cfg(not(unix))]
        {
            let _ = bytes;
            Err(Error::UnsupportedPlatform)
        }
    }

    /// Open a checked owner read-only file; `None` when absent.
    pub fn read_only_file(&self, file_name: &str) -> Result<Option<File>, Error> {
        name(file_name)?;
        #[cfg(unix)]
        {
            unix::read_only_file(&self.directory, file_name)
        }
        #[cfg(not(unix))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }

    /// Durably remove an abandoned staging file of this directory's own
    /// writes (see [`is_staging`]) in either state a crash can leave it in:
    /// a checked owner file still 0600 while being written, or already 0400
    /// before its read-only publication was renamed. Absent is removed;
    /// anything else is refused.
    pub fn remove_staging(&self, file_name: &str) -> Result<(), Error> {
        if !is_staging(file_name) {
            return Err(Error::InvalidPath);
        }
        #[cfg(unix)]
        {
            let present = match unix::read_only_file(&self.directory, file_name) {
                Ok(found) => found.is_some(),
                Err(Error::UnsafeStorage) => {
                    unix::file(&self.directory, file_name, false)?.is_some()
                }
                Err(error) => return Err(error),
            };
            if !present {
                return Ok(());
            }
            unix::unlink(&self.directory, file_name)
        }
        #[cfg(not(unix))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }

    /// Durably remove a checked owner read-only file; absent is removed.
    pub fn remove_read_only(&self, file_name: &str) -> Result<(), Error> {
        name(file_name)?;
        #[cfg(unix)]
        {
            if unix::read_only_file(&self.directory, file_name)?.is_none() {
                return Ok(());
            }
            unix::unlink(&self.directory, file_name)
        }
        #[cfg(not(unix))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }

    /// Durably give the checked owner-only file `existing` a second name
    /// `new_name` (a hard link; `new_name` must not exist). Until one of the
    /// names is removed or renamed over, neither is a checked [`Self::file`].
    pub fn link(&self, existing: &str, new_name: &str) -> Result<(), Error> {
        name(existing)?;
        name(new_name)?;
        if existing == new_name || existing == self.lock_name || new_name == self.lock_name {
            return Err(Error::InvalidPath);
        }
        #[cfg(unix)]
        {
            unix::link(&self.directory, existing, new_name)
        }
        #[cfg(not(unix))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }

    /// Durably remove `alias` when it is the only other name of `name`'s
    /// owner-only file `file_name` (what an interrupted [`Self::link`]
    /// publication left);
    /// `false` when `alias` is absent or a separate file, which is kept.
    pub fn remove_link(&self, file_name: &str, alias: &str) -> Result<bool, Error> {
        name(file_name)?;
        name(alias)?;
        if file_name == alias || file_name == self.lock_name || alias == self.lock_name {
            return Err(Error::InvalidPath);
        }
        #[cfg(unix)]
        {
            unix::remove_link(&self.directory, file_name, alias)
        }
        #[cfg(not(unix))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }

    /// Durably rename the complete checked owner-only file `source` over
    /// `final_name` (which, when present, must be a checked file too).
    pub fn rename_over(&self, source: &str, final_name: &str) -> Result<(), Error> {
        name(source)?;
        name(final_name)?;
        if source == final_name || source == self.lock_name || final_name == self.lock_name {
            return Err(Error::InvalidPath);
        }
        #[cfg(unix)]
        {
            unix::rename_over(&self.directory, source, final_name)
        }
        #[cfg(not(unix))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }

    /// Bytes an unprivileged writer can still allocate on this directory's
    /// filesystem.
    pub fn available_bytes(&self) -> Result<u64, Error> {
        #[cfg(unix)]
        {
            let stat = rustix::fs::fstatvfs(&self.directory).map_err(|_| Error::Io)?;
            Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
        }
        #[cfg(not(unix))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }

    /// Names of the entries in this directory (bounded), the lock excluded.
    pub fn names(&self) -> Result<Vec<String>, Error> {
        #[cfg(unix)]
        {
            let mut names = unix::names(&self.directory)?;
            names.retain(|n| *n != self.lock_name);
            Ok(names)
        }
        #[cfg(not(unix))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }
}

const STALE: &str = ".stale-";
/// Archives of one record kept by [`Directory::archive_stale`] callers that
/// archive a record of another boot or network namespace: one per such
/// change, the newest three.
pub const STALE_ARCHIVES: usize = 3;
const STAGING: &str = ".write-";

/// The Unix nanosecond stamp of an archive that [`Directory::archive_stale`]
/// made of `source`, or `None` when `name` is not one.
pub fn stale_stamp(name: &str, source: &str) -> Option<u128> {
    let stamp = name.strip_prefix(source)?.strip_prefix(STALE)?;
    if stamp.is_empty() || !stamp.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    stamp.parse().ok()
}

/// Whether `name` is the unpublished staging file of an interrupted
/// [`Directory::replace`]. It is never read; the published name is authoritative.
pub fn is_staging(name: &str) -> bool {
    name.strip_prefix(STAGING)
        .is_some_and(|nonce| nonce.len() == 32 && nonce.bytes().all(|b| b.is_ascii_hexdigit()))
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
