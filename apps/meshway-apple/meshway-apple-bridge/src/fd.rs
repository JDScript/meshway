//! Audited FFI boundary: duplicate an integer descriptor without borrowing an
//! unvalidated raw fd. Failure accepts no ownership; success owns only the dup.
use std::{
    io,
    os::fd::{FromRawFd, OwnedFd},
};

pub(super) fn duplicate(fd: i32) -> io::Result<OwnedFd> {
    // SAFETY: F_DUPFD_CLOEXEC consumes integer arguments, never pointers. Any
    // invalid input is rejected by the kernel. NE owns the original throughout.
    let copied = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if copied < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful fcntl returned a fresh descriptor, exclusively owned
    // here. No Rust/Swift owner can close this duplicate before its OwnedFd.
    Ok(unsafe { OwnedFd::from_raw_fd(copied) })
}
