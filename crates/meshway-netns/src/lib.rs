//! The cookie of the calling thread's network namespace (Linux ≥ 5.7).
//!
//! A namespace's nsfs inode identifies it only while it lives: once destroyed,
//! the next namespace (a recreated container) may get the same inode. The
//! kernel assigns each namespace a 64-bit cookie from a counter that only
//! grows within a boot, so `(boot, cookie)` never names two namespaces.
//!
//! No safe binding exposes `SO_NETNS_COOKIE`, so this crate is the workspace's
//! single audited `unsafe` block: one `getsockopt` on a socket it owns, with a
//! buffer of exactly the size it passes. Callers own every identity rule.
#![deny(unsafe_code)]

#[cfg(target_os = "linux")]
mod linux {
    use std::{io, os::fd::AsRawFd};

    #[cfg(any(target_arch = "sparc", target_arch = "sparc64"))]
    const SO_NETNS_COOKIE: libc::c_int = 0x0050;
    #[cfg(not(any(target_arch = "sparc", target_arch = "sparc64")))]
    const SO_NETNS_COOKIE: libc::c_int = 71;

    /// `Ok(None)` only when the kernel does not know the option (before 5.7).
    pub fn cookie() -> io::Result<Option<u64>> {
        // An unbound AF_UNIX socket belongs to the caller's network namespace
        // and touches no network state.
        let socket = std::os::unix::net::UnixDatagram::unbound()?;
        let mut value: u64 = 0;
        let mut length = std::mem::size_of::<u64>() as libc::socklen_t;
        #[allow(unsafe_code)]
        // SAFETY: `socket` stays open for the whole call. `value` and `length`
        // are live, exclusively borrowed locals; `length` is the exact size of
        // `value`, the most the kernel writes, and it reports what it wrote.
        let result = unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                SO_NETNS_COOKIE,
                (&raw mut value).cast(),
                &raw mut length,
            )
        };
        if result != 0 {
            let error = io::Error::last_os_error();
            return match error.raw_os_error() {
                Some(libc::ENOPROTOOPT) => Ok(None),
                _ => Err(error),
            };
        }
        if length as usize != std::mem::size_of::<u64>() || value == 0 {
            return Err(io::Error::other("unexpected SO_NETNS_COOKIE value"));
        }
        Ok(Some(value))
    }
}

#[cfg(target_os = "linux")]
pub use linux::cookie;

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn the_cookie_of_one_namespace_is_stable() {
        let first = super::cookie().unwrap();
        assert_eq!(first, super::cookie().unwrap());
        let thread = std::thread::spawn(super::cookie).join().unwrap().unwrap();
        assert_eq!(first, thread);
    }
}
