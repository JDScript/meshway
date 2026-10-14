//! The one network-namespace identity every Underlay record carries (the
//! capture journal, the exit and egress ledgers, the exit-child fence).
//!
//! Boot ID and nsfs device/inode identify a namespace only while it lives: a
//! recreated container may get the destroyed namespace's inode. The namespace
//! cookie never repeats within a boot, so a record that carries one is
//! compared by it. A record written without one (an earlier build) or a
//! kernel that has none (before 5.7) keeps the inode comparison.
use crate::{Error, Result};
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    /// As read, with its trailing newline.
    pub boot: String,
    pub device: u64,
    pub inode: u64,
    pub cookie: Option<u64>,
}

impl Identity {
    pub fn current() -> Result<Self> {
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
        if boot.len() > 64 {
            return Err(Error::Journal);
        }
        let namespace = std::fs::metadata("/proc/self/ns/net")?;
        Ok(Self {
            boot,
            device: namespace.dev(),
            inode: namespace.ino(),
            cookie: meshway_netns::cookie()?,
        })
    }

    /// Whether a record written as `(boot, device, inode, cookie)` was written
    /// in this namespace.
    pub fn is(&self, boot: &str, device: u64, inode: u64, cookie: Option<u64>) -> bool {
        meshway_netns::same(
            meshway_netns::Recorded {
                boot: &self.boot,
                device: self.device,
                inode: self.inode,
                cookie: self.cookie,
            },
            meshway_netns::Recorded {
                boot,
                device,
                inode,
                cookie,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Identity;

    #[test]
    fn a_reused_inode_with_another_cookie_is_another_namespace() {
        let current = Identity {
            boot: "b\n".into(),
            device: 5,
            inode: 4026532721,
            cookie: Some(9),
        };
        assert!(current.is("b", 5, 4026532721, Some(9)));
        assert!(!current.is("b", 5, 4026532721, Some(8)), "reused inode");
        assert!(!current.is("b", 5, 4026532722, Some(9)));
        assert!(!current.is("other", 5, 4026532721, Some(9)));
        // An earlier build's record, or a kernel without cookies: the inode.
        assert!(current.is("b\n", 5, 4026532721, None));
        let old_kernel = Identity {
            cookie: None,
            ..current
        };
        assert!(old_kernel.is("b", 5, 4026532721, Some(9)));
        assert!(!old_kernel.is("b", 5, 4026532722, None));
    }

    #[test]
    fn the_current_namespace_is_itself() {
        let current = Identity::current().unwrap();
        assert!(current.is(&current.boot, current.device, current.inode, current.cookie));
        assert_eq!(current, Identity::current().unwrap());
    }
}
