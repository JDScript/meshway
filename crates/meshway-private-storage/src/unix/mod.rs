use super::{Directory, Error};
use rustix::{
    fs::{Mode, OFlags, mkdirat, open as open_directory, openat},
    io::Errno,
};
use std::{
    fs::{File, TryLockError},
    io::Write,
    os::unix::fs::MetadataExt,
    path::{Component, Path},
};

const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const FILE_FLAGS: OFlags = OFlags::RDWR
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

pub(super) fn open(path: &Path, lock_name: &str) -> Result<Directory, Error> {
    open_owned(path, lock_name, true)?.ok_or(Error::Io)
}

pub(super) fn open_existing(path: &Path, lock_name: &str) -> Result<Option<Directory>, Error> {
    open_owned(path, lock_name, false)
}

fn open_owned(path: &Path, lock_name: &str, create: bool) -> Result<Option<Directory>, Error> {
    if !path.is_absolute() {
        return Err(Error::InvalidPath);
    }
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => names.push(name),
            _ => return Err(Error::InvalidPath),
        }
    }
    let Some((name, parents)) = names.split_last() else {
        return Err(Error::InvalidPath);
    };
    let mut parent =
        File::from(open_directory("/", DIRECTORY_FLAGS, Mode::empty()).map_err(io_error)?);
    check_ancestor(&parent)?;
    for name in parents {
        parent = File::from(
            match openat(&parent, *name, DIRECTORY_FLAGS, Mode::empty()) {
                Ok(fd) => fd,
                Err(Errno::NOENT) if !create => return Ok(None),
                Err(error) => return Err(io_error(error)),
            },
        );
        check_ancestor(&parent)?;
    }
    if create {
        match mkdirat(&parent, *name, Mode::from_raw_mode(0o700)) {
            Ok(()) => parent.sync_all().map_err(|_| Error::Io)?,
            Err(Errno::EXIST) => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    let directory = File::from(
        match openat(&parent, *name, DIRECTORY_FLAGS, Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) if !create => return Ok(None),
            Err(error) => return Err(io_error(error)),
        },
    );
    check(&directory, true)?;
    let lock = file(&directory, lock_name, create)?.ok_or(Error::UnsafeStorage)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(Error::InUse),
        Err(TryLockError::Error(_)) => return Err(Error::Io),
    }
    Ok(Some(Directory {
        path: path.to_owned(),
        lock_name: lock_name.into(),
        directory,
        _lock: lock,
    }))
}

pub(super) fn file(directory: &File, name: &str, create: bool) -> Result<Option<File>, Error> {
    let flags = if create {
        FILE_FLAGS | OFlags::CREATE | OFlags::EXCL
    } else {
        FILE_FLAGS
    };
    // Another reference to this owner can atomically replace the name between
    // openat and fstat. Never return the unlinked descriptor: reopen the current
    // name and validate it in full. This also works across separately locked
    // resources in the same private directory, without a process-global mutex.
    for attempt in 0..8 {
        let flags = if attempt == 0 { flags } else { FILE_FLAGS };
        let fd = match openat(directory, name, flags, Mode::from_raw_mode(0o600)) {
            Ok(fd) => fd,
            Err(Errno::EXIST) if create && attempt == 0 => {
                openat(directory, name, FILE_FLAGS, Mode::empty()).map_err(io_error)?
            }
            Err(Errno::NOENT) if !create && attempt == 0 => return Ok(None),
            Err(Errno::NOENT) => return Err(Error::UnsafeStorage),
            Err(error) => return Err(io_error(error)),
        };
        let file = File::from(fd);
        #[cfg(test)]
        tests::after_open();
        let metadata = file.metadata().map_err(|_| Error::Io)?;
        check_metadata(&metadata, false)?;
        match metadata.nlink() {
            1 => return Ok(Some(file)),
            0 => continue,
            _ => return Err(Error::UnsafeStorage),
        }
    }
    Err(Error::UnsafeStorage)
}

pub(super) fn publish_once(
    directory: &File,
    final_name: &str,
    staging: &str,
    bytes: Option<&[u8]>,
) -> Result<(), Error> {
    let flags = if bytes.is_some() {
        FILE_FLAGS | OFlags::CREATE | OFlags::EXCL
    } else {
        FILE_FLAGS
    };
    let mut file = File::from(
        openat(directory, staging, flags, Mode::from_raw_mode(0o600)).map_err(io_error)?,
    );
    check(&file, false)?;
    if let Some(bytes) = bytes {
        file.write_all(bytes).map_err(|_| Error::Io)?;
    }
    file.sync_all().map_err(|_| Error::Io)?;
    rustix::fs::renameat_with(
        directory,
        staging,
        directory,
        final_name,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(io_error)?;
    directory.sync_all().map_err(|_| Error::Io)
}

/// Remove all but the newest `keep` regular files named `<prefix><digits>`.
pub(super) fn prune_stale(directory: &File, prefix: &str, keep: usize) -> Result<(), Error> {
    let mut archives = Vec::new();
    let mut entries = rustix::fs::Dir::read_from(directory).map_err(io_error)?;
    while let Some(entry) = entries.read() {
        let entry = entry.map_err(io_error)?;
        let Ok(name) = entry.file_name().to_str() else {
            continue;
        };
        let Some(stamp) = name
            .strip_prefix(prefix)
            .filter(|stamp| !stamp.is_empty() && stamp.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|stamp| stamp.parse::<u128>().ok())
        else {
            continue;
        };
        archives.push((stamp, name.to_owned()));
        if archives.len() > 4096 {
            return Err(Error::Io);
        }
    }
    archives.sort();
    let excess = archives.len().saturating_sub(keep);
    for (_, name) in &archives[..excess] {
        // Only checked owner-only regular files are removed.
        file(directory, name, false)?;
        rustix::fs::unlinkat(directory, name.as_str(), rustix::fs::AtFlags::empty())
            .map_err(io_error)?;
    }
    if excess > 0 {
        directory.sync_all().map_err(|_| Error::Io)?;
    }
    Ok(())
}

pub(super) fn replace(directory: &File, final_name: &str, bytes: &[u8]) -> Result<(), Error> {
    file(directory, final_name, false)?;
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| Error::Io)?;
    let staging = format!(".write-{:032x}", u128::from_be_bytes(nonce));
    let mut file = File::from(
        openat(
            directory,
            staging.as_str(),
            FILE_FLAGS | OFlags::CREATE | OFlags::EXCL,
            Mode::from_raw_mode(0o600),
        )
        .map_err(io_error)?,
    );
    let result = (|| {
        check(&file, false)?;
        file.write_all(bytes).map_err(|_| Error::Io)?;
        file.sync_all().map_err(|_| Error::Io)?;
        // The directory is exclusively owned. Check destination type again so
        // accidental same-owner corruption is never silently overwritten.
        self::file(directory, final_name, false)?;
        rustix::fs::renameat(directory, staging.as_str(), directory, final_name)
            .map_err(io_error)?;
        directory.sync_all().map_err(|_| Error::Io)
    })();
    if result.is_err() {
        let _ = rustix::fs::unlinkat(directory, staging.as_str(), rustix::fs::AtFlags::empty());
    }
    result
}

fn check(file: &File, directory: bool) -> Result<(), Error> {
    let metadata = file.metadata().map_err(|_| Error::Io)?;
    check_metadata(&metadata, directory)?;
    if !directory && metadata.nlink() != 1 {
        return Err(Error::UnsafeStorage);
    }
    Ok(())
}

fn check_metadata(metadata: &std::fs::Metadata, directory: bool) -> Result<(), Error> {
    let kind_ok = if directory {
        metadata.is_dir()
    } else {
        metadata.is_file()
    };
    let mode = if directory { 0o700 } else { 0o600 };
    if !kind_ok
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o7777 != mode
    {
        return Err(Error::UnsafeStorage);
    }
    Ok(())
}

fn check_ancestor(directory: &File) -> Result<(), Error> {
    let metadata = directory.metadata().map_err(|_| Error::Io)?;
    let owner = metadata.uid();
    let trusted_owner = owner == 0 || owner == rustix::process::geteuid().as_raw();
    let writable_by_others = metadata.mode() & 0o022 != 0;
    let sticky = metadata.mode() & 0o1000 != 0;
    // Root/euid-owned sticky temporary ancestors cannot have our children
    // renamed by other UIDs. Root and our own UID are inside the trust boundary.
    if !metadata.is_dir() || !trusted_owner || (writable_by_others && !sticky) {
        return Err(Error::UnsafeStorage);
    }
    Ok(())
}

fn io_error(error: Errno) -> Error {
    match error {
        Errno::LOOP | Errno::NOTDIR => Error::UnsafeStorage,
        _ => Error::Io,
    }
}

#[cfg(test)]
mod tests;
