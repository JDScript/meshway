//! Commit-aware private file publication, shared by identities and config caches.

use std::{
    fs::File,
    io::{self, Write},
    path::Path,
};

use super::open_private;

/// Publication is the commit point, distinct from confirmed durability. A
/// post-publication error cannot safely promise that the old file survived.
#[derive(Debug)]
pub enum AtomicWriteError {
    NotCommitted(io::Error),
    CommittedNotDurable(io::Error),
}

impl AtomicWriteError {
    pub fn committed(&self) -> bool {
        matches!(self, Self::CommittedNotDurable(_))
    }
}

impl From<io::Error> for AtomicWriteError {
    fn from(error: io::Error) -> Self {
        Self::NotCommitted(error)
    }
}

impl std::fmt::Display for AtomicWriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCommitted(error) => {
                write!(formatter, "private file write did not commit: {error}")
            }
            Self::CommittedNotDurable(error) => write!(
                formatter,
                "private file was published but durability is unconfirmed: {error}"
            ),
        }
    }
}

impl std::error::Error for AtomicWriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotCommitted(error) | Self::CommittedNotDurable(error) => Some(error),
        }
    }
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Reconfirm the visible private file and its directory entry are durable. This
/// is required before relying on a write that may have failed after publication,
/// including after process restart. The caller holds the same sidecar lock.
pub fn ensure_durable(path: &Path) -> io::Result<()> {
    let directory = File::open(parent(path))?;
    open_private(path, false)?.sync_all()?;
    sync_directory(&directory, path, false)
}

/// Write/fsync a private temporary file, atomically publish it, then fsync the
/// already-open directory. The caller holds a sidecar lock and validates any
/// existing state. On CommittedNotDurable, retain/reconcile the published value
/// and stop: never roll back memory alone, ACK, or claim persistence succeeded.
pub fn atomic_write(path: &Path, value: &[u8], replace: bool) -> Result<(), AtomicWriteError> {
    // Opening after rename can fail even when creation/rename was permitted
    // (e.g. a 0300 directory). Fail that case before changing the destination.
    let directory = File::open(parent(path))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent(path))?;
    temporary.write_all(value)?;
    temporary.as_file().sync_all()?;
    if replace {
        open_private(path, false)?;
        temporary.persist(path).map_err(|error| error.error)?;
    } else {
        temporary
            .persist_noclobber(path)
            .map_err(|error| error.error)?;
    }
    sync_directory(&directory, path, true).map_err(AtomicWriteError::CommittedNotDurable)
}

fn sync_directory(directory: &File, _path: &Path, _published: bool) -> io::Result<()> {
    #[cfg(test)]
    test_faults::before_directory_sync(_path, _published)?;
    directory.sync_all()
}

// Compiled only into unit-test binaries. Agent unit tests include this exact
// source module so fault injection exercises the real publication/sync boundary
// without a production feature, environment switch, or alternate write routine.
#[cfg(test)]
pub(crate) mod test_faults {
    use std::{
        cell::RefCell,
        io,
        path::{Path, PathBuf},
    };

    std::thread_local! {
        static FAILURE: RefCell<Option<(PathBuf, bool)>> = const { RefCell::new(None) };
    }

    pub struct DirectorySyncFailure;

    impl DirectorySyncFailure {
        pub fn after_publication(path: &Path) -> Self {
            FAILURE.with(|failure| {
                assert!(
                    failure.borrow().is_none(),
                    "overlapping durability fault injection"
                );
                *failure.borrow_mut() = Some((path.to_owned(), false));
            });
            Self
        }
    }

    impl Drop for DirectorySyncFailure {
        fn drop(&mut self) {
            FAILURE.with(|failure| *failure.borrow_mut() = None);
        }
    }

    pub(super) fn before_directory_sync(path: &Path, published: bool) -> io::Result<()> {
        let fail = FAILURE.with(|failure| {
            let mut failure = failure.borrow_mut();
            let Some((target, active)) = failure.as_mut() else {
                return false;
            };
            if target != path {
                return false;
            }
            *active |= published;
            *active
        });
        if fail {
            return Err(io::Error::other("injected directory fsync failure"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_after_publication_is_typed_and_requires_durability_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("private.json");
        atomic_write(&path, b"old", false).unwrap();
        let fault = test_faults::DirectorySyncFailure::after_publication(&path);
        let result = atomic_write(&path, b"new", true);
        assert!(matches!(
            result,
            Err(AtomicWriteError::CommittedNotDurable(_))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert!(ensure_durable(&path).is_err());
        drop(fault);
        ensure_durable(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        let result = atomic_write(&path, b"must not replace", false);
        assert!(matches!(result, Err(AtomicWriteError::NotCommitted(_))));
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }
}
