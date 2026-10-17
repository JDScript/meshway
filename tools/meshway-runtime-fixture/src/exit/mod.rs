//! Real process fixture. The daemon owns all permits, packets and NAT resources.
use crate::{Cli, IO, Result, actor};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::BTreeMap, net::Ipv4Addr, path::Path, time::Duration};

mod dns;
pub use dns::{NAME_ZONE, SPLIT_ZONE};
mod service;
mod traffic;

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Controller,
    A,
    R,
    E,
    External,
    Lan,
    Resolver,
    /// A plain kernel router between E and the far link; no Meshway process.
    Router,
}
impl Role {
    pub const NODES: [Self; 3] = [Self::A, Self::R, Self::E];
    pub fn name(self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::A => "a",
            Self::R => "r",
            Self::E => "e",
            Self::External => "external",
            Self::Lan => "lan",
            Self::Resolver => "resolver",
            Self::Router => "router",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Topology {
    pub addresses: BTreeMap<String, BTreeMap<String, Ipv4Addr>>,
    pub lan_cidr: String,
    /// exit6/off: the IPv6 addresses of the dual-stack links.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub addresses6: BTreeMap<String, BTreeMap<String, std::net::Ipv6Addr>>,
    /// exit6/off: the exit LAN's ULA prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lan6_cidr: Option<String>,
}
impl Topology {
    pub fn controller(&self, role: Role) -> Ipv4Addr {
        self.addresses[match role {
            Role::A => "ca",
            Role::R => "cr",
            _ => "ce",
        }]["controller"]
    }
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
pub async fn wait(cli: &Cli, name: &str) -> Result<()> {
    tokio::time::timeout(crate::remaining(Duration::from_secs(180))?, async {
        while !tokio::fs::try_exists(cli.signals.join(name)).await? {
            if tokio::fs::try_exists(cli.signals.join("exit-stop")).await? {
                return Err("exit supervisor stopped the run".into());
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

/// After SIGKILL the old socket path remains until the new daemon binds it;
/// wait until this process answers.
async fn restarted_ipc(cli: &Cli, daemon: &mut tokio::process::Child) -> Result<()> {
    let process = daemon.id();
    tokio::time::timeout(crate::remaining(crate::STARTUP)?, async {
        loop {
            if let Ok(status) = actor::command(cli, "status").await
                && Some(status.process_id) == process
            {
                return Ok(());
            }
            if daemon.try_wait()?.is_some() {
                return Err("restarted exit daemon ended before IPC ready".into());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await?
}

pub async fn run(cli: &Cli, role: Role) -> Result<()> {
    if std::env::var("MESHWAY_EXIT_FIXTURE_OWNED").as_deref() != Ok("1") {
        return Err("exit run requires an owned fixture namespace".into());
    }
    write(&cli.evidence.join("ready"), &true).await?;
    wait(cli, "exit-start").await?;
    if role == Role::Controller {
        return crate::controller::run(cli).await;
    }
    match role {
        // E's only OS DNS upstream shares the isolated public-classified link.
        Role::External => {
            return tokio::try_join!(service::run(cli), dns::run(cli, role)).map(|_| ());
        }
        Role::Lan => return service::run(cli).await,
        Role::Resolver => return dns::run(cli, role).await,
        Role::Router => {
            while !tokio::fs::try_exists(cli.signals.join("exit-stop")).await? {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            return Ok(());
        }
        _ => {}
    }
    let topology: Topology = read(&cli.public.join("exit-topology.json")).await?;
    wait(cli, "exit-enroll").await?;
    let mut daemon = actor::spawn_named_daemon(cli, role.name(), topology.controller(role))?;
    let result = async {
        actor::wait_ipc(cli, &mut daemon).await?;
        actor::command(cli, "up").await?;
        let status = actor::wait_status(cli, |s| s.enrollment_request_id.is_some()).await?;
        if daemon.id() != Some(status.process_id) {
            return Err("wrong daemon process".into());
        }
        write(
            &cli.signals
                .join(format!("exit-enrollment-{}.json", role.name())),
            &status,
        )
        .await?;
        let status = actor::wait_status(cli, |s| s.identity.is_some()).await?;
        write(
            &cli.signals
                .join(format!("exit-enrolled-{}.json", role.name())),
            &status,
        )
        .await?;
        let mut crashed = false;
        let mut traffic_done = false;
        loop {
            if tokio::fs::try_exists(cli.signals.join("exit-stop")).await? {
                break;
            }
            if tokio::fs::try_exists(cli.signals.join("exit-down")).await? {
                break;
            }
            if role == Role::A
                && !traffic_done
                && tokio::fs::try_exists(cli.signals.join("exit-traffic")).await?
            {
                traffic::run(cli, &topology).await?;
                traffic_done = true;
                ready(cli, "exit-traffic-done").await?;
            }
            if role == Role::E
                && !crashed
                && tokio::fs::try_exists(cli.signals.join("exit-crash-e")).await?
            {
                actor::kill_join(&mut daemon).await?;
                ready(cli, "exit-crashed-e").await?;
                wait(cli, "exit-restart-after-crash").await?;
                daemon = actor::spawn_named_daemon(cli, role.name(), topology.controller(role))?;
                actor::wait_ipc(cli, &mut daemon).await?;
                // SIGKILL leaves the old socket path behind: wait for the new process.
                restarted_ipc(cli, &mut daemon).await?;
                crashed = true;
                ready(cli, "exit-crash-restarted-e").await?;
            }
            let status = actor::command(cli, "status").await?;
            write(&cli.evidence.join("status.json"), &status).await?;
            if daemon.try_wait()?.is_some() {
                return Err("exit daemon ended unexpectedly".into());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
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
    let stopped = actor::terminate(&mut daemon).await;
    result?;
    write(&cli.evidence.join("down-status.json"), &down?).await?;
    stopped?;
    ready(cli, &format!("exit-down-{}", role.name())).await?;
    tokio::time::timeout(Duration::from_secs(90), async {
        while !tokio::fs::try_exists(cli.signals.join("exit-stop")).await? {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok::<(), crate::Error>(())
    })
    .await?
}
