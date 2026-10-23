//! Private exit resource journal, under the Host's existing directory lock.
use super::{objects::Receipt, program::Program, tuple::OriginalTuple};
use crate::{Error, Result};
use meshway_model::{NetworkId, NodeId};
use meshway_private_storage::Directory;
use serde::{Deserialize, Serialize};
use std::{io::Read, sync::Arc};

const FILE: &str = crate::files::EXIT;
const LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Projection {
    pub incarnation: super::ExitIncarnation,
    pub slot: u32,
    pub tuple: OriginalTuple,
    pub expires_ms: u64,
    pub retired: bool,
    pub ct: Option<super::conntrack::Entry>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ledger {
    pub version: u32,
    pub boot: String,
    pub namespace_device: u64,
    pub namespace_inode: u64,
    /// Absent in a ledger of an earlier build or of a kernel without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace_cookie: Option<u64>,
    pub base_installation: String,
    pub network: NetworkId,
    pub node: NodeId,
    /// Role identity; a legacy instance id is accepted and kept when present.
    pub plan: crate::target::RoleIdentity,
    pub program: Program,
    pub tun_name: String,
    pub mtu: usize,
    pub route_table: u32,
    pub rules: Vec<Vec<u8>>,
    pub routes: Vec<Vec<u8>>,
    pub receipt: Option<Receipt>,
    pub projections: Vec<Projection>,
    /// Translated tuples already emitted by a retired role. Never authority.
    pub quarantine: Vec<super::conntrack::RetiredTuple>,
    pub legacy_quarantine: Vec<super::conntrack::Tuple>,
    pub closing: bool,
    pub removed: bool,
    pub pending: bool,
}

impl Ledger {
    fn validate(&self) -> Result<()> {
        if self.version != 2
            || self.base_installation.len() != 32
            || self.program.installation.len() != 32
            || !self
                .program
                .installation
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.program.interface == 0
            || !self.program.mark.is_power_of_two()
            || !(1280..=65535).contains(&self.mtu)
            || self.tun_name.is_empty()
            || self.tun_name.len() > 15
            || self.program.interface_name != self.tun_name
            || self.program.pools.is_empty()
            || self.program.pools.len() > 128
            || self.route_table < 10000
            || self.rules.len() > 8192
            || self.routes.len() > 1024
            || self.projections.len() > super::program::HISTORY
            || self.quarantine.len() + self.legacy_quarantine.len() > super::program::HISTORY
        {
            return Err(Error::Journal);
        }
        for (index, projection) in self.projections.iter().enumerate() {
            if projection.incarnation.is_zero()
                || projection.slot == 0
                || projection.slot as usize > super::program::HISTORY
                || self.projections[..index].iter().any(|prior| {
                    prior.incarnation == projection.incarnation
                        || prior.slot == projection.slot
                        || prior.tuple == projection.tuple
                })
            {
                return Err(Error::Journal);
            }
        }
        for rule in &self.rules {
            if super::super::plan::encode_rule(super::super::plan::decode_rule(rule)?) != *rule {
                return Err(Error::Journal);
            }
        }
        for route in &self.routes {
            let parsed = super::super::plan::decode_route(route)?;
            if super::super::plan::route_table(&parsed) != self.route_table
                || super::super::plan::encode_route(parsed) != *route
            {
                return Err(Error::Journal);
            }
        }
        Ok(())
    }
}

pub(super) struct Store {
    pub directory: Arc<Directory>,
    writes: meshway_private_storage::Writes<Error>,
}

impl Store {
    pub async fn open(directory: Arc<Directory>) -> Result<(Self, Option<Ledger>)> {
        let ledger = read(directory.clone(), true).await?;
        Ok((
            Self {
                directory,
                writes: writes(),
            },
            ledger,
        ))
    }
    pub async fn settle(&mut self) -> Result<()> {
        self.writes.settle().await
    }
    /// After a write of `ledger`'s owner failed, what the file holds is
    /// unknown (the previous record or the new one): read it back. A ledger
    /// of the same program (or none yet) is this owner's, and the owner
    /// continues with a new writer that records `ledger` again; a read that
    /// fails proves nothing (`Err` unknown, retried by the owner); anything
    /// else read back is a journal that does not match (`Journal`). Without a
    /// failed write this only settles.
    pub async fn recover(&mut self, ledger: &Ledger) -> Result<()> {
        if self.settle().await.is_ok() {
            return Ok(());
        }
        if let Some(recorded) = read(self.directory.clone(), false).await?
            && recorded.program.installation != ledger.program.installation
        {
            return Err(Error::Journal);
        }
        tracing::info!("Exit journal read back after a failed write; recording it again");
        self.writes = writes();
        self.persist(ledger).await
    }
    pub async fn persist(&mut self, ledger: &Ledger) -> Result<()> {
        self.settle().await?;
        ledger.validate()?;
        let bytes = serde_json::to_vec(ledger).map_err(|_| Error::Journal)?;
        if bytes.len() > LIMIT {
            return Err(Error::Capacity);
        }
        let directory = self.directory.clone();
        #[cfg(test)]
        let failing = super::super::tool::command::faults::take(
            &super::super::tool::command::faults::FAILING_EXIT_JOURNAL_WRITES,
        );
        self.writes
            .run(move || {
                #[cfg(test)]
                if failing {
                    return Err(std::io::Error::from_raw_os_error(28).into());
                }
                directory.replace(FILE, &bytes)?;
                Ok(())
            })
            .await
    }
}

/// A failed write refuses every later one of the same writer: its owner
/// reads the file back ([`Store::recover`]) before writing again.
fn writes() -> meshway_private_storage::Writes<Error> {
    meshway_private_storage::Writes::poisoning(|| Error::JournalUnwritten)
}

/// The ledger the file holds. `archive`: a ledger of another boot or
/// namespace is archived (a process start); otherwise it is not this
/// owner's (`Journal`).
async fn read(directory: Arc<Directory>, archive: bool) -> Result<Option<Ledger>> {
    tokio::task::spawn_blocking(move || -> Result<Option<Ledger>> {
        super::child::settled(&directory)?;
        let Some(file) = directory.file(FILE, false)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        file.take(LIMIT as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > LIMIT {
            return Err(Error::Journal);
        }
        let ledger = super::legacy::decode(&bytes)?;
        if !super::super::namespace::Identity::current()?.is(
            &ledger.boot,
            ledger.namespace_device,
            ledger.namespace_inode,
            ledger.namespace_cookie,
        ) {
            if !archive {
                return Err(Error::Journal);
            }
            directory.archive_stale(FILE, super::super::journal::STALE_ARCHIVES)?;
            return Ok(None);
        }
        ledger.validate()?;
        Ok(Some(ledger))
    })
    .await
    .map_err(|_| Error::Worker)?
}
