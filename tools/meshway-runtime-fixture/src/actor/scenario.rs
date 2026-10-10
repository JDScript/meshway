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
use std::{collections::BTreeMap, path::Path, time::Duration};
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
    match scenario {
        NetmapScenario::Offline => offline(context).await,
        NetmapScenario::Expiry => expiry(context).await,
        NetmapScenario::Restart => restart(context).await,
        NetmapScenario::Revoke => revoke(context).await,
    }
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
async fn fresh(context: &Context<'_>, stage: &str, ping: bool) -> Result<()> {
    let mut traffic =
        Traffic::new(&context.addresses[&Role::A], &context.addresses[&Role::B]).await?;
    traffic.first_udp().await?;
    traffic
        .open_remaining_flows(&context.addresses[&Role::A])
        .await?;
    traffic.exchange().await?;
    if ping {
        let path = crate::fallback::original_path(context.signals).await?;
        let run =
            crate::ping_cli::run(context.cli, crate::ping::Stage::Restored, "b", &path).await?;
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
                    fresh_flows == 0,
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

/// The Controller is unreachable past the lease: Engine withdraws every
/// component (no TUN, negative capture, traffic blocked), then a new lease
/// after reopening reinstalls the same content.
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
    if tokio::fs::try_exists(Path::new("/sys/class/net").join(&context.tun)).await? {
        return Err("lease end left the TUN installed".into());
    }
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

/// With reported endpoints the restart waits until every peer fact is past
/// its freshness bound (at most 60 s after the last report), so the restarted
/// daemons must dial from saved, stale last known facts alone.
const STALE_FACTS: Duration = Duration::from_secs(90);

/// Every daemon restarts while the Controller is unreachable. The signed
/// cached netmap installs at once; new flows cross all three restarted
/// daemons without any Server contact.
async fn restart(mut context: Context<'_>) -> Result<String> {
    let Context {
        cli, role, signals, ..
    } = context;
    closed(signals).await?;
    if cli.reported_endpoints {
        tokio::time::sleep(STALE_FACTS).await;
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
        fresh(&context, "restarted", true).await?;
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
