//! Local bearer credentials and private durable files; not mesh cryptography.
//! Unix ownership/permissions are mandatory for this initial implementation.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::Path,
};

use sha2::{Digest, Sha256};

mod write;
pub use write::{AtomicWriteError, atomic_write, ensure_durable};

pub fn credential() -> io::Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| io::Error::other("OS randomness unavailable"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn valid_credential(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn digest(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}

pub fn private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        if !path.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(path)?;
        }
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err(io::Error::other(
                "directory must be owned by the current user, non-symlink and private (0700)",
            ));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    Err(io::Error::other(
        "private credential storage currently requires Unix",
    ))
}

fn check_file(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            return Err(io::Error::other(
                "credential file must be a private, singly-linked regular file owned by the current user (0600)",
            ));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    Err(io::Error::other(
        "private credential storage currently requires Unix",
    ))
}

/// Refuses symlinks, hard links, other owners, and group/world-readable files.
pub fn open_private(path: &Path, create: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    let file = options.open(path)?;
    check_file(&file)?;
    Ok(file)
}

pub fn lock(path: &Path) -> io::Result<File> {
    let file = open_private(path, true)?;
    file.try_lock()
        .map_err(|_| io::Error::other("state is already in use by another process"))?;
    Ok(file)
}

pub fn read_private(path: &Path) -> io::Result<String> {
    read_private_bounded(path, 65536)
}

pub fn read_private_bounded(path: &Path, limit: usize) -> io::Result<String> {
    let mut value = String::new();
    open_private(path, false)?
        .take(limit as u64 + 1)
        .read_to_string(&mut value)?;
    if value.len() > limit {
        return Err(io::Error::other("private file exceeds size budget"));
    }
    Ok(value)
}
