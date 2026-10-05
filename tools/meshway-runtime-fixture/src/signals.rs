//! Run-private rendezvous contains public identities/status, never credentials.
use crate::{Result, STARTUP};
use meshway_ipc::Status;
use meshway_model::Revision;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    time::Duration,
};

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

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Underlay {
    pub controller: Ipv4Addr,
    pub actors: BTreeMap<Role, Ipv4Addr>,
    #[serde(default)]
    pub routers: BTreeMap<Role, NatRouter>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NatRouter {
    pub private: Ipv4Addr,
    pub wan: Ipv4Addr,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Addresses {
    pub ipv4: std::net::Ipv4Addr,
    pub ipv6: std::net::Ipv6Addr,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Signal {
    Status {
        role: Role,
        status: Box<Status>,
    },
    Configuration {
        revision: Revision,
        addresses: BTreeMap<Role, Addresses>,
        dns_domain: String,
    },
    Revision {
        revision: Revision,
    },
    Ready,
    Report {
        role: Role,
        node: meshway_model::NodeId,
        udp4: u64,
        udp6: u64,
        tcp4: u64,
        tcp6: u64,
        relay_tun_received: Option<u64>,
        tun_removed: bool,
        capture_removed: bool,
        identity_preserved: bool,
        dns: Option<crate::dns::Evidence>,
        fallback: Option<Box<crate::fallback::Evidence>>,
    },
}

pub struct Signals(PathBuf);
impl Signals {
    pub fn new(path: &Path) -> Self {
        Self(path.into())
    }
    pub async fn write(&self, name: &str, value: &Signal) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        let destination = self.0.join(format!("{name}.json"));
        let temporary = self.0.join(format!(".{name}.{}.tmp", std::process::id()));
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await?;
        file.write_all(&serde_json::to_vec(value)?).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(temporary, destination).await?;
        Ok(())
    }
    pub async fn read(&self, name: &str) -> Result<Option<Signal>> {
        for role in ["a", "r", "b", "controller"] {
            if tokio::fs::try_exists(self.0.join(format!("abort-{role}.json"))).await? {
                return Err("another runtime actor failed; inspect its private evidence".into());
            }
        }
        let path = self.0.join(format!("{name}.json"));
        let metadata = match tokio::fs::metadata(&path).await {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() || metadata.len() > 64 * 1024 {
            return Err("invalid rendezvous file".into());
        }
        Ok(Some(serde_json::from_slice(&tokio::fs::read(path).await?)?))
    }
    pub async fn wait_for(&self, name: &str, duration: Duration) -> Result<Signal> {
        tokio::time::timeout(crate::remaining(duration)?, async {
            loop {
                if let Some(signal) = self.read(name).await? {
                    return Ok(signal);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await?
    }
    pub async fn wait(&self, name: &str) -> Result<Signal> {
        self.wait_for(name, STARTUP).await
    }
    pub async fn ready(&self, name: &str) -> Result<()> {
        if !matches!(self.wait(name).await?, Signal::Ready) {
            return Err("invalid ready signal".into());
        }
        Ok(())
    }
}

pub async fn underlay(cli: &crate::Cli) -> Result<Underlay> {
    let underlay: Underlay =
        serde_json::from_slice(&tokio::fs::read(cli.public.join("underlay.json")).await?)?;
    if underlay.actors.len() != 3
        || Role::ALL
            .iter()
            .any(|role| !underlay.actors.contains_key(role))
    {
        return Err("incomplete underlay actor addresses".into());
    }
    Ok(underlay)
}
