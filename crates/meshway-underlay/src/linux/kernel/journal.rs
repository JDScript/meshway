use ipnet::IpNet;
use meshway_private_storage::Directory;
use serde::{Deserialize, Serialize};
use std::{io::Read, sync::Arc};

use super::{
    super::protection::Mark,
    plan::{RouteSpec, RuleSpec, decode_route, decode_rule, encode_route, encode_rule},
};
use crate::{Error, Result};

const FILE: &str = crate::files::JOURNAL;
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
    /// Absent in a journal of an earlier build or of a kernel without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace_cookie: Option<u64>,
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

    /// Whether this namespace still holds what the journal says it installed:
    /// any of its recorded rules (exact bytes, in its own random table and
    /// priority range) or its DNS anchor (alias with the random
    /// installation). A journal without a namespace cookie (see
    /// [`super::namespace`]) identifies its namespace only by an inode the
    /// next namespace (a restarted container) may reuse; that one holds none
    /// of these objects.
    pub fn resident(&self, rules: &[Vec<u8>], aliases: &[String]) -> bool {
        self.rules
            .iter()
            .any(|record| rules.contains(&record.observed))
            || self
                .anchor
                .as_ref()
                .is_some_and(|anchor| aliases.contains(&anchor.alias))
    }

    /// Resources in another kernel or network namespace cannot be adopted or
    /// cleaned up here. Preserve their evidence and allocate in this namespace.
    pub fn is_stale(&self, current: &super::namespace::Identity) -> bool {
        !current.is(
            &self.boot,
            self.namespace_device,
            self.namespace_inode,
            self.namespace_cookie,
        )
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
        if (!self.rules.is_empty() || !self.routes.is_empty() || self.anchor.is_some())
            && self.is_stale(&super::namespace::Identity::current()?)
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
    writes: meshway_private_storage::Writes<Error>,
}

impl Store {
    pub fn directory(&self) -> Arc<Directory> {
        self.directory.clone()
    }
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
            if journal.is_stale(&super::namespace::Identity::current()?) {
                archive(&worker_directory, &journal, "changed_namespace")?;
                return Ok(None);
            }
            journal.validate()?;
            Ok(Some(journal))
        })
        .await
        .map_err(|_| Error::Worker)??;
        Ok((
            Self {
                directory,
                writes: meshway_private_storage::Writes::poisoning(|| Error::Journal),
            },
            journal,
        ))
    }

    pub async fn settle(&mut self) -> Result<()> {
        self.writes.settle().await
    }

    /// Preserve a journal written in another namespace that reused this
    /// namespace's nsfs inode (see [`Journal::resident`]).
    pub async fn archive_foreign(&mut self, journal: &Journal) -> Result<()> {
        self.settle().await?;
        let directory = self.directory.clone();
        let journal = journal.clone();
        tokio::task::spawn_blocking(move || archive(&directory, &journal, "reused_namespace_inode"))
            .await
            .map_err(|_| Error::Worker)?
    }

    pub async fn persist(&mut self, journal: &Journal) -> Result<()> {
        self.settle().await?;
        let bytes = serde_json::to_vec(journal).map_err(|_| Error::Journal)?;
        if bytes.len() > LIMIT {
            return Err(Error::Capacity);
        }
        let directory = self.directory.clone();
        self.writes
            .run(move || {
                directory.replace(FILE, &bytes)?;
                Ok(())
            })
            .await
    }
}

/// Archives kept per journal; older ones are removed (one archive per boot
/// or namespace change would otherwise grow without bound).
pub(super) const STALE_ARCHIVES: usize = 3;

/// Checked descriptor-relative, durable NOREPLACE rename. A failed archive
/// never permits a fresh allocation/overwrite.
fn archive(directory: &Directory, journal: &Journal, cause: &'static str) -> Result<()> {
    let archive = directory.archive_stale(FILE, STALE_ARCHIVES)?;
    tracing::info!(
        old_boot = journal.boot.trim(),
        old_namespace_device = journal.namespace_device,
        old_namespace_inode = journal.namespace_inode,
        old_namespace_cookie = journal.namespace_cookie,
        cause,
        archive,
        "Archived stale Underlay journal"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::kernel::namespace::Identity;

    fn journal(boot: &str) -> Vec<u8> {
        let current = Identity::current().unwrap();
        serde_json::json!({
            "version": 1,
            "boot": boot,
            "namespace_device": current.device,
            "namespace_inode": current.inode,
            "namespace_cookie": current.cookie,
            "installation": "11e127022ad1c0b02359a8298b165be2",
            "binding": null,
            "mark": {"value": 131072, "mask": 131072},
            "table": 36177953,
            "start": 7266,
            "pool_limit": 64,
            "pools": ["100.64.0.0/10"],
            "rules": [],
            "routes": [],
            "tun": null,
            "anchor": null,
            "pending": {"operation": "CreateTun"}
        })
        .to_string()
        .into_bytes()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn previous_boot_journal_is_not_a_conflict_or_a_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let directory = Arc::new(Directory::open(&temp.path().join("node"), "node.lock").unwrap());
        directory
            .replace(FILE, &journal("4496fc3a-3cee-488d-a13e-2942c1d2e249\n"))
            .unwrap();
        let (_, previous) = Store::open(directory.clone()).await.unwrap();
        assert!(previous.is_none());
        assert!(directory.file(FILE, false).unwrap().is_none());

        let boot = Identity::current().unwrap().boot;
        directory.replace(FILE, &journal(&boot)).unwrap();
        assert!(matches!(Store::open(directory).await, Err(Error::Journal)));
    }

    #[test]
    fn journal_classifies_same_namespace_changed_namespace_and_changed_boot() {
        let current = Identity::current().unwrap();
        let journal: Journal = serde_json::from_slice(&journal(&current.boot)).unwrap();
        let other = |change: fn(&mut Identity)| {
            let mut other = current.clone();
            change(&mut other);
            other
        };
        assert!(!journal.is_stale(&current));
        assert!(journal.is_stale(&other(|i| i.inode += 1)));
        assert!(journal.is_stale(&other(|i| i.boot = "another-boot".into())));
        assert!(journal.is_stale(&other(|i| i.device += 1)));
        // The same inode in a namespace with another cookie (a recreated
        // container), on kernels that have namespace cookies.
        if current.cookie.is_some() {
            assert!(journal.is_stale(&other(|i| i.cookie = i.cookie.map(|c| c + 1))));
        }
    }

    #[test]
    fn namespace_identity_needs_a_resident_object_not_only_the_inode() {
        let boot = Identity::current().unwrap().boot;
        let mut journal: Journal = serde_json::from_slice(&journal(&boot)).unwrap();
        journal.pending = None;
        let rule = super::super::plan::RuleSpec {
            kind: super::super::plan::RuleKind::End,
            priority: journal.end(),
            table: journal.table,
            ipv6: false,
        };
        let observed = encode_rule(rule.message());
        journal.rules.push(RuleRecord {
            spec: rule,
            observed: observed.clone(),
        });
        assert!(journal.resident(std::slice::from_ref(&observed), &[]));
        assert!(
            !journal.resident(&[], &[]),
            "same inode, none of its objects"
        );
        assert!(!journal.resident(&[vec![1, 2, 3]], &["meshway-dns:other".into()]));
        journal.rules.clear();
        assert!(!journal.resident(&[observed], &[]));
    }

    #[tokio::test]
    async fn changed_namespace_archives_exact_journal_and_same_namespace_recovers() {
        let temp = tempfile::tempdir().unwrap();
        let directory = Arc::new(Directory::open(&temp.path().join("node"), "node.lock").unwrap());
        let boot = Identity::current().unwrap().boot;
        let mut journal: Journal = serde_json::from_slice(&journal(&boot)).unwrap();
        journal.pending = None;
        directory
            .replace(FILE, &serde_json::to_vec(&journal).unwrap())
            .unwrap();
        assert!(Store::open(directory.clone()).await.unwrap().1.is_some());
        journal.namespace_inode += 1;
        let bytes = serde_json::to_vec(&journal).unwrap();
        directory.replace(FILE, &bytes).unwrap();
        assert!(Store::open(directory.clone()).await.unwrap().1.is_none());
        assert!(directory.file(FILE, false).unwrap().is_none());
        let archives: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("underlay.json.stale-")
            })
            .collect();
        assert_eq!(archives.len(), 1);
        assert_eq!(std::fs::read(&archives[0]).unwrap(), bytes);
        assert!(Store::open(directory).await.unwrap().1.is_none());
    }

    #[tokio::test]
    async fn same_inode_another_cookie_is_archived_and_no_cookie_keeps_the_inode_rule() {
        let current = Identity::current().unwrap();
        let Some(cookie) = current.cookie else {
            return; // kernel before 5.7: the inode rule above is the only one
        };
        let temp = tempfile::tempdir().unwrap();
        let directory = Arc::new(Directory::open(&temp.path().join("node"), "node.lock").unwrap());
        let mut journal: Journal = serde_json::from_slice(&journal(&current.boot)).unwrap();
        journal.pending = None;
        journal.namespace_cookie = Some(cookie + 1);
        directory
            .replace(FILE, &serde_json::to_vec(&journal).unwrap())
            .unwrap();
        assert!(Store::open(directory.clone()).await.unwrap().1.is_none());
        assert!(directory.file(FILE, false).unwrap().is_none());
        // Written by an earlier build: same boot and inode is this namespace.
        journal.namespace_cookie = None;
        let bytes = serde_json::to_vec(&journal).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("namespace_cookie"));
        directory.replace(FILE, &bytes).unwrap();
        assert!(Store::open(directory.clone()).await.unwrap().1.is_some());
        journal.namespace_cookie = Some(cookie);
        directory
            .replace(FILE, &serde_json::to_vec(&journal).unwrap())
            .unwrap();
        assert!(Store::open(directory).await.unwrap().1.is_some());
    }
}
