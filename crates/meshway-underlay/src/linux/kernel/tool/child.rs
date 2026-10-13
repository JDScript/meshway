//! Crash fence for an external kernel tool. The shell executes only a fixed
//! stdin handshake and quoted argv; no command text is assembled from input.
use crate::{Error, Result};
use meshway_private_storage::Directory;
use serde::{Deserialize, Serialize};
use std::{io::Read, os::unix::fs::MetadataExt, sync::Arc};

const FILE: &str = crate::files::EXIT_CHILD;
pub(super) const GATE: &str = "IFS= read -r gate && [ \"$gate\" = go ] && exec \"$@\"";

#[derive(Clone)]
pub(in crate::linux::kernel) struct Fence {
    pub directory: Arc<Directory>,
    pub installation: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    installation: String,
    boot: String,
    net_device: u64,
    net_inode: u64,
    /// Absent in a record of an earlier build or of a kernel without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    net_cookie: Option<u64>,
    pid_device: u64,
    pid_inode: u64,
    pid: u32,
    started: u64,
}

impl Fence {
    /// Called while the child is blocked on the handshake, before any tool can
    /// run. A crash before this durable write closes stdin and executes nothing.
    pub fn record(&self, pid: u32) -> Result<()> {
        let net = super::super::namespace::Identity::current()?;
        let namespace = std::fs::metadata("/proc/self/ns/pid")?;
        let started = start_time(pid)?.ok_or(Error::Worker)?;
        // In particular, a tool must never keep the nonpersistent exit TUN alive
        // after the daemon is killed. Reject descriptor inheritance before exec.
        for (index, entry) in std::fs::read_dir(format!("/proc/{pid}/fd"))?
            .take(4097)
            .enumerate()
        {
            if index == 4096 {
                return Err(Error::Capacity);
            }
            let entry = entry?;
            if std::fs::read_link(entry.path())
                .is_ok_and(|path| path == std::path::Path::new("/dev/net/tun"))
            {
                return Err(Error::Conflict);
            }
        }
        let record = Record {
            installation: self.installation.clone(),
            boot: net.boot,
            net_device: net.device,
            net_inode: net.inode,
            net_cookie: net.cookie,
            pid_device: namespace.dev(),
            pid_inode: namespace.ino(),
            pid,
            started,
        };
        self.directory.replace(
            FILE,
            &serde_json::to_vec(&record).map_err(|_| Error::Journal)?,
        )?;
        Ok(())
    }
}

/// No PID is killed during recovery. A matching still-live helper is a fence;
/// a reused PID is not touched. A different PID namespace cannot prove absence.
/// A helper of another network namespace (another boot, or a namespace
/// cookie other than this one's: a recreated container that reused the
/// destroyed namespace's inode) cannot change this one; its record is kept
/// as an archive.
pub(in crate::linux::kernel) fn settled(directory: &Directory) -> Result<()> {
    settled_in(directory, &super::super::namespace::Identity::current()?)
}

fn settled_in(directory: &Directory, net: &super::super::namespace::Identity) -> Result<()> {
    let Some(file) = directory.file(FILE, false)? else {
        return Ok(());
    };
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(Error::Journal);
    }
    let record: Record = serde_json::from_slice(&bytes).map_err(|_| Error::Journal)?;
    if record.pid == 0
        || record.installation.len() != 32
        || !record.installation.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Error::Journal);
    }
    if !net.is(
        &record.boot,
        record.net_device,
        record.net_inode,
        record.net_cookie,
    ) {
        let archive = directory.archive_stale(FILE, super::super::journal::STALE_ARCHIVES)?;
        tracing::info!(
            old_namespace_inode = record.net_inode,
            old_namespace_cookie = record.net_cookie,
            archive,
            "Archived the exit-child record of another network namespace"
        );
        return Ok(());
    }
    let namespace = std::fs::metadata("/proc/self/ns/pid")?;
    if record.pid_device != namespace.dev()
        || record.pid_inode != namespace.ino()
        || start_time(record.pid)? == Some(record.started)
    {
        return Err(Error::Conflict);
    }
    Ok(())
}

fn start_time(pid: u32) -> Result<Option<u64>> {
    let file = match std::fs::File::open(format!("/proc/{pid}/stat")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut text = String::new();
    file.take(4097).read_to_string(&mut text)?;
    if text.len() > 4096 {
        return Err(Error::Journal);
    }
    let (_, fields) = text.rsplit_once(") ").ok_or(Error::Journal)?;
    let fields: Vec<_> = fields.split_ascii_whitespace().collect();
    // Zombies cannot execute another mutation; their PID may await another reaper.
    if fields.first().copied() == Some("Z") {
        return Ok(None);
    }
    fields
        .get(19)
        .ok_or(Error::Journal)?
        .parse()
        .map(Some)
        .map_err(|_| Error::Journal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::kernel::namespace::Identity;

    /// A helper recorded in another PID namespace: absence is unprovable
    /// there, so only the network namespace decides.
    fn record(directory: &Directory, net: &Identity, cookie: Option<u64>) {
        let record = Record {
            installation: "2ecb1e7a8c0f2a492def3834781185ca".into(),
            boot: net.boot.clone(),
            net_device: net.device,
            net_inode: net.inode,
            net_cookie: cookie,
            pid_device: 5,
            pid_inode: 4026532510,
            pid: 45657,
            started: 5999400,
        };
        directory
            .replace(FILE, &serde_json::to_vec(&record).unwrap())
            .unwrap();
    }

    fn archives(directory: &Directory) -> usize {
        std::fs::read_dir(directory.path())
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("exit-child.json.stale-")
            })
            .count()
    }

    #[test]
    fn same_inode_another_cookie_is_archived_and_the_same_cookie_stays_a_fence() {
        let temp = tempfile::tempdir().unwrap();
        let directory = Directory::open(&temp.path().join("node"), "node.lock").unwrap();
        let net = Identity {
            boot: "e2118d8e-699e-44fd-b84c-58c15f9cdbe6\n".into(),
            device: 5,
            inode: 4026532721,
            cookie: Some(41),
        };
        // The recreated container (N5): same inode, new namespace.
        record(&directory, &net, Some(40));
        settled_in(&directory, &net).unwrap();
        assert!(directory.file(FILE, false).unwrap().is_none());
        assert_eq!(archives(&directory), 1);
        // The same namespace: a helper of another PID namespace stays a fence.
        record(&directory, &net, Some(41));
        assert!(matches!(settled_in(&directory, &net), Err(Error::Conflict)));
        assert!(directory.file(FILE, false).unwrap().is_some());
        // An earlier build's record carries no cookie: the inode rule.
        record(&directory, &net, None);
        assert!(matches!(settled_in(&directory, &net), Err(Error::Conflict)));
        // A kernel without cookies: the inode rule.
        let old_kernel = Identity {
            cookie: None,
            ..net.clone()
        };
        record(&directory, &net, Some(40));
        assert!(matches!(
            settled_in(&directory, &old_kernel),
            Err(Error::Conflict)
        ));
        let moved = Identity {
            inode: net.inode + 1,
            ..old_kernel
        };
        settled_in(&directory, &moved).unwrap();
        assert_eq!(archives(&directory), 2);
    }

    #[test]
    fn a_record_without_a_cookie_still_parses() {
        let bytes = br#"{"installation":"2ecb1e7a8c0f2a492def3834781185ca","boot":"e2118d8e-699e-44fd-b84c-58c15f9cdbe6\n","net_device":5,"net_inode":4026532721,"pid_device":5,"pid_inode":4026532510,"pid":45657,"started":5999400}"#;
        let record: Record = serde_json::from_slice(bytes).unwrap();
        assert_eq!(record.net_cookie, None);
        assert!(
            !String::from_utf8(serde_json::to_vec(&record).unwrap())
                .unwrap()
                .contains("net_cookie")
        );
    }
}
