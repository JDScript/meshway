use ipnet::IpNet;
use meshway_private_storage::Directory;
use serde::{Deserialize, Serialize};
use std::{io::Read, os::unix::fs::MetadataExt, sync::Arc};

use super::{
    super::protection::Mark,
    plan::{RouteSpec, RuleSpec, decode_route, decode_rule, encode_route, encode_rule},
};
use crate::{Error, Result};

const FILE: &str = "underlay.json";
const LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RuleRecord {
    pub spec: RuleSpec,
    pub observed: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RouteRecord {
    pub spec: RouteSpec,
    pub observed: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TunRecord {
    pub name: String,
    pub ifindex: u32,
    pub mtu: usize,
    pub addresses: meshway_configuration::NodeAddresses,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
pub(super) enum Pending {
    AddRule { spec: RuleSpec },
    DeleteRule { record: RuleRecord },
    AddRoute { spec: RouteSpec },
    DeleteRoute { record: RouteRecord },
    CreateTun,
    CreateDnsAnchor,
    DeleteDnsAnchor,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    pub version: u32,
    pub boot: String,
    pub namespace_device: u64,
    pub namespace_inode: u64,
    pub installation: String,
    /// Restricts later installations; never reconstructs execution authority.
    pub binding: Option<(meshway_model::NetworkId, meshway_model::NodeId)>,
    pub mark: Mark,
    pub table: u32,
    pub start: u32,
    pub pool_limit: usize,
    pub pools: Vec<IpNet>,
    pub rules: Vec<RuleRecord>,
    pub routes: Vec<RouteRecord>,
    pub tun: Option<TunRecord>,
    #[serde(default)]
    pub anchor: Option<super::anchor::Record>,
    pub pending: Option<Pending>,
}

impl Journal {
    pub fn end(&self) -> u32 {
        self.start + (self.pool_limit as u32) * 2 + 1
    }

    pub fn identity() -> Result<(String, u64, u64)> {
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
        if boot.len() > 64 {
            return Err(Error::Journal);
        }
        let namespace = std::fs::metadata("/proc/self/ns/net")?;
        Ok((boot, namespace.dev(), namespace.ino()))
    }

    fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.pending.is_some()
            || self.installation.len() != 32
            || !self.installation.bytes().all(|b| b.is_ascii_hexdigit())
            || !(1..=128).contains(&self.pool_limit)
            || self.start == 0
            || self.start > 32765 - (self.pool_limit as u32) * 2 - 1
            || self.table < 10000
            || self.mark.value == 0
            || self.mark.mask != self.mark.value
            || !self.mark.mask.is_power_of_two()
            || self.pools.len() > self.pool_limit
            || self.rules.len() > self.pool_limit * 2 + 4
            || self.routes.len() > self.pool_limit
        {
            return Err(Error::Journal);
        }
        let (boot, device, inode) = Self::identity()?;
        if (!self.rules.is_empty() || !self.routes.is_empty() || self.anchor.is_some())
            && (self.boot != boot
                || self.namespace_device != device
                || self.namespace_inode != inode)
        {
            return Err(Error::Conflict);
        }
        if self.anchor.as_ref().is_some_and(|anchor| {
            anchor.identity.index == 0
                || anchor.hardware_address.len() != 6
                || anchor.mtu == 0
                || anchor.identity.boot.trim() != self.boot.trim()
                || anchor.identity.namespace_device != self.namespace_device
                || anchor.identity.namespace_inode != self.namespace_inode
                || anchor.identity.installation != self.installation
                || anchor.alias != format!("meshway-dns:{}", self.installation)
                || anchor.identity.name != format!("mwd{}", &self.installation[..10])
        }) {
            return Err(Error::Journal);
        }
        for record in &self.rules {
            let layout = match record.spec.kind {
                super::plan::RuleKind::Bypass { mark, end } => {
                    mark == self.mark && end == self.end() && record.spec.priority == self.start
                }
                super::plan::RuleKind::End => record.spec.priority == self.end(),
                super::plan::RuleKind::Lookup { pool } | super::plan::RuleKind::Sink { pool } => {
                    self.pools
                        .iter()
                        .position(|p| *p == pool)
                        .is_some_and(|index| {
                            let offset =
                                if matches!(record.spec.kind, super::plan::RuleKind::Lookup { .. })
                                {
                                    1
                                } else {
                                    2
                                };
                            record.spec.ipv6 == pool.addr().is_ipv6()
                                && record.spec.priority == self.start + index as u32 * 2 + offset
                        })
                }
            };
            if !layout
                || self
                    .rules
                    .iter()
                    .filter(|other| {
                        other.spec.ipv6 == record.spec.ipv6
                            && other.spec.priority == record.spec.priority
                    })
                    .count()
                    != 1
            {
                return Err(Error::Journal);
            }
            if record.spec.table != self.table
                || record.spec.priority < self.start
                || record.spec.priority > self.end()
                || encode_rule(record.spec.message()) != record.observed
                || encode_rule(decode_rule(&record.observed)?) != record.observed
            {
                return Err(Error::Journal);
            }
        }
        for record in &self.routes {
            if record.spec.table != self.table
                || !self.pools.contains(&record.spec.pool)
                || encode_route(record.spec.message()?) != record.observed
                || encode_route(decode_route(&record.observed)?) != record.observed
            {
                return Err(Error::Journal);
            }
        }
        if self.pools.iter().enumerate().any(|(index, pool)| {
            pool.addr() != pool.network()
                || pool.prefix_len() == 0
                || self.pools[..index].contains(pool)
        }) {
            return Err(Error::Journal);
        }
        if self.tun.as_ref().is_some_and(|tun| {
            tun.ifindex == 0
                || tun.name.is_empty()
                || tun.name.len() > 15
                || !tun.name.bytes().all(|b| b.is_ascii_alphanumeric())
                || !(1280..=65535).contains(&tun.mtu)
        }) {
            return Err(Error::Journal);
        }
        Ok(())
    }
}

pub(super) struct Store {
    directory: Arc<Directory>,
    write: Option<tokio::task::JoinHandle<std::result::Result<(), meshway_private_storage::Error>>>,
    poisoned: bool,
}

impl Store {
    pub async fn open(directory: Arc<Directory>) -> Result<(Self, Option<Journal>)> {
        let worker_directory = directory.clone();
        let journal = tokio::task::spawn_blocking(move || -> Result<Option<Journal>> {
            let Some(file) = worker_directory.file(FILE, false)? else {
                return Ok(None);
            };
            let mut bytes = Vec::new();
            file.take(LIMIT as u64 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > LIMIT {
                return Err(Error::Journal);
            }
            let journal: Journal = serde_json::from_slice(&bytes).map_err(|_| Error::Journal)?;
            journal.validate()?;
            Ok(Some(journal))
        })
        .await
        .map_err(|_| Error::Worker)??;
        Ok((
            Self {
                directory,
                write: None,
                poisoned: false,
            },
            journal,
        ))
    }

    pub async fn settle(&mut self) -> Result<()> {
        if let Some(write) = &mut self.write {
            let result = write.await;
            self.write = None;
            if !matches!(result, Ok(Ok(()))) {
                self.poisoned = true;
                return Err(Error::Journal);
            }
        }
        if self.poisoned {
            return Err(Error::Journal);
        }
        Ok(())
    }

    pub async fn persist(&mut self, journal: &Journal) -> Result<()> {
        self.settle().await?;
        let bytes = serde_json::to_vec(journal).map_err(|_| Error::Journal)?;
        if bytes.len() > LIMIT {
            return Err(Error::Capacity);
        }
        let directory = self.directory.clone();
        self.write = Some(tokio::task::spawn_blocking(move || {
            directory.replace(FILE, &bytes)
        }));
        self.settle().await
    }
}
