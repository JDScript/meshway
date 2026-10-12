//! Read-only memory-mapped artifacts. This is the crate's only `unsafe`
//! (approved exception, see the design's "mmap 与 unsafe").
//!
//! Mapping a file is undefined behaviour if the file is truncated or written
//! while mapped. Callers therefore only map their own content-addressed cache
//! files: written once to a private directory, made read-only, published by
//! atomic rename and never rewritten in place (replacement is a new rename,
//! which leaves existing mappings on the old inode). [`open`] enforces what it
//! can check: a regular file within the size limit whose permission bits
//! grant no write access. The mapping is then verified exactly like a heap
//! buffer, including the digest.
#![allow(unsafe_code)]

use crate::{LoadError, MAX_ARTIFACT_BYTES, RuleSet, set::Backing};
use std::{fs::File, io, path::Path};

#[derive(Debug, thiserror::Error)]
pub enum MapError {
    #[error("Cannot map the rule-set artifact: {0}")]
    Io(#[from] io::Error),
    #[error("The rule-set artifact is not a regular read-only file.")]
    Writable,
    #[error(transparent)]
    Load(#[from] LoadError),
}

pub fn open(path: &Path) -> Result<RuleSet, MapError> {
    open_file(File::open(path)?)
}

/// Same as [`open`] for a file the caller already opened through its own
/// checked directory handle.
pub fn open_file(file: File) -> Result<RuleSet, MapError> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || !read_only(&metadata) {
        return Err(MapError::Writable);
    }
    if metadata.len() > MAX_ARTIFACT_BYTES as u64 {
        return Err(LoadError::TooLarge.into());
    }
    // SAFETY: `file` is a regular file without write permission bits, and the
    // caller contract above (private, content-addressed, replaced only by
    // rename) means no process truncates or writes it while mapped. The map
    // is read-only (`Mmap`, PROT_READ), so this process cannot write it
    // either. Under that contract the mapped bytes are immutable for the
    // lifetime of the `Mmap`, which `Backing::Mapped` owns.
    let map = unsafe { memmap2::Mmap::map(&file) }?;
    Ok(RuleSet::open(Backing::Mapped(map))?)
}

#[cfg(unix)]
fn read_only(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o222 == 0
}
#[cfg(not(unix))]
fn read_only(metadata: &std::fs::Metadata) -> bool {
    metadata.permissions().readonly()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Builder, Entry, Name};

    #[test]
    fn mapped_and_heap_sets_agree() {
        let mut b = Builder::new();
        b.insert(Entry::Suffix(Name::parse("example.com").unwrap()));
        let artifact = b.build().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(artifact.digest().file_name());
        std::fs::write(&path, artifact.bytes()).unwrap();
        assert!(matches!(open(&path), Err(MapError::Writable)));
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
        let mapped = open(&path).unwrap();
        let heap = RuleSet::from_vec(artifact.bytes().to_vec()).unwrap();
        assert_eq!(mapped.digest(), heap.digest());
        assert_eq!(mapped.digest(), artifact.digest());
        assert_eq!(
            mapped.match_domain("a.example.com"),
            heap.match_domain("a.example.com")
        );
        assert!(matches!(
            open(dir.path()),
            Err(MapError::Writable | MapError::Io(_))
        ));
    }
}
