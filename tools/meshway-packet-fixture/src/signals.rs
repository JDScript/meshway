//! Small, public, typed rendezvous records. No credentials or execution plans.
use crate::{Result, STARTUP};
use meshway_model::{EnrollmentRequestId, NodeId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, clap::ValueEnum, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    A,
    R,
    B,
}
impl Role {
    pub const ALL: [Self; 3] = [Self::A, Self::R, Self::B];
    pub fn name(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::R => "r",
            Self::B => "b",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Signal {
    Enrollment {
        role: Role,
        request: EnrollmentRequestId,
    },
    Enrolled {
        role: Role,
        node: NodeId,
    },
    Go,
    Ready {
        role: Role,
    },
    Done,
    Report {
        role: Role,
        forwarded: u64,
        delivered: Option<u64>,
        tun_removed: Option<bool>,
    },
}

#[derive(Clone)]
pub struct Signals(PathBuf);
impl Signals {
    pub fn new(path: &Path) -> Self {
        Self(path.into())
    }
    pub async fn write(&self, name: &str, value: Signal) -> Result<()> {
        let destination = self.0.join(format!("{name}.json"));
        let temporary = self.0.join(format!(".{name}.{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec(&value)?;
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await?;
        file.write_all(&bytes).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(temporary, destination).await?;
        Ok(())
    }
    pub async fn read(&self, name: &str) -> Result<Option<Signal>> {
        let file = self.0.join(format!("{name}.json"));
        let metadata = match tokio::fs::metadata(&file).await {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() || metadata.len() > 4096 {
            return Err("invalid public signal".into());
        }
        Ok(Some(serde_json::from_slice(&tokio::fs::read(file).await?)?))
    }
    pub async fn wait(&self, name: &str) -> Result<Signal> {
        tokio::time::timeout(STARTUP, async {
            loop {
                if let Some(value) = self.read(name).await? {
                    return Ok(value);
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await?
    }
    pub async fn done(&self) -> Result<bool> {
        match self.read("done").await? {
            None => Ok(false),
            Some(Signal::Done) => Ok(true),
            _ => Err("invalid done signal".into()),
        }
    }
}
