//! Roaming rows (`roam/direct`, `roam/nat`, `roam/relay`, `roam/flap`,
//! `roam/exit`): real
//! daemons whose underlay the isolated Docker owner changes. Traffic,
//! perturbation and measurement belong to `dev/roam-fixture/runtime.py`;
//! these actors only run the daemons.
use crate::{Cli, Result, actor};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, Ipv6Addr},
    time::Duration,
};

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
    /// `roam/relay`: a public relay with channels to A and B.
    R,
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
            Self::R => "r",
            Self::Router => "router",
        }
    }
}

/// Which roaming row this topology runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scenario {
    /// A changes uplinks; B public (both edge directions).
    Direct,
    /// A and B each behind a port-restricted NAT: only coordinated traversal.
    Nat,
    /// `Nat` plus a public relay R on every policy path's alternative.
    Relay,
    /// A switches uplinks back and forth quickly; B public.
    Flap,
    /// A is B's exit; its masquerade leaves through the uplink it roams on,
    /// toward a service T on the WAN.
    Exit,
}
impl Scenario {
    pub fn phases(self) -> &'static [Phase] {
        match self {
            Self::Direct => &[Phase::Dialer, Phase::Listener],
            Self::Flap | Self::Exit => &[Phase::Dialer],
            Self::Nat | Self::Relay => &[Phase::Traversal],
        }
    }
    pub fn nodes(self) -> &'static [Role] {
        match self {
            Self::Relay => &[Role::A, Role::B, Role::C, Role::R],
            _ => &Role::NODES,
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
    /// A and B both behind NAT: the A–B channel is a coordinated traversal.
    Traversal,
}
impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Self::Dialer => "dialer",
            Self::Listener => "listener",
            Self::Traversal => "traversal",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Topology {
    pub scenario: Scenario,
    pub controller: Ipv4Addr,
    pub b: Ipv4Addr,
    pub c_lan: Ipv4Addr,
    /// The WAN address of A's first router, which forwards UDP 35000 to A
    /// (`direct` and `flap` only).
    pub a_published: Option<Ipv4Addr>,
    /// R's WAN address (`relay` only).
    pub r: Option<Ipv4Addr>,
    /// The controller's address on a network only A and it share, which A
    /// reaches the Server at: A's control path is IPv6 while its peers are
    /// IPv4 (`direct` only).
    pub controller6: Option<Ipv6Addr>,
    /// T, the WAN service B reaches through its exit A (`exit` only).
    pub t: Option<Ipv4Addr>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Configured {
    pub revision: meshway_model::Revision,
    pub nodes: BTreeMap<String, meshway_model::NodeId>,
    pub mesh: BTreeMap<String, Ipv4Addr>,
}

pub use crate::supervised::{read, ready, write};

pub async fn stopped(cli: &Cli) -> Result<bool> {
    crate::supervised::stopped(cli, "roam").await
}
pub async fn wait(cli: &Cli, name: &str) -> Result<()> {
    crate::supervised::wait(cli, "roam", name).await
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
    let controller: std::net::IpAddr = match (role, topology.controller6) {
        (Role::A, Some(controller6)) => controller6.into(),
        _ => topology.controller.into(),
    };
    let mut daemon = actor::spawn_named_daemon(cli, role.name(), controller)?;
    let result = async {
        crate::supervised::enroll(cli, "roam", role.name(), &mut daemon).await?;
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
