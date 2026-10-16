use super::{At, SystemDnsError, SystemDnsFailure};
use serde::{Deserialize, Serialize};
use std::{os::unix::fs::MetadataExt, path::PathBuf};

/// The network namespace this owner runs in: boot ID, nsfs device/inode and
/// the namespace cookie, compared by the one rule of every namespace record
/// (`meshway_netns::same`, shared with the Underlay).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Namespace {
    pub(super) boot: String,
    pub(super) device: u64,
    pub(super) inode: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) cookie: Option<u64>,
}
impl Namespace {
    /// Whether `self` and `other` name the same namespace.
    pub(super) fn same(&self, other: &Self) -> bool {
        meshway_netns::same(self.recorded(), other.recorded())
    }
    /// Whether a record written as `self` belongs to a namespace that no
    /// longer exists while `current` lives: the same boot and inode (its
    /// directory) under another cookie. A cookie is never reused within a
    /// boot, so the inode was reused by `current`.
    pub(super) fn replaced_by(&self, current: &Self) -> bool {
        self.boot == current.boot
            && self.device == current.device
            && self.inode == current.inode
            && self
                .cookie
                .zip(current.cookie)
                .is_some_and(|(old, new)| old != new)
    }
    pub(super) fn recorded(&self) -> meshway_netns::Recorded<'_> {
        meshway_netns::Recorded {
            boot: &self.boot,
            device: self.device,
            inode: self.inode,
            cookie: self.cookie,
        }
    }
    pub(super) fn directory(&self) -> PathBuf {
        PathBuf::from("/run/meshway")
            .join(format!("dns-{}-{}-{}", self.boot, self.device, self.inode))
    }
    pub(super) fn current() -> Result<Self, SystemDnsError> {
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|_| SystemDnsError::Unsupported)?;
        let boot = boot.trim().to_owned();
        if boot.len() != 36 || !boot.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            return Err(SystemDnsError::Unsupported);
        }
        let meta =
            std::fs::metadata("/proc/self/ns/net").map_err(|_| SystemDnsError::Unsupported)?;
        Ok(Self {
            boot,
            device: meta.dev(),
            inode: meta.ino(),
            cookie: meshway_netns::cookie().map_err(|_| SystemDnsError::Unsupported)?,
        })
    }
    pub(super) fn verify_pid(&self, pid: u32) -> Result<(), SystemDnsFailure> {
        let source = format!("/proc/{pid}/ns/net");
        let failure = |actual: String| {
            SystemDnsFailure::new(SystemDnsError::Conflict, "resolved_namespace")
                .expected(format!("resolved (pid {pid}) in netns {self}"))
                .actual(actual)
                .source(&source)
        };
        let meta = std::fs::metadata(&source).map_err(|error| failure(error.to_string()))?;
        if meta.dev() != self.device || meta.ino() != self.inode {
            return Err(failure(format!("netns {}:{}", meta.dev(), meta.ino())));
        }
        self.check_current()
    }
    /// This process still runs in the recorded namespace.
    pub(super) fn check_current(&self) -> Result<(), SystemDnsFailure> {
        let current = Self::current().at("namespace")?;
        if !self.same(&current) {
            return Err(SystemDnsFailure::new(SystemDnsError::Conflict, "namespace")
                .expected(self)
                .actual(&current)
                .source("/proc/self/ns/net, SO_NETNS_COOKIE, boot_id"));
        }
        Ok(())
    }
}
impl std::fmt::Display for Namespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.device, self.inode)?;
        if let Some(cookie) = self.cookie {
            write!(f, " cookie {cookie}")?;
        }
        write!(f, " boot {}", self.boot)
    }
}
