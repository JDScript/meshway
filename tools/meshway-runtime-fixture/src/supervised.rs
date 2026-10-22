//! File rendezvous of the rows a Python supervisor drives (`dev/roam-fixture`,
//! `dev/fault-fixture`): the supervisor owns the Docker topology and the
//! perturbations; these actors only run the real Server and daemons. Every
//! signal of a row is a file `<row>-<name>` in the shared signal directory.
use crate::{Cli, Result, actor};
use serde::{Serialize, de::DeserializeOwned};
use std::{path::Path, time::Duration};
use tokio::process::Child;

pub async fn write<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let temporary = path.with_extension("tmp");
    tokio::fs::write(&temporary, serde_json::to_vec_pretty(value)?).await?;
    tokio::fs::rename(temporary, path).await?;
    Ok(())
}

pub async fn read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&tokio::fs::read(path).await?)?)
}

/// The supervisor ended the row (`<row>-stop`).
pub async fn stopped(cli: &Cli, row: &str) -> Result<bool> {
    Ok(tokio::fs::try_exists(cli.signals.join(format!("{row}-stop"))).await?)
}

pub async fn wait(cli: &Cli, row: &str, name: &str) -> Result<()> {
    tokio::time::timeout(crate::remaining(Duration::from_secs(600))?, async {
        while !tokio::fs::try_exists(cli.signals.join(name)).await? {
            if stopped(cli, row).await? {
                return Err(format!("{row} supervisor stopped the run").into());
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

/// `up` on the spawned daemon; its enrollment request and then its identity
/// are handed to the row's controller (`<row>-enrollment-<name>.json`,
/// `<row>-enrolled-<name>.json`), which approves it.
pub async fn enroll(cli: &Cli, row: &str, name: &str, daemon: &mut Child) -> Result<()> {
    actor::wait_ipc(cli, daemon).await?;
    actor::command(cli, "up").await?;
    let status = actor::wait_status(cli, |s| s.enrollment_request_id.is_some()).await?;
    if daemon.id() != Some(status.process_id) {
        return Err("wrong daemon process".into());
    }
    write(
        &cli.signals.join(format!("{row}-enrollment-{name}.json")),
        &status,
    )
    .await?;
    let status = actor::wait_status(cli, |s| s.identity.is_some()).await?;
    write(
        &cli.signals.join(format!("{row}-enrolled-{name}.json")),
        &status,
    )
    .await
}
