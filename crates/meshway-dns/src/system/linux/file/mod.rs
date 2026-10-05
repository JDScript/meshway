//! Explicitly owned regular resolv.conf only. Never follow a symlink, replace a
//! bind-mounted file, or fall back to in-place writes after rename failure.
use super::{SystemDnsError, SystemDnsPlan, storage};
use meshway_private_storage::Directory;
use rustix::fs::{AtFlags, Mode, OFlags, StatxFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    net::{IpAddr, SocketAddr},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

const LIMIT: usize = 65536;
const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const READ_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    device: u64,
    inode: u64,
    mount: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    digest: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    path: PathBuf,
    parent_device: u64,
    parent_inode: u64,
    bind: SocketAddr,
    original: Vec<u8>,
    original_identity: Identity,
    installed: Option<Identity>,
    pending: bool,
}
struct State {
    directory: Arc<Directory>,
    record: Record,
    active: bool,
}
pub(super) struct Backend {
    state: Arc<Mutex<State>>,
    writes: storage::Writes,
}
pub(super) fn has_record(directory: &Directory) -> Result<bool, SystemDnsError> {
    storage::read::<Record>(directory, "resolv-conf.json").map(|record| record.is_some())
}
impl Backend {
    pub fn prepare(
        directory: Arc<Directory>,
        path: PathBuf,
        bind: SocketAddr,
    ) -> Result<Self, SystemDnsError> {
        let (parent, _, file) = open(&path)?;
        let (identity, bytes) = identity(&file)?;
        let meta = parent.metadata().map_err(|_| SystemDnsError::Storage)?;
        let existing: Option<Record> = storage::read(&directory, "resolv-conf.json")?;
        let record = match existing {
            Some(record) => {
                if record.version != 1
                    || record.original.len() > LIMIT
                    || format!("{:x}", Sha256::digest(&record.original))
                        != record.original_identity.digest
                    || record.original_identity.mode & 0o7022 != 0
                    || record.original_identity.mode & 0o170000 != 0o100000
                    || (record.original_identity.uid != 0
                        && record.original_identity.uid != rustix::process::geteuid().as_raw())
                    || record.pending
                    || record.path != path
                    || record.bind != bind
                    || record.parent_device != meta.dev()
                    || record.parent_inode != meta.ino()
                    || record
                        .installed
                        .as_ref()
                        .unwrap_or(&record.original_identity)
                        != &identity
                {
                    return Err(SystemDnsError::Conflict);
                }
                record
            }
            None => Record {
                version: 1,
                path,
                parent_device: meta.dev(),
                parent_inode: meta.ino(),
                bind,
                original: bytes,
                original_identity: identity,
                installed: None,
                pending: false,
            },
        };
        upstreams(&record.original)?;
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                directory,
                record,
                active: true,
            })),
            writes: storage::Writes::default(),
        })
    }
    pub fn snapshot_id(&self) -> Result<String, SystemDnsError> {
        let state = self.state.lock().map_err(|_| SystemDnsError::Storage)?;
        let bytes = serde_json::to_vec(&(&state.record.path, &state.record.original_identity))
            .map_err(|_| SystemDnsError::Storage)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
    pub fn upstreams(&self) -> Result<Vec<SocketAddr>, SystemDnsError> {
        upstreams(
            &self
                .state
                .lock()
                .map_err(|_| SystemDnsError::Storage)?
                .record
                .original,
        )
    }
    pub fn transport(&self) -> Result<meshway_configuration::DnsTransport, SystemDnsError> {
        let state = self.state.lock().map_err(|_| SystemDnsError::Storage)?;
        let config = resolv_conf::Config::parse(&state.record.original)
            .map_err(|_| SystemDnsError::Settings)?;
        Ok(if config.use_vc {
            meshway_configuration::DnsTransport::Tcp
        } else {
            meshway_configuration::DnsTransport::Udp
        })
    }
    pub async fn settle(&mut self) -> Result<(), SystemDnsError> {
        self.writes.settle().await
    }
    pub async fn observe(&mut self) -> Result<(), SystemDnsError> {
        self.writes.settle().await?;
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || {
            state
                .lock()
                .map_err(|_| SystemDnsError::Storage)?
                .verify()
                .map(|_| ())
        })
        .await
        .map_err(|_| SystemDnsError::Storage)?
    }
    pub async fn install(&mut self, plan: &SystemDnsPlan) -> Result<(), SystemDnsError> {
        let state = self.state.clone();
        let search = plan.search.clone();
        self.writes
            .run(move || {
                let mut state = state.lock().map_err(|_| SystemDnsError::Storage)?;
                let mut config = resolv_conf::Config::parse(&state.record.original)
                    .map_err(|_| SystemDnsError::Settings)?;
                config.nameservers = vec![match state.record.bind.ip() {
                    IpAddr::V4(ip) => resolv_conf::ScopedIp::V4(ip),
                    IpAddr::V6(ip) => resolv_conf::ScopedIp::V6(ip, None),
                }];
                let mut combined = search;
                for suffix in config.get_last_search_or_domain() {
                    if !combined.iter().any(|s| s == suffix) {
                        combined.push(suffix.to_owned());
                    }
                }
                if !combined.is_empty() {
                    config.set_search(combined);
                }
                // Trust comes from the response, never from the replacement file.
                config.trust_ad = false;
                state.replace(config.to_string().as_bytes(), false)
            })
            .await
    }
    pub async fn down(&mut self) -> Result<(), SystemDnsError> {
        let state = self.state.clone();
        self.writes
            .run(move || {
                let mut state = state.lock().map_err(|_| SystemDnsError::Storage)?;
                if state.record.installed.is_some() {
                    let bytes = state.record.original.clone();
                    state.replace(&bytes, true)?;
                } else {
                    state.verify()?;
                }
                storage::save(
                    &state.directory,
                    "resolv-conf.json",
                    &Option::<Record>::None,
                )?;
                state.active = false;
                Ok(())
            })
            .await
    }
}
impl State {
    fn verify(&self) -> Result<(File, String), SystemDnsError> {
        if !self.active || self.record.pending {
            return Err(SystemDnsError::Conflict);
        }
        let (parent, name, file) = open(&self.record.path)?;
        let meta = parent.metadata().map_err(|_| SystemDnsError::Storage)?;
        if meta.dev() != self.record.parent_device
            || meta.ino() != self.record.parent_inode
            || &identity(&file)?.0
                != self
                    .record
                    .installed
                    .as_ref()
                    .unwrap_or(&self.record.original_identity)
        {
            return Err(SystemDnsError::Conflict);
        }
        Ok((parent, name))
    }
    fn replace(&mut self, bytes: &[u8], restore: bool) -> Result<(), SystemDnsError> {
        if bytes.len() > LIMIT {
            return Err(SystemDnsError::Capacity);
        }
        let (parent, name) = self.verify()?;
        let digest = format!("{:x}", Sha256::digest(bytes));
        if !restore
            && self
                .record
                .installed
                .as_ref()
                .is_some_and(|installed| installed.digest == digest)
        {
            return Ok(());
        }
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| SystemDnsError::Storage)?;
        let temporary = format!(".meshway-dns-{:032x}", u128::from_be_bytes(nonce));
        let mut file = File::from(
            rustix::fs::openat(
                &parent,
                temporary.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|_| SystemDnsError::Storage)?,
        );
        let result = (|| {
            let original = &self.record.original_identity;
            rustix::fs::fchown(
                &file,
                Some(rustix::process::Uid::from_raw(original.uid)),
                Some(rustix::process::Gid::from_raw(original.gid)),
            )
            .map_err(|_| SystemDnsError::Storage)?;
            file.set_permissions(std::fs::Permissions::from_mode(original.mode & 0o777))
                .map_err(|_| SystemDnsError::Storage)?;
            file.write_all(bytes).map_err(|_| SystemDnsError::Storage)?;
            file.sync_all().map_err(|_| SystemDnsError::Storage)?;
            self.verify()?;
            self.record.pending = true;
            storage::save(&self.directory, "resolv-conf.json", &Some(&self.record))?;
            // Recheck immediately before rename. Same-UID concurrent mutation is
            // an ownership violation; no unsupported cross-manager CAS claim.
            let (_, _, current) = open(&self.record.path)?;
            if &identity(&current)?.0
                != self
                    .record
                    .installed
                    .as_ref()
                    .unwrap_or(&self.record.original_identity)
            {
                return Err(SystemDnsError::Conflict);
            }
            rustix::fs::renameat(&parent, temporary.as_str(), &parent, name.as_str())
                .map_err(|_| SystemDnsError::Storage)?;
            parent.sync_all().map_err(|_| SystemDnsError::Storage)?;
            let (_, _, installed) = open(&self.record.path)?;
            let installed = identity(&installed)?.0;
            if installed.inode != file.metadata().map_err(|_| SystemDnsError::Storage)?.ino()
                || installed.digest != digest
            {
                return Err(SystemDnsError::Conflict);
            }
            self.record.installed = Some(installed);
            self.record.pending = false;
            storage::save(&self.directory, "resolv-conf.json", &Some(&self.record))
        })();
        if result.is_err() {
            let _ = rustix::fs::unlinkat(&parent, temporary.as_str(), AtFlags::empty());
        }
        result
    }
}
fn upstreams(bytes: &[u8]) -> Result<Vec<SocketAddr>, SystemDnsError> {
    let config = resolv_conf::Config::parse(bytes).map_err(|_| SystemDnsError::Settings)?;
    if config.nameservers.is_empty() || config.nameservers.len() > 8 {
        return Err(SystemDnsError::ResolverPlan);
    }
    // A no-reload libc configuration cannot implement a live up/down switch.
    // Scoped/link-dependent upstreams require a separately executable plan.
    if config.no_reload {
        return Err(SystemDnsError::Unsupported);
    }
    config
        .nameservers
        .iter()
        .map(|server| {
            if matches!(server, resolv_conf::ScopedIp::V6(_, Some(_))) {
                return Err(SystemDnsError::Unsupported);
            }
            let ip = IpAddr::from(server);
            if ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || matches!(ip, IpAddr::V6(ip) if ip.is_unicast_link_local())
            {
                return Err(SystemDnsError::ResolverPlan);
            }
            Ok(SocketAddr::new(ip, 53))
        })
        .collect()
}
fn check_directory(file: &File) -> Result<(), SystemDnsError> {
    let meta = file.metadata().map_err(|_| SystemDnsError::Storage)?;
    let uid = rustix::process::geteuid().as_raw();
    if !meta.is_dir() || (meta.uid() != 0 && meta.uid() != uid) || meta.mode() & 0o022 != 0 {
        return Err(SystemDnsError::Conflict);
    }
    Ok(())
}
fn mount(file: &File) -> Result<u64, SystemDnsError> {
    let stat = rustix::fs::statx(file, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)
        .map_err(|_| SystemDnsError::Unsupported)?;
    if stat.stx_mask & StatxFlags::MNT_ID.bits() == 0 {
        return Err(SystemDnsError::Unsupported);
    }
    Ok(stat.stx_mnt_id)
}
fn open(path: &Path) -> Result<(File, String, File), SystemDnsError> {
    if !path.is_absolute() {
        return Err(SystemDnsError::Settings);
    }
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(s) => names.push(s),
            _ => return Err(SystemDnsError::Settings),
        }
    }
    let Some((name, parents)) = names.split_last() else {
        return Err(SystemDnsError::Settings);
    };
    let name = name.to_str().ok_or(SystemDnsError::Settings)?.to_owned();
    let mut parent = File::from(
        rustix::fs::open("/", DIR_FLAGS, Mode::empty()).map_err(|_| SystemDnsError::Conflict)?,
    );
    check_directory(&parent)?;
    for name in parents {
        parent = File::from(
            rustix::fs::openat(&parent, *name, DIR_FLAGS, Mode::empty())
                .map_err(|_| SystemDnsError::Conflict)?,
        );
        check_directory(&parent)?;
    }
    let file = File::from(
        rustix::fs::openat(&parent, name.as_str(), READ_FLAGS, Mode::empty())
            .map_err(|_| SystemDnsError::Conflict)?,
    );
    let meta = file.metadata().map_err(|_| SystemDnsError::Storage)?;
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.mode() & 0o7022 != 0
        || (meta.uid() != 0 && meta.uid() != rustix::process::geteuid().as_raw())
        || mount(&file)? != mount(&parent)?
    {
        return Err(SystemDnsError::Conflict);
    }
    Ok((parent, name, file))
}
fn identity(file: &File) -> Result<(Identity, Vec<u8>), SystemDnsError> {
    let meta = file.metadata().map_err(|_| SystemDnsError::Storage)?;
    let mut bytes = Vec::new();
    file.take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SystemDnsError::Storage)?;
    if bytes.len() > LIMIT {
        return Err(SystemDnsError::Capacity);
    }
    Ok((
        Identity {
            device: meta.dev(),
            inode: meta.ino(),
            mount: mount(file)?,
            uid: meta.uid(),
            gid: meta.gid(),
            mode: meta.mode(),
            digest: format!("{:x}", Sha256::digest(&bytes)),
        },
        bytes,
    ))
}

#[cfg(test)]
mod tests;
