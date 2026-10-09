//! Private exit resource journal, under the Host's existing directory lock.
use super::{objects::Receipt, program::Program, tuple::OriginalTuple};
use crate::{Error, Result};
use meshway_configuration::execution::ExitRolePlan;
use meshway_model::{NetworkId, NodeId};
use meshway_private_storage::Directory;
use serde::{Deserialize, Serialize};
use std::{io::Read, sync::Arc};

const FILE: &str = "underlay-exit.json";
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
    pub base_installation: String,
    pub network: NetworkId,
    pub node: NodeId,
    pub plan: ExitRolePlan,
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
    write: Option<tokio::task::JoinHandle<Result<()>>>,
    failed: bool,
}

impl Store {
    pub async fn open(directory: Arc<Directory>) -> Result<(Self, Option<Ledger>)> {
        let reader = directory.clone();
        let ledger = tokio::task::spawn_blocking(move || -> Result<Option<Ledger>> {
            super::child::settled(&reader)?;
            let Some(file) = reader.file(FILE, false)? else {
                return Ok(None);
            };
            let mut bytes = Vec::new();
            file.take(LIMIT as u64 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > LIMIT {
                return Err(Error::Journal);
            }
            let ledger = super::legacy::decode(&bytes)?;
            let (boot, device, inode) = super::super::journal::Journal::identity()?;
            if ledger.boot != boot
                || ledger.namespace_device != device
                || ledger.namespace_inode != inode
            {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| Error::Journal)?
                    .as_nanos();
                reader.publish_once(&format!("{FILE}.stale-{stamp}"), FILE, None)?;
                return Ok(None);
            }
            ledger.validate()?;
            Ok(Some(ledger))
        })
        .await
        .map_err(|_| Error::Worker)??;
        Ok((
            Self {
                directory,
                write: None,
                failed: false,
            },
            ledger,
        ))
    }
    pub async fn settle(&mut self) -> Result<()> {
        if let Some(write) = &mut self.write {
            let result = write.await;
            self.write = None;
            if !matches!(result, Ok(Ok(()))) {
                self.failed = true;
            }
        }
        if self.failed {
            Err(Error::Journal)
        } else {
            Ok(())
        }
    }
    pub async fn persist(&mut self, ledger: &Ledger) -> Result<()> {
        self.settle().await?;
        ledger.validate()?;
        let bytes = serde_json::to_vec(ledger).map_err(|_| Error::Journal)?;
        if bytes.len() > LIMIT {
            return Err(Error::Capacity);
        }
        let directory = self.directory.clone();
        self.write = Some(tokio::task::spawn_blocking(move || {
            directory.replace(FILE, &bytes)?;
            Ok(())
        }));
        self.settle().await
    }
}
