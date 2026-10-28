//! Actor side of the netmap lifecycle rows, after the shared startup and
//! persistent traffic. Every check reads the real daemon through its CLI
//! and the OS sockets; nothing here instantiates a control or data plane.
use super::{
    Capture, capture, command, owned_tun, route_proof, running, save, spawn_daemon, terminate,
    verify_tun_index, wait_ipc, wait_status, wait_status_for,
};
use crate::{
    Cli, NetmapScenario, Result, STARTUP,
    application::Traffic,
    recovery::now_ms,
    signals::{self, Addresses, Role, Signal, Signals},
};
use meshway_model::{NetmapRef, Revision};
use meshway_protocol::ipc::{DataPlane, Phase, Status};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};
use tokio::process::Child;

pub(super) struct Context<'a> {
    pub cli: &'a Cli,
    pub role: Role,
    pub signals: &'a Signals,
    pub daemon: &'a mut Child,
    pub addresses: &'a BTreeMap<Role, Addresses>,
    pub revision: Revision,
    pub traffic: &'a mut Option<Traffic>,
    pub tun: String,
    pub tun_index: u32,
}

/// Run `scenario`; returns the TUN that the final `down` must remove.
pub(super) async fn run(scenario: NetmapScenario, context: Context<'_>) -> Result<String> {
    artifact_held(context.cli, "initial").await?;
    match scenario {
        NetmapScenario::Offline => offline(context).await,
        NetmapScenario::Expiry => expiry(context).await,
        NetmapScenario::Restart => restart(context).await,
        NetmapScenario::Revoke => revoke(context).await,
        NetmapScenario::Relay => relay(context).await,
        NetmapScenario::Reconnect => reconnect(context).await,
    }
}

/// Every netmap row's netmap references one inline rule-set artifact: the
/// daemon holds it and its Control cache has exactly that one read-only file,
/// named by digest. After the restart row's offline restart it is the cache
/// alone that supplies it.
async fn artifact_held(cli: &Cli, stage: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let status = wait_status(cli, |status| {
        status
            .netmap
            .as_ref()
            .is_some_and(|netmap| netmap.missing_artifacts.is_empty())
    })
    .await?;
    let directory = cli.state.join("node").join("identity").join("artifacts");
    let mut files = Vec::new();
    let mut entries = tokio::fs::read_dir(&directory).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "artifacts.lock" {
            continue;
        }
        let mode = entry.metadata().await?.permissions().mode() & 0o7777;
        if !(name.starts_with("sha256-") && name.ends_with(".mwrs")) || mode != 0o400 {
            return Err(format!("unexpected artifact cache entry {name} ({mode:o})").into());
        }
        files.push(name);
    }
    if files.len() != 1 {
        return Err(format!("expected one cached artifact, found {files:?}").into());
    }
    save(
        cli,
        &format!("scenario-{stage}-artifact"),
        &serde_json::json!({ "files": files, "netmap": status.netmap }),
    )
    .await
}

async fn observed(signals: &Signals, name: &str, role: Role) -> Result<()> {
    signals
        .write(
            name,
            &Signal::Observed {
                role,
                at_ms: now_ms()?,
            },
        )
        .await
}

fn installed(status: &Status) -> Option<NetmapRef> {
    status.netmap.as_ref().map(|netmap| netmap.netmap)
}

async fn closed(signals: &Signals) -> Result<()> {
    match signals.wait("scenario-closed").await? {
        Signal::HttpAvailability {
            available: false, ..
        } => Ok(()),
        _ => Err("invalid scenario outage boundary".into()),
    }
}

async fn opened(signals: &Signals) -> Result<()> {
    match signals
        .wait_for("scenario-open", Duration::from_secs(1500))
        .await?
    {
        Signal::HttpAvailability {
            available: true, ..
        } => Ok(()),
        _ => Err("invalid scenario reopen boundary".into()),
    }
}

/// New 5-tuples, decided locally on the installed netmap, and (once per
/// run: the CLI evidence file is per stage) a CLI ping.
async fn fresh(context: &Context<'_>, stage: &str, ping: Option<crate::ping::Stage>) -> Result<()> {
    let mut traffic =
        Traffic::new(&context.addresses[&Role::A], &context.addresses[&Role::B]).await?;
    traffic.first_udp().await?;
    traffic
        .open_remaining_flows(&context.addresses[&Role::A])
        .await?;
    traffic.exchange().await?;
    if let Some(ping) = ping {
        let path = crate::fallback::original_path(context.signals).await?;
        let run = crate::ping_cli::run(context.cli, ping, "b", &path).await?;
        save(context.cli, &format!("scenario-{stage}-ping"), &run).await?;
    }
    save(
        context.cli,
        &format!("scenario-{stage}-new-flows"),
        &traffic.sockets()?,
    )
    .await
}

/// The Controller is unreachable for the whole window: the same sockets keep
/// working, new flows and ping are admitted locally, the installed netmap and
/// TUN never change, and HTTP recovery needs no second `up`.
async fn offline(context: Context<'_>) -> Result<String> {
    let Context {
        cli, role, signals, ..
    } = context;
    closed(signals).await?;
    let before = command(cli, "status").await?;
    let netmap = installed(&before).ok_or("no installed netmap before the outage")?;
    let mut checks = 0_u64;
    let mut fresh_flows = 0_u8;
    loop {
        let done = signals.read("scenario-window-end").await?.is_some();
        if let Some(traffic) = context.traffic.as_mut() {
            traffic.exchange().await?;
            if fresh_flows == 0 || (done && fresh_flows == 1) {
                fresh(
                    &context,
                    if fresh_flows == 0 {
                        "offline-start"
                    } else {
                        "offline-end"
                    },
                    (fresh_flows == 0).then_some(crate::ping::Stage::Restored),
                )
                .await?;
                fresh_flows += 1;
            }
        }
        verify_tun_index(&context.tun, context.tun_index).await?;
        let status = command(cli, "status").await?;
        if status.data_plane != DataPlane::Ready || installed(&status) != Some(netmap) {
            save(cli, "scenario-offline-failure-status", &status).await?;
            return Err("installed netmap changed while the Controller was unreachable".into());
        }
        if context.daemon.try_wait()?.is_some() {
            return Err("daemon exited during the Controller outage".into());
        }
        checks += 1;
        if done && (context.traffic.is_none() || fresh_flows == 2) {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    save(cli, "scenario-offline-checks", &checks).await?;
    observed(signals, &format!("scenario-offline-{}", role.name()), role).await?;
    opened(signals).await?;
    let status = running(cli, context.revision).await?;
    if installed(&status) != Some(netmap) {
        return Err("recovery reinstalled a different netmap".into());
    }
    verify_tun_index(&context.tun, context.tun_index).await?;
    if let Some(traffic) = context.traffic.as_mut() {
        traffic.exchange().await?;
    }
    save(cli, "scenario-recovered-status", &status).await?;
    observed(
        signals,
        &format!("scenario-recovered-{}", role.name()),
        role,
    )
    .await?;
    Ok(context.tun)
}

/// Bound on the daemon's own withdrawal after the lease deadline, from the
/// first Status without the lease to the TUN's removal.
const WITHDRAWAL: Duration = Duration::from_secs(2);

/// SP1: the window after the lease end, the Server still unreachable, over
/// which each daemon's busiest thread stays below [`EXPIRED_IDLE_CPU`].
const EXPIRED_IDLE_WINDOW: Duration = Duration::from_secs(60);

/// A withdrawn daemon whose Server is unreachable has no packet work: its
/// only wakeups are Control's 1–15 s retries and the DNS owner's 1 s
/// observation (dev 2026-10-28: 0 % per thread once recovered, against one
/// thread at 100 % before the fix; the `fault/throttle` row's 5 % bounds a
/// live plane under pressure).
const EXPIRED_IDLE_CPU: f64 = 0.02;

/// Control offers again after each failed attempt, at most 15 s apart
/// (`BACKOFF_MAX`): after this, it offers no netmap any more.
const WITHDRAWN_OFFER: Duration = Duration::from_secs(20);

/// B's resolved stalls this long: the DNS owner's observation misses its
/// 2 s deadline (the field's `observe_deadline` fault), and the worker's
/// retry of the withdrawal falls due (5 s) before the owner re-proves.
const RESOLVED_STALL: Duration = Duration::from_secs(10);

/// The Controller is unreachable past the lease: Engine withdraws every
/// component (no TUN, negative capture, traffic blocked), then a new lease
/// after reopening reinstalls the same content. Meanwhile B's resolved
/// stalls, as on `meshway-vm` (SP1): the DNS owner fails and recovers with
/// no netmap to install, and every daemon stays idle.
async fn expiry(mut context: Context<'_>) -> Result<String> {
    let Context {
        cli, role, signals, ..
    } = context;
    closed(signals).await?;
    let budget = cli.netmap_validity.as_std() + Duration::from_secs(90);
    let status = wait_status_for(cli, budget, |status| {
        status.data_plane == DataPlane::Suspended && status.desired_up && status.netmap.is_none()
    })
    .await?;
    save(cli, "scenario-expired-status", &status).await?;
    // Status drops the lease by the clock at the deadline; the daemon's
    // timer withdraws the components at the same deadline, which takes a
    // moment (tens of milliseconds) to remove the TUN.
    let expired = Instant::now();
    while tokio::fs::try_exists(Path::new("/sys/class/net").join(&context.tun)).await? {
        if expired.elapsed() >= WITHDRAWAL {
            return Err("lease end left the TUN installed".into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    save(
        cli,
        "scenario-expired-tun-removed-ms",
        &(expired.elapsed().as_millis() as u64),
    )
    .await?;
    route_proof(
        cli,
        &context.addresses[&Role::B],
        &context.tun,
        true,
        "scenario-expired",
    )
    .await?;
    if let Some(traffic) = context.traffic.as_mut() {
        traffic.blocked().await?;
    }
    if role == Role::B {
        tokio::time::sleep(WITHDRAWN_OFFER.saturating_sub(expired.elapsed())).await;
        stall_resolved(cli).await?;
    }
    expired_idle(&mut context).await?;
    observed(signals, &format!("scenario-offline-{}", role.name()), role).await?;
    opened(signals).await?;
    let status = running(cli, context.revision).await?;
    context.tun = owned_tun().await?;
    if let Some(traffic) = context.traffic.as_mut() {
        traffic.restored().await?;
    }
    save(cli, "scenario-recovered-status", &status).await?;
    observed(
        signals,
        &format!("scenario-recovered-{}", role.name()),
        role,
    )
    .await?;
    Ok(context.tun)
}

/// Stop this actor's systemd-resolved for [`RESOLVED_STALL`], then let it
/// run again; the DNS owner reports the fault and its re-proof.
async fn stall_resolved(cli: &Cli) -> Result<()> {
    let mut pid = None;
    let mut processes = tokio::fs::read_dir("/proc").await?;
    while let Some(entry) = processes.next_entry().await? {
        let Ok(candidate) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if tokio::fs::read_to_string(entry.path().join("comm"))
            .await
            .is_ok_and(|name| name.trim() == "systemd-resolve")
        {
            pid = Some(candidate);
        }
    }
    let pid = pid
        .ok_or("this actor runs no systemd-resolved")?
        .to_string();
    let signal = async |name: &str| -> Result<()> {
        if !tokio::process::Command::new("/bin/kill")
            .args([name, &pid])
            .status()
            .await?
            .success()
        {
            return Err("signalling systemd-resolved failed".into());
        }
        Ok(())
    };
    signal("-STOP").await?;
    let stalled = command(cli, "status").await;
    tokio::time::sleep(RESOLVED_STALL).await;
    let during = command(cli, "status").await;
    signal("-CONT").await?;
    let failed = |status: &Result<Status>| {
        status.as_ref().ok().and_then(|status| {
            status
                .network_error
                .as_ref()
                .map(|failure| failure.problem)
                .or(status.last_error)
        })
    };
    save(
        cli,
        "scenario-expired-resolved-stall",
        &serde_json::json!({
            "pid": pid,
            "stallMs": RESOLVED_STALL.as_millis() as u64,
            "stoppedStatus": stalled.as_ref().ok(),
            "stalledStatus": during.as_ref().ok(),
        }),
    )
    .await?;
    if failed(&during) != Some(meshway_protocol::ipc::Problem::Dns) {
        return Err("the DNS owner did not report the stalled resolved".into());
    }
    Ok(())
}

/// Every daemon thread's time on CPU (ns, `/proc/<pid>/task/*/schedstat`).
async fn thread_cpu(pid: u32) -> Result<BTreeMap<u32, (String, u64)>> {
    let mut threads = BTreeMap::new();
    let mut tasks = tokio::fs::read_dir(format!("/proc/{pid}/task")).await?;
    while let Some(task) = tasks.next_entry().await? {
        let Ok(tid) = task.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        // A thread that ended between the listing and the read is skipped.
        let (Ok(schedstat), Ok(name)) = (
            tokio::fs::read_to_string(task.path().join("schedstat")).await,
            tokio::fs::read_to_string(task.path().join("comm")).await,
        ) else {
            continue;
        };
        let ns = schedstat
            .split_whitespace()
            .next()
            .and_then(|field| field.parse().ok())
            .ok_or("unreadable schedstat")?;
        threads.insert(tid, (name.trim().to_owned(), ns));
    }
    Ok(threads)
}

/// SP1: past the lease end with the Server unreachable, the daemon's busiest
/// thread stays below [`EXPIRED_IDLE_CPU`] over [`EXPIRED_IDLE_WINDOW`].
async fn expired_idle(context: &mut Context<'_>) -> Result<()> {
    let pid = context.daemon.id().ok_or("daemon PID absent")?;
    let begin = thread_cpu(pid).await?;
    let started = Instant::now();
    tokio::time::sleep(EXPIRED_IDLE_WINDOW).await;
    let end = thread_cpu(pid).await?;
    let wall = started.elapsed().as_nanos() as f64;
    let mut threads: Vec<_> = end
        .iter()
        .map(|(tid, (name, ns))| {
            let before = begin.get(tid).map_or(0, |(_, ns)| *ns);
            (*tid, name.clone(), ns.saturating_sub(before) as f64 / wall)
        })
        .collect();
    threads.sort_by(|a, b| b.2.total_cmp(&a.2));
    let busiest = threads.first().map_or(0.0, |thread| thread.2);
    let status = command(context.cli, "status").await?;
    save(
        context.cli,
        "scenario-expired-idle-cpu",
        &serde_json::json!({
            "pid": pid,
            "windowMs": started.elapsed().as_millis() as u64,
            "limit": EXPIRED_IDLE_CPU,
            "busiest": busiest,
            "threads": threads
                .iter()
                .take(8)
                .map(|(tid, name, share)| serde_json::json!({ "tid": tid, "name": name, "share": share }))
                .collect::<Vec<_>>(),
            "status": status,
        }),
    )
    .await?;
    tracing::info!(
        phase = "scenario-expired-idle",
        role = context.role.name(),
        busiest,
        "daemon CPU past the lease end"
    );
    if busiest >= EXPIRED_IDLE_CPU {
        return Err(format!(
            "a daemon thread spun past the lease end: {:.1} % of one CPU",
            busiest * 100.0
        )
        .into());
    }
    if status.netmap.is_some() || status.applied.is_some() {
        return Err("the daemon left the expired state while the Server was unreachable".into());
    }
    Ok(())
}

/// The restart row first waits past the 120 s idle adjacency close with no
/// traffic, which is also past every peer fact's freshness bound (at most
/// 60 s after the last report): the redial and the restarted daemons must use
/// saved, stale last known facts alone.
const IDLE_CLOSE: Duration = Duration::from_secs(125);

/// Every daemon restarts while the Controller is unreachable. The signed
/// cached netmap installs at once; new flows cross all three restarted
/// daemons without any Server contact.
async fn restart(mut context: Context<'_>) -> Result<String> {
    let Context {
        cli, role, signals, ..
    } = context;
    closed(signals).await?;
    tokio::time::sleep(IDLE_CLOSE).await;
    // Every on-demand adjacency closed idle while the Controller is gone:
    // one ping redials from the installed netmap and last known facts.
    if context.traffic.is_some() {
        fresh(
            &context,
            "idle-redial",
            Some(crate::ping::Stage::IdleRedial),
        )
        .await?;
        observed(signals, "scenario-idle-redial", role).await?;
    } else {
        signals
            .wait_for("scenario-idle-redial", STARTUP + Duration::from_secs(60))
            .await?;
    }
    let before = command(cli, "status").await?;
    let netmap = installed(&before).ok_or("no installed netmap before restart")?;
    let identity = before.identity.clone();
    terminate(context.daemon).await?;
    *context.daemon = spawn_daemon(cli, role, signals::underlay(cli).await?.controller)?;
    wait_ipc(cli, context.daemon).await?;
    let status = wait_status(cli, |status| {
        status.data_plane == DataPlane::Ready
            && status.desired_up
            && installed(status) == Some(netmap)
    })
    .await?;
    if status.identity != identity || context.daemon.id() != Some(status.process_id) {
        return Err("restart changed identity or status came from another process".into());
    }
    save(cli, "scenario-restarted-status", &status).await?;
    artifact_held(cli, "restarted").await?;
    context.tun = owned_tun().await?;
    observed(
        signals,
        &format!("scenario-restarted-{}", role.name()),
        role,
    )
    .await?;
    for actor in Role::ALL {
        signals
            .wait_for(&format!("scenario-restarted-{}", actor.name()), STARTUP)
            .await?;
    }
    if let Some(traffic) = context.traffic.as_mut() {
        traffic.restored().await?;
        fresh(&context, "restarted", Some(crate::ping::Stage::Restored)).await?;
    }
    observed(signals, &format!("scenario-offline-{}", role.name()), role).await?;
    opened(signals).await?;
    let status = running(cli, context.revision).await?;
    if installed(&status) != Some(netmap) {
        return Err("recovery after restart reinstalled a different netmap".into());
    }
    if let Some(traffic) = context.traffic.as_mut() {
        traffic.exchange().await?;
    }
    observed(
        signals,
        &format!("scenario-recovered-{}", role.name()),
        role,
    )
    .await?;
    Ok(context.tun)
}

/// A's new flows and CLI ping to B take [A, R2, B] although [A, B] is now
/// allowed and preferred: A's router drops every A↔B packet, and of the
/// relays R2 alone has channels with both ends (R1 lost B, A has no channel
/// with R3). Status shows B relayed through R2 and R3 without a channel.
/// Flows established on [A, R1, B] keep that path and lose B with R1.
async fn relay(context: Context<'_>) -> Result<String> {
    let Context {
        cli, role, signals, ..
    } = context;
    let path = if role == Role::A {
        let Signal::Path { nodes } = signals
            .wait_for("scenario-relay-path", Duration::from_secs(600))
            .await?
        else {
            return Err("invalid relay path".into());
        };
        Some(nodes)
    } else {
        None
    };
    let (revision, _) = step(signals, "relay").await?;
    running(cli, revision).await?;
    if let Some(path) = path {
        let run = crate::ping_cli::run(cli, crate::ping::Stage::Relayed, "b", &path).await?;
        save(cli, "scenario-relay-ping", &run).await?;
        let mut traffic =
            Traffic::new(&context.addresses[&Role::A], &context.addresses[&Role::B]).await?;
        traffic.first_udp().await?;
        traffic
            .open_remaining_flows(&context.addresses[&Role::A])
            .await?;
        traffic.exchange().await?;
        save(cli, "scenario-relay-new-flows", &traffic.sockets()?).await?;
        observed(signals, "scenario-relay-observed", role).await?;
        let status = wait_status(cli, |status| {
            status.peers.iter().any(|peer| {
                peer.node_id == path[2]
                    && peer.state == meshway_protocol::ipc::PeerState::Relay
                    && peer.via == [path[1]]
            })
        })
        .await?;
        save(cli, "scenario-relay-status", &status).await?;
    }
    observed(
        signals,
        &format!("scenario-relay-done-{}", role.name()),
        role,
    )
    .await?;
    Ok(context.tun)
}

/// A's status: R reached over a selected channel of its own.
fn direct_to_r(status: &Status) -> bool {
    status.peers.iter().any(|peer| {
        peer.hostname == Role::R.name() && peer.state == meshway_protocol::ipc::PeerState::Direct
    })
}

async fn direct_probe(cli: &Cli) -> Result<bool> {
    Ok(direct_to_r(&command(cli, "status").await?))
}

/// A–R is an on-demand edge (R can dial A's reverse TCP listener): while it
/// has no channel, A keeps needing R the way a user does, with one `meshway
/// ping r` every two seconds. The reply is not awaited.
async fn direct_needed(cli: &Cli, last: &std::cell::Cell<Option<Instant>>) -> Result<bool> {
    let direct = direct_probe(cli).await?;
    if !direct
        && last
            .get()
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(2))
    {
        last.set(Some(Instant::now()));
        tokio::process::Command::new(cli.binaries.join("meshway"))
            .arg("--socket")
            .arg(cli.state.join("node/control.sock"))
            .args(["ping", Role::R.name(), "--count", "1"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
    }
    Ok(direct)
}

/// See the Controller's `reconnect`. A reports when its channel with R is
/// gone and when a new one is selected; R and A restart their own daemons.
async fn reconnect(mut context: Context<'_>) -> Result<String> {
    let Context {
        cli, role, signals, ..
    } = context;
    if role == Role::B {
        signals
            .wait_for("reconnect-done", Duration::from_secs(1500))
            .await?;
        return Ok(context.tun);
    }
    let budget = Duration::from_secs(180);
    let report =
        async |name: &str, at_ms: u64| signals.write(name, &Signal::Observed { role, at_ms }).await;
    let pinged = std::cell::Cell::new(None);
    let instance = |status: &Status| status.instance.as_ref().map(|lease| lease.instance_id);
    for stage in ["nat", "server"] {
        if stage == "server" {
            let before = instance(&command(cli, "status").await?);
            stepped(signals, "reconnect-server").await?;
            let status = wait_status_for(cli, budget, |status| {
                instance(status).is_some() && instance(status) != before
            })
            .await?;
            save(cli, "scenario-reconnect-instance", &status).await?;
            report(&format!("reconnect-instance-{}", role.name()), now_ms()?).await?;
        }
        if role == Role::A {
            stepped(signals, &format!("reconnect-break-{stage}")).await?;
            let lost = until(async || direct_probe(cli).await, false, budget).await?;
            report(&format!("reconnect-lost-{stage}"), lost).await?;
            stepped(signals, &format!("reconnect-heal-{stage}")).await?;
            let direct = until(async || direct_needed(cli, &pinged).await, true, budget).await?;
            save(
                cli,
                &format!("scenario-reconnect-{stage}-status"),
                &command(cli, "status").await?,
            )
            .await?;
            report(&format!("reconnect-direct-{stage}"), direct).await?;
        }
    }
    for actor in [Role::R, Role::A] {
        let stage = format!("restart-{}", actor.name());
        stepped(signals, &format!("reconnect-{stage}")).await?;
        if role == actor {
            let netmap = installed(&command(cli, "status").await?);
            terminate(context.daemon).await?;
            *context.daemon = spawn_daemon(cli, role, signals::underlay(cli).await?.controller)?;
            wait_ipc(cli, context.daemon).await?;
            let status = wait_status(cli, |status| {
                status.data_plane == DataPlane::Ready
                    && status.desired_up
                    && installed(status) == netmap
            })
            .await?;
            context.tun = owned_tun().await?;
            save(
                cli,
                &format!("scenario-reconnect-{stage}-restarted"),
                &status,
            )
            .await?;
            report(&format!("reconnect-restarted-{}", role.name()), now_ms()?).await?;
        }
        if role == Role::A {
            if actor == Role::R {
                let lost = until(async || direct_probe(cli).await, false, budget).await?;
                save(cli, "scenario-reconnect-restart-r-lost", &lost).await?;
            }
            let direct = until(async || direct_needed(cli, &pinged).await, true, budget).await?;
            save(
                cli,
                &format!("scenario-reconnect-{stage}-status"),
                &command(cli, "status").await?,
            )
            .await?;
            report(&format!("reconnect-direct-{stage}"), direct).await?;
        }
    }
    signals
        .wait_for("reconnect-done", Duration::from_secs(600))
        .await?;
    Ok(context.tun)
}

/// A Controller step signal under its exact name (routers read the same files).
async fn stepped(signals: &Signals, name: &str) -> Result<u64> {
    match signals.wait_for(name, Duration::from_secs(600)).await? {
        Signal::Step { at_ms, .. } => Ok(at_ms),
        _ => Err("invalid reconnect step".into()),
    }
}

async fn step(signals: &Signals, name: &str) -> Result<(Revision, u64)> {
    match signals
        .wait_for(&format!("scenario-{name}"), Duration::from_secs(600))
        .await?
    {
        Signal::Step { revision, at_ms } => Ok((revision, at_ms)),
        _ => Err("invalid scenario step".into()),
    }
}

/// Poll `probe` every 100 ms until it returns `wanted` twice in a row; the
/// first of the two is the observed time.
async fn until(
    mut probe: impl AsyncFnMut() -> Result<bool>,
    wanted: bool,
    budget: Duration,
) -> Result<u64> {
    let deadline = tokio::time::Instant::now() + budget;
    let mut first = None;
    loop {
        let at = now_ms()?;
        if probe().await? == wanted {
            match first {
                Some(first) => return Ok(first),
                None => first = Some(at),
            }
        } else {
            first = None;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("scenario effect not observed".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// New serials: a TCP port revoked then restored (UDP unaffected throughout),
/// B's key rotation, and B's membership revocation.
async fn revoke(mut context: Context<'_>) -> Result<String> {
    let Context {
        cli, role, signals, ..
    } = context;
    if let Some(traffic) = context.traffic.as_mut() {
        step(signals, "tcp-revoked").await?;
        let at = until(
            async || Ok(!traffic.tcp_connects(Duration::from_millis(400)).await?),
            true,
            STARTUP,
        )
        .await?;
        traffic.udp_only().await?;
        signals
            .write(
                "scenario-tcp-revoked-observed",
                &Signal::Observed { role, at_ms: at },
            )
            .await?;
        step(signals, "tcp-restored").await?;
        let at = until(
            async || traffic.tcp_connects(Duration::from_millis(400)).await,
            true,
            STARTUP,
        )
        .await?;
        signals
            .write(
                "scenario-tcp-restored-observed",
                &Signal::Observed { role, at_ms: at },
            )
            .await?;
        traffic.restored().await?;
        signals.ready("scenario-rotate").await?;
        step(signals, "rotated").await?;
        traffic.restored().await?;
        observed(signals, "scenario-rotated-observed", role).await?;
        step(signals, "member-revoked").await?;
        let at = until(
            async || Ok(!traffic.udp_echoes(Duration::from_millis(400)).await?),
            true,
            STARTUP,
        )
        .await?;
        traffic.blocked().await?;
        signals
            .write(
                "scenario-member-revoked-observed",
                &Signal::Observed { role, at_ms: at },
            )
            .await?;
    } else {
        // The restored configuration is the revision every Node runs now.
        let (revision, _) = step(signals, "tcp-restored").await?;
        signals.ready("scenario-rotate").await?;
        if role == Role::B {
            command(cli, "rotate-key").await?;
            let status = running(cli, revision).await?;
            save(cli, "scenario-rotated-status", &status).await?;
            signals
                .write(
                    "scenario-rotated",
                    &Signal::Step {
                        revision,
                        at_ms: now_ms()?,
                    },
                )
                .await?;
        }
        step(signals, "member-revoked").await?;
    }
    let status = if role == Role::B {
        // The revoked member stops; it never keeps a data plane.
        wait_status(cli, |status| {
            status.phase != Phase::Running && status.data_plane != DataPlane::Ready
        })
        .await?
    } else {
        command(cli, "status").await?
    };
    save(cli, "scenario-member-revoked-status", &status).await?;
    observed(signals, &format!("scenario-revoked-{}", role.name()), role).await?;
    if role == Role::B {
        context.tun = String::new();
    }
    Ok(context.tun)
}

/// Shared offline `down`: TUN and owned capture removed, identity kept.
pub(super) async fn down(
    cli: &Cli,
    role: Role,
    signals: &Signals,
    tun: &str,
    baseline: &Capture,
) -> Result<()> {
    signals.ready("offline").await?;
    command(cli, "down").await?;
    let status = wait_status(cli, |status| {
        status.phase == Phase::Down && !status.desired_up && status.data_plane == DataPlane::Stopped
    })
    .await?;
    save(cli, "down-status", &status).await?;
    if !tun.is_empty() && tokio::fs::try_exists(Path::new("/sys/class/net").join(tun)).await? {
        return Err("CLI down left TUN installed".into());
    }
    let after = capture().await?;
    save(cli, "capture-after", &after).await?;
    if &after != baseline {
        return Err("CLI down left owned capture routes/rules".into());
    }
    observed(signals, &format!("scenario-down-{}", role.name()), role).await
}
