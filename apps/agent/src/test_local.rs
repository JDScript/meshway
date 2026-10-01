//! Exact production write implementation with unit-test-only filesystem faults.

pub use meshway_protocol::local::{
    credential, digest, lock, open_private, private_dir, read_private, read_private_bounded,
    valid_credential,
};

#[path = "../../../crates/protocol/src/local/write.rs"]
mod write;
pub(crate) use write::test_faults;
pub use write::{AtomicWriteError, ensure_durable};

pub fn atomic_write(
    path: &std::path::Path,
    value: &[u8],
    replace: bool,
) -> Result<(), AtomicWriteError> {
    let _unavailable_parent = precommit_faults::make_parent_unavailable(path)?;
    write::atomic_write(path, value, replace)
}

/// Make the production primitive encounter an actual ENOTDIR before it can
/// create a temporary file. This runs at the write boundary, after identity/key
/// durability checks, and works even in root CI where chmod cannot force EACCES.
/// The original test directory is restored before atomic_write returns or unwinds.
pub(crate) mod precommit_faults {
    use std::{
        cell::RefCell,
        fs, io,
        path::{Path, PathBuf},
    };

    std::thread_local! {
        static TARGET: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    pub struct ParentUnavailable;

    impl ParentUnavailable {
        pub fn during_write(path: &Path) -> Self {
            TARGET.with(|target| {
                assert!(
                    target.borrow().is_none(),
                    "overlapping precommit fault injection"
                );
                *target.borrow_mut() = Some(path.to_owned());
            });
            Self
        }
    }

    impl Drop for ParentUnavailable {
        fn drop(&mut self) {
            TARGET.with(|target| *target.borrow_mut() = None);
        }
    }

    pub(super) struct RestoreParent {
        original: PathBuf,
        saved: PathBuf,
        _staging: tempfile::TempDir,
    }

    impl Drop for RestoreParent {
        fn drop(&mut self) {
            match fs::remove_file(&self.original) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => panic!("cannot remove test parent placeholder: {error}"),
            }
            fs::rename(&self.saved, &self.original).expect("restore agent test directory");
        }
    }

    pub(super) fn make_parent_unavailable(path: &Path) -> io::Result<Option<RestoreParent>> {
        if !TARGET.with(|target| target.borrow().as_deref() == Some(path)) {
            return Ok(None);
        }
        let original = path.parent().expect("test state has a parent");
        let staging = tempfile::Builder::new()
            .prefix("agent-precommit-")
            .tempdir_in(original.parent().expect("test state has a grandparent"))?;
        let saved = staging.path().join("saved");
        fs::rename(original, &saved)?;
        let restore = RestoreParent {
            original: original.to_owned(),
            saved,
            _staging: staging,
        };
        fs::File::create_new(original)?;
        Ok(Some(restore))
    }
}
