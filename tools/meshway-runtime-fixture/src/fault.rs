//! `fault/throttle`: real daemons (A→R→B, B's exit E, systemd-resolved on
//! each) under external pressure the supervisor `dev/fault-fixture/runtime.py`
//! applies from outside the daemons: CPU limits, a resolved restart, Control
//! packets dropped in the controller's namespace. These actors only run the
//! Server and the daemons and record what the daemons report; measurement and
//! gates belong to `dev/fault-fixture/evidence.py`.
use crate::{Cli, Result, actor, supervised};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    net::Ipv4Addr,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::io::AsyncWriteExt;

pub const ROW: &str = "fault";
/// Status samples per Node: 2/s for a 25-minute bound.
const SAMPLES: usize = 3000;
const INTERVAL: Duration = Duration::from_millis(500);
/// A resolved restart is answered again within this (the `udp/resolved`
/// row's limit, `dns.rs`).
const RESOLVED_LIMIT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Controller,
    /// The source of the business flow, through R to B.
    A,
    /// The mandatory relay of A→B.
    R,
    /// The Node under pressure: resolved, an egress capture through E.
    B,
    /// B's exit.
    E,
}
impl Role {
    pub const NODES: [Self; 4] = [Self::A, Self::R, Self::B, Self::E];
    pub fn name(self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::A => "a",
            Self::R => "r",
            Self::B => "b",
            Self::E => "e",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Topology {
    pub controller: Ipv4Addr,
    /// Each Node's address on the one fixture bridge, by role name.
    pub nodes: BTreeMap<String, Ipv4Addr>,
    /// The address B's profile sends through E (no host answers it).
    pub target: Ipv4Addr,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Configured {
    pub revision: meshway_model::Revision,
    pub nodes: BTreeMap<String, meshway_model::NodeId>,
    pub mesh: BTreeMap<String, Ipv4Addr>,
    pub dns_domain: String,
}

pub async fn run(cli: &Cli, role: Role) -> Result<()> {
    if std::env::var("MESHWAY_FAULT_FIXTURE_OWNED").as_deref() != Ok("1") {
        return Err("fault run requires an owned fixture namespace".into());
    }
    supervised::write(&cli.evidence.join("ready"), &true).await?;
    supervised::wait(cli, ROW, "fault-start").await?;
    if role == Role::Controller {
        return crate::controller::run(cli).await;
    }
    let topology: Topology = supervised::read(&cli.public.join("fault-topology.json")).await?;
    supervised::wait(cli, ROW, "fault-enroll").await?;
    let mut daemon = actor::spawn_named_daemon(cli, role.name(), topology.controller)?;
    let result = async {
        supervised::enroll(cli, ROW, role.name(), &mut daemon).await?;
        supervised::wait(cli, ROW, "fault-configured.json").await?;
        let configured: Configured =
            supervised::read(&cli.signals.join("fault-configured.json")).await?;
        // B resolves A's Mesh name after each resolved restart.
        let name = format!("a.{}.", configured.dns_domain.trim_end_matches('.'));
        observe(cli, role, &mut daemon, &name).await
    }
    .await;
    if let Err(error) = &result {
        crate::record_error(cli, "failure.txt", error.as_ref());
    }
    if let Ok(status) = actor::command(cli, "status").await {
        supervised::write(&cli.evidence.join("final-status.json"), &status).await?;
    }
    let down = actor::command(cli, "down").await;
    let stopped = actor::terminate(&mut daemon).await;
    result?;
    down?;
    stopped?;
    supervised::ready(cli, &format!("fault-down-{}", role.name())).await
}

fn unix_ns() -> Result<u128> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos())
}

/// The part of a Status the gates read, kept by name from its wire form so
/// one projection reads every daemon version the row runs on.
fn project(status: &Value) -> Value {
    let mut out = Map::new();
    let Some(fields) = status.as_object() else {
        return Value::Object(out);
    };
    for key in [
        "processId",
        "phase",
        "dataPlane",
        "lastError",
        "lastErrorDetail",
        "networkError",
        "sync",
        "syncError",
        "lastSyncAt",
        "retryAt",
        "observedAt",
        "applied",
        "saved",
        "changePending",
        "controlRelay",
    ] {
        if let Some(value) = fields.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    if let Some(netmap) = fields.get("netmap").and_then(Value::as_object) {
        let mut lease = Map::new();
        for key in ["netmap", "expiresAt"] {
            if let Some(value) = netmap.get(key) {
                lease.insert(key.into(), value.clone());
            }
        }
        out.insert("netmap".into(), Value::Object(lease));
    }
    if let Some(exit) = fields.get("exit").and_then(Value::as_object) {
        let mut role = Map::new();
        for key in ["capability", "cleanupPending"] {
            if let Some(value) = exit.get(key) {
                role.insert(key.into(), value.clone());
            }
        }
        out.insert("exit".into(), Value::Object(role));
    }
    if let Some(peers) = fields.get("peers").and_then(Value::as_array) {
        let peers = peers
            .iter()
            .map(|peer| {
                serde_json::json!({
                    "hostname": peer.get("hostname"),
                    "state": peer.get("state"),
                })
            })
            .collect();
        out.insert("peers".into(), Value::Array(peers));
    }
    Value::Object(out)
}

/// One requested resolved restart in progress (B).
struct Restart {
    number: u32,
    requested: Instant,
    restarted: Option<Instant>,
}

async fn observe(
    cli: &Cli,
    role: Role,
    daemon: &mut tokio::process::Child,
    name: &str,
) -> Result<()> {
    let mut samples = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(cli.evidence.join("status.jsonl"))
        .await?;
    let mut written = 0;
    let mut handled = 0;
    let mut restart: Option<Restart> = None;
    while !supervised::stopped(cli, ROW).await?
        && !tokio::fs::try_exists(cli.signals.join("fault-down")).await?
    {
        if daemon.try_wait()?.is_some() {
            return Err("fault daemon ended unexpectedly".into());
        }
        let started = Instant::now();
        let at = unix_ns()?;
        // Under a CPU limit the CLI may outlast its own bound: the sample
        // says so and the next one follows.
        let mut line = match actor::command(cli, "status").await {
            Ok(status) => {
                supervised::write(&cli.evidence.join("status.json"), &status).await?;
                project(&serde_json::to_value(&status)?)
            }
            Err(_) => serde_json::json!({ "error": "status unavailable" }),
        };
        if let Some(object) = line.as_object_mut() {
            object.insert("at".into(), Value::from(at as f64 / 1e9));
            object.insert(
                "elapsedMs".into(),
                Value::from(started.elapsed().as_millis() as u64),
            );
        }
        if written < SAMPLES {
            let mut bytes = serde_json::to_vec(&line)?;
            bytes.push(b'\n');
            samples.write_all(&bytes).await?;
            samples.flush().await?;
            written += 1;
        }
        if restart.is_none()
            && tokio::fs::try_exists(cli.signals.join(format!(
                "fault-resolved-restart-{}-{}",
                role.name(),
                handled + 1
            )))
            .await?
        {
            handled += 1;
            restart = Some(request_restart(handled).await?);
        }
        if let Some(pending) = &mut restart
            && let Some(result) = advance(cli, pending, name).await?
        {
            supervised::write(
                &cli.evidence
                    .join(format!("resolved-restart-{}.json", pending.number)),
                &result,
            )
            .await?;
            restart = None;
        }
        let pause = if restart.is_some() {
            Duration::from_millis(100)
        } else {
            INTERVAL
        };
        tokio::time::sleep(pause.saturating_sub(started.elapsed())).await;
    }
    // Only a completed row owes both restarts; an aborted one stops early.
    if role == Role::B
        && handled != 2
        && tokio::fs::try_exists(cli.signals.join("fault-complete")).await?
    {
        return Err("B did not run the two requested resolved restarts".into());
    }
    Ok(())
}

/// The actor's supervisor (`namespace.sh`) restarts resolved on SIGUSR1: a
/// new process and D-Bus owner with the same configuration, as `systemctl
/// restart` does; it writes `resolved-restarted-<n>.txt` once it serves.
async fn request_restart(number: u32) -> Result<Restart> {
    let supervisor = std::os::unix::process::parent_id().to_string();
    let status = tokio::process::Command::new("kill")
        .args(["-USR1", &supervisor])
        .status()
        .await?;
    if !status.success() {
        return Err("resolved restart request failed".into());
    }
    Ok(Restart {
        number,
        requested: Instant::now(),
        restarted: None,
    })
}

async fn advance(cli: &Cli, pending: &mut Restart, name: &str) -> Result<Option<Value>> {
    let restarted = match pending.restarted {
        Some(at) => at,
        None => {
            let marker = cli
                .evidence
                .join(format!("resolved-restarted-{}.txt", pending.number));
            if !restarted(&marker).await? {
                return Ok((pending.requested.elapsed() >= RESOLVED_LIMIT).then(|| {
                    serde_json::json!({ "restarted": false, "resolved": false, "recoveredMs": null })
                }));
            }
            *pending.restarted.insert(Instant::now())
        }
    };
    let at = unix_ns()?;
    if crate::dns::stub_answers(name).await.unwrap_or(false) {
        return Ok(Some(serde_json::json!({
            "restarted": true,
            "resolved": true,
            "name": name,
            "recoveredMs": restarted.elapsed().as_millis() as u64,
            "resolvedAt": at as f64 / 1e9,
        })));
    }
    Ok((restarted.elapsed() >= RESOLVED_LIMIT).then(|| {
        serde_json::json!({ "restarted": true, "resolved": false, "name": name, "recoveredMs": null })
    }))
}

async fn restarted(marker: &Path) -> Result<bool> {
    Ok(tokio::fs::try_exists(marker).await?)
}
