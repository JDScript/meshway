//! Roaming row (`roam/direct`): real daemons whose underlay the isolated
//! Docker owner changes. Traffic, perturbation and measurement belong to
//! `dev/roam-fixture/runtime.py`; these actors only run the daemons.
use crate::{Cli, Result, actor};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::BTreeMap, net::Ipv4Addr, path::Path, time::Duration};

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Controller,
    /// The roaming Node: two NAT uplinks and a LAN shared with C.
    A,
    /// A public Node A exchanges business traffic with.
    B,
    /// A's LAN peer, whose channel no uplink change may touch.
    C,
    /// A plain NAT router of one of A's uplinks; no Meshway process.
    Router,
}
impl Role {
    pub const NODES: [Self; 3] = [Self::A, Self::B, Self::C];
    pub fn name(self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::A => "a",
            Self::B => "b",
            Self::C => "c",
            Self::Router => "router",
        }
    }
}

/// Which side of the A–B edge dials.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    /// A dials B's published listener.
    Dialer,
    /// B dials A's listener, published at A's first router.
    Listener,
}
impl Phase {
    pub const ALL: [Self; 2] = [Self::Dialer, Self::Listener];
    pub fn name(self) -> &'static str {
        match self {
            Self::Dialer => "dialer",
            Self::Listener => "listener",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Topology {
    pub controller: Ipv4Addr,
    pub b: Ipv4Addr,
    pub c_lan: Ipv4Addr,
    /// The WAN address of A's first router, which forwards UDP 35000 to A.
    pub a_published: Ipv4Addr,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Configured {
    pub revision: meshway_model::Revision,
    pub nodes: BTreeMap<String, meshway_model::NodeId>,
    pub mesh: BTreeMap<String, Ipv4Addr>,
}

pub async fn write<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let temporary = path.with_extension("tmp");
    tokio::fs::write(&temporary, serde_json::to_vec_pretty(value)?).await?;
    tokio::fs::rename(temporary, path).await?;
    Ok(())
}
pub async fn read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&tokio::fs::read(path).await?)?)
}
pub async fn stopped(cli: &Cli) -> Result<bool> {
    Ok(tokio::fs::try_exists(cli.signals.join("roam-stop")).await?)
}
pub async fn wait(cli: &Cli, name: &str) -> Result<()> {
    tokio::time::timeout(crate::remaining(Duration::from_secs(600))?, async {
        while !tokio::fs::try_exists(cli.signals.join(name)).await? {
            if stopped(cli).await? {
                return Err("roam supervisor stopped the run".into());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    })
    .await?
}
pub async fn ready(cli: &Cli, name: &str) -> Result<()> {
    write(&cli.signals.join(name), &true).await
}

pub async fn run(cli: &Cli, role: Role) -> Result<()> {
    if std::env::var("MESHWAY_ROAM_FIXTURE_OWNED").as_deref() != Ok("1") {
        return Err("roam run requires an owned fixture namespace".into());
    }
    write(&cli.evidence.join("ready"), &true).await?;
    wait(cli, "roam-start").await?;
    match role {
        Role::Controller => return crate::controller::run(cli).await,
        Role::Router => {
            while !stopped(cli).await? {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            return Ok(());
        }
        _ => {}
    }
    let topology: Topology = read(&cli.public.join("roam-topology.json")).await?;
    wait(cli, "roam-enroll").await?;
    let mut daemon = actor::spawn_named_daemon(cli, role.name(), topology.controller)?;
    let result = async {
        actor::wait_ipc(cli, &mut daemon).await?;
        actor::command(cli, "up").await?;
        let status = actor::wait_status(cli, |s| s.enrollment_request_id.is_some()).await?;
        if daemon.id() != Some(status.process_id) {
            return Err("wrong daemon process".into());
        }
        write(
            &cli.signals
                .join(format!("roam-enrollment-{}.json", role.name())),
            &status,
        )
        .await?;
        let status = actor::wait_status(cli, |s| s.identity.is_some()).await?;
        write(
            &cli.signals
                .join(format!("roam-enrolled-{}.json", role.name())),
            &status,
        )
        .await?;
        while !stopped(cli).await? && !tokio::fs::try_exists(cli.signals.join("roam-down")).await? {
            let status = actor::command(cli, "status").await?;
            write(&cli.evidence.join("status.json"), &status).await?;
            if daemon.try_wait()?.is_some() {
                return Err("roam daemon ended unexpectedly".into());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok::<(), crate::Error>(())
    }
    .await;
    if let Err(error) = &result {
        crate::record_error(cli, "failure.txt", error.as_ref());
    }
    if let Ok(status) = actor::command(cli, "status").await {
        write(&cli.evidence.join("final-status.json"), &status).await?;
    }
    let down = actor::command(cli, "down").await;
    let stopped_daemon = actor::terminate(&mut daemon).await;
    result?;
    down?;
    stopped_daemon?;
    ready(cli, &format!("roam-down-{}", role.name())).await
}
