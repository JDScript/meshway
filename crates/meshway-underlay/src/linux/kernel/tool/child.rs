//! Crash fence for an external kernel tool. The shell executes only a fixed
//! stdin handshake and quoted argv; no command text is assembled from input.
use crate::{Error, Result};
use meshway_private_storage::Directory;
use serde::{Deserialize, Serialize};
use std::{io::Read, os::unix::fs::MetadataExt, sync::Arc};

const FILE: &str = "exit-child.json";
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
    pid_device: u64,
    pid_inode: u64,
    pid: u32,
    started: u64,
}

impl Fence {
    /// Called while the child is blocked on the handshake, before any tool can
    /// run. A crash before this durable write closes stdin and executes nothing.
    pub fn record(&self, pid: u32) -> Result<()> {
        let (boot, net_device, net_inode) = super::super::journal::Journal::identity()?;
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
            boot,
            net_device,
            net_inode,
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
pub(in crate::linux::kernel) fn settled(directory: &Directory) -> Result<()> {
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
    let (boot, device, inode) = super::super::journal::Journal::identity()?;
    if record.boot != boot || record.net_device != device || record.net_inode != inode {
        directory.archive_stale(FILE, super::super::journal::STALE_ARCHIVES)?;
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
