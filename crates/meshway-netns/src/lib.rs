//! The cookie of the calling thread's network namespace (Linux ≥ 5.7).
//!
//! A namespace's nsfs inode identifies it only while it lives: once destroyed,
//! the next namespace (a recreated container) may get the same inode. The
//! kernel assigns each namespace a 64-bit cookie from a counter that only
//! grows within a boot, so `(boot, cookie)` never names two namespaces.
//!
//! No safe binding exposes `SO_NETNS_COOKIE`, so this crate is the workspace's
//! single audited `unsafe` block: one `getsockopt` on a socket it owns, with a
//! buffer of exactly the size it passes. [`same`] is the one rule comparing
//! a recorded namespace with another.
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

/// A network namespace as a record names it: boot ID, nsfs device/inode and
/// the cookie, when one was read.
#[derive(Clone, Copy, Debug)]
pub struct Recorded<'a> {
    pub boot: &'a str,
    pub device: u64,
    pub inode: u64,
    pub cookie: Option<u64>,
}

/// The one identity rule of every namespace record (Underlay journals and
/// ledgers, the system DNS reservation): the same boot and device/inode
/// and, when both carry a cookie, the same cookie. An inode is reused by a
/// later namespace (a recreated container); a cookie never is within a boot.
/// A record without a cookie (an earlier build) or a kernel without cookies
/// falls back to the inode.
pub fn same(a: Recorded<'_>, b: Recorded<'_>) -> bool {
    a.boot.trim() == b.boot.trim()
        && a.device == b.device
        && a.inode == b.inode
        && match (a.cookie, b.cookie) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
}

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

#[cfg(test)]
mod rule_tests {
    use super::{Recorded, same};

    #[test]
    fn cookies_decide_when_both_records_carry_one() {
        let at = |inode, cookie| Recorded {
            boot: "b\n",
            device: 5,
            inode,
            cookie,
        };
        assert!(same(at(7, Some(9)), at(7, Some(9))));
        assert!(!same(at(7, Some(9)), at(7, Some(8))), "reused inode");
        assert!(!same(at(7, Some(9)), at(8, Some(9))));
        assert!(same(at(7, None), at(7, Some(9))));
        assert!(!same(at(7, None), at(8, None)));
        let other_boot = Recorded {
            boot: "c",
            ..at(7, Some(9))
        };
        assert!(!same(at(7, Some(9)), other_boot));
        assert!(same(
            at(7, None),
            Recorded {
                boot: "b",
                ..at(7, None)
            }
        ));
    }
}
