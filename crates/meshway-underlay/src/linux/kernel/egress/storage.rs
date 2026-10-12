use super::{plan::Ranges, program::Program};
use crate::{Error, Result};
use meshway_configuration::execution::EgressCapturePlan;
use meshway_private_storage::Directory;
use serde::{Deserialize, Serialize};
use std::{io::Read, sync::Arc};

const FILE: &str = crate::files::EGRESS;
const LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Device {
    pub index: u32,
    pub name: String,
    pub source: std::net::Ipv4Addr,
}
/// The capture sets and, with a device, the routes of the capture table:
/// the CIDR cover of `ranges.v4` through the device. No other route is ever
/// in that table.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Gate {
    pub ranges: Ranges,
    pub mesh: Ranges,
    pub device: Option<Device>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ledger {
    pub version: u32,
    pub boot: String,
    pub namespace_device: u64,
    pub namespace_inode: u64,
    pub program: Program,
    pub table: u32,
    pub plan: EgressCapturePlan,
    pub gate: Gate,
    pub pending_gate: Option<Gate>,
    pub receipt: Option<super::super::exit::objects::Receipt>,
    /// Owned RPDB rules: the capture-table selector.
    pub rules: Vec<Vec<u8>>,
    pub removed: bool,
}
pub(super) struct Store {
    pub directory: Arc<Directory>,
    writes: meshway_private_storage::Writes<Error>,
}
impl Store {
    pub async fn open(directory: Arc<Directory>) -> Result<(Self, Option<Ledger>)> {
        let source = directory.clone();
        let prior = tokio::task::spawn_blocking(move || -> Result<Option<Ledger>> {
            super::super::tool::child::settled(&source)?;
            let Some(file) = source.file(FILE, false)? else {
                return Ok(None);
            };
            let mut bytes = Vec::new();
            file.take((LIMIT + 1) as u64).read_to_end(&mut bytes)?;
            if bytes.len() > LIMIT {
                return Err(Error::Journal);
            }
            // A removed record owns nothing, whatever version wrote it.
            let raw: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| Error::Journal)?;
            if raw.get("removed") == Some(&serde_json::Value::Bool(true)) {
                source.archive_stale(FILE, super::super::journal::STALE_ARCHIVES)?;
                return Ok(None);
            }
            // Nor does one of another boot or namespace.
            let (boot, dev, ino) = super::super::journal::Journal::identity()?;
            let recorded = (
                raw.get("boot").and_then(serde_json::Value::as_str),
                raw.get("namespace_device")
                    .and_then(serde_json::Value::as_u64),
                raw.get("namespace_inode")
                    .and_then(serde_json::Value::as_u64),
            );
            let (Some(recorded_boot), Some(recorded_dev), Some(recorded_ino)) = recorded else {
                return Err(Error::Journal);
            };
            if (recorded_boot, recorded_dev, recorded_ino) != (boot.as_str(), dev, ino) {
                source.archive_stale(FILE, super::super::journal::STALE_ARCHIVES)?;
                return Ok(None);
            }
            supported(&raw)?;
            let ledger: Ledger = serde_json::from_value(raw).map_err(|_| Error::Journal)?;
            ledger.validate()?;
            Ok(Some(ledger))
        })
        .await
        .map_err(|_| Error::Worker)??;
        Ok((
            Self {
                directory,
                writes: meshway_private_storage::Writes::per_write(|| Error::Worker),
            },
            prior,
        ))
    }
    pub async fn settle(&mut self) -> Result<()> {
        self.writes.settle().await
    }
    pub async fn persist(&mut self, ledger: &Ledger) -> Result<()> {
        self.settle().await?;
        ledger.validate()?;
        let bytes = serde_json::to_vec(ledger).map_err(|_| Error::Journal)?;
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
/// The ledger version this build reads and writes.
const VERSION: u64 = 2;

/// A live (not removed) record of another version is named as such before
/// its content is parsed: only the version that wrote it can release what
/// it owns.
fn supported(raw: &serde_json::Value) -> Result<()> {
    match raw.get("version").and_then(serde_json::Value::as_u64) {
        Some(VERSION) => Ok(()),
        Some(version) => Err(Error::JournalVersion {
            file: FILE,
            version,
        }),
        None => Err(Error::Journal),
    }
}

impl Ledger {
    fn validate(&self) -> Result<()> {
        if u64::from(self.version) != VERSION
            || self.program.installation.len() != 32
            || !self
                .program
                .installation
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            || !self.program.mark.is_power_of_two()
            || self.table < 10000
            || self.rules.len() > 1
        {
            return Err(Error::Journal);
        }
        for gate in std::iter::once(&self.gate).chain(self.pending_gate.iter()) {
            // A staged gate is the union of two plans; routes exist only
            // for one plan's ranges.
            if !gate.ranges.canonical()
                || gate.ranges.v4.len() + gate.ranges.v6.len() > super::plan::UNION_RANGES
                || (gate.device.is_some() && !gate.ranges.within_limits())
                || !gate.mesh.canonical()
                || gate.mesh.v4.len() + gate.mesh.v6.len() > 128
                || gate.device.as_ref().is_some_and(|device| {
                    device.index == 0 || device.name.is_empty() || device.name.len() > 15
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
        Ok(())
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn a_live_record_of_another_version_is_named_with_its_file_and_version() {
        let error = supported(&serde_json::json!({"version": 1, "removed": false})).unwrap_err();
        assert!(matches!(
            error,
            Error::JournalVersion {
                file: FILE,
                version: 1
            }
        ));
        let text = error.to_string();
        assert!(
            text.contains(FILE) && text.contains("version 1") && text.contains("meshway down"),
            "{text}"
        );
        assert!(supported(&serde_json::json!({"version": 2})).is_ok());
        assert!(matches!(
            supported(&serde_json::json!({"version": "x"})),
            Err(Error::Journal)
        ));
    }
}
