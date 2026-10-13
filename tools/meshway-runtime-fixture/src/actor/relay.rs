//! nat/relay's additional public relays. Each runs the real daemon and CLI
//! only once the relay scenario enrolls it, after the shared startup and
//! traffic of A, R and B, and carries no business of its own.
use super::{command, running, save, spawn_named_daemon, terminate, wait_ipc, wait_status};
use crate::{
    Cli, Result, STARTUP,
    signals::{self, Signal, Signals},
};
use std::time::Duration;

/// The extra relays' names, also their hostnames' suffix and signal names.
pub const NAMES: [&str; 2] = ["x", "y"];

pub async fn run(cli: &Cli, name: &str) -> Result<()> {
    if !NAMES.contains(&name) {
        return Err("unknown extra relay".into());
    }
    let signals = Signals::new(&cli.signals);
    signals.ready("enroll").await?;
    let underlay = signals::underlay(cli).await?;
    if !matches!(
        signals
            .wait_for("relay-extra-enroll", Duration::from_secs(1500))
            .await?,
        Signal::Ready
    ) {
        return Err("invalid extra relay enrollment rendezvous".into());
    }
    let hostname = format!("relay-{name}");
    let mut daemon = spawn_named_daemon(cli, &hostname, underlay.controller)?;
    let result = serve(cli, &signals, name, &mut daemon).await;
    if let Err(error) = &result {
        crate::record_error(cli, "failure.txt", error.as_ref());
        let _ = signals
            .write(&format!("abort-relay-{name}"), &Signal::Ready)
            .await;
        if let Ok(status) = command(cli, "status").await {
            let _ = save(cli, "failure-status", &status).await;
        }
    }
    let down = command(cli, "down").await;
    let stopped = terminate(&mut daemon).await;
    result?;
    down?;
    stopped?;
    Ok(())
}

async fn serve(
    cli: &Cli,
    signals: &Signals,
    name: &str,
    daemon: &mut tokio::process::Child,
) -> Result<()> {
    wait_ipc(cli, daemon).await?;
    command(cli, "up").await?;
    let status = wait_status(cli, |status| status.enrollment_request_id.is_some()).await?;
    if daemon.id() != Some(status.process_id) {
        return Err("CLI status did not come from the spawned daemon".into());
    }
    let relay = |status| Signal::Relay {
        name: name.to_owned(),
        status: Box::new(status),
    };
    signals
        .write(&format!("relay-enrollment-{name}"), &relay(status))
        .await?;
    let status = wait_status(cli, |status| status.identity.is_some()).await?;
    signals
        .write(&format!("relay-enrolled-{name}"), &relay(status))
        .await?;
    let Signal::Revision { revision } = signals.wait("relay-extra-config").await? else {
        return Err("invalid extra relay configuration rendezvous".into());
    };
    let status = running(cli, revision).await?;
    save(cli, "relay-running-status", &status).await?;
    signals
        .write(&format!("relay-running-{name}"), &Signal::Ready)
        .await?;
    if !matches!(
        signals.wait_for("relay-extra-stop", STARTUP * 10).await?,
        Signal::Ready
    ) {
        return Err("invalid extra relay stop rendezvous".into());
    }
    save(cli, "relay-final-status", &command(cli, "status").await?).await
}
