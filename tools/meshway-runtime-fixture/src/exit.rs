//! Exit infrastructure rendezvous. No alternate permit issuer, flow table or NAT.
use crate::{Cli, IO, Result};
use serde::Serialize;
use std::{
    net::SocketAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

// The packet fixture's A/R/B Role is owned by another crate. Keep the new
// topology explicit here rather than pretending its B is an executable exit.
#[derive(Clone, Copy, clap::ValueEnum, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Controller,
    A,
    R,
    E,
    External,
    Lan,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Stage {
    Install,
    Ipv4NatAndReply,
    Denials,
    Withdrawal,
    ExpiryAndCancellation,
    HostFailures,
    CaptureAndBypass,
    OfflineCleanup,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingStage {
    number: usize,
    stage: Stage,
    status: &'static str,
}

#[derive(Serialize)]
struct Report {
    role: Role,
    status: &'static str,
    stages: Vec<PendingStage>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Visit {
    peer: SocketAddr,
    local: SocketAddr,
    received_at_unix_ms: u64,
}

pub async fn run(cli: &Cli, role: Role) -> Result<()> {
    if std::env::var("MESHWAY_EXIT_FIXTURE_OWNED").as_deref() != Ok("1") {
        return Err("exit preparation requires an owned fixture namespace".into());
    }
    let listener = if matches!(role, Role::External | Role::Lan) {
        Some(TcpListener::bind("0.0.0.0:8080").await?)
    } else {
        None
    };
    let report = Report {
        role,
        status: "unsupported",
        stages: [
            Stage::Install,
            Stage::Ipv4NatAndReply,
            Stage::Denials,
            Stage::Withdrawal,
            Stage::ExpiryAndCancellation,
            Stage::HostFailures,
            Stage::CaptureAndBypass,
            Stage::OfflineCleanup,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, stage)| PendingStage {
            number: index + 1,
            stage,
            status: "pending-x2-x5",
        })
        .collect(),
    };
    tokio::fs::write(
        cli.evidence.join("exit-stages.json"),
        serde_json::to_vec_pretty(&report)?,
    )
    .await?;
    tokio::fs::write(cli.evidence.join("ready"), b"infrastructure-only\n").await?;
    tracing::info!(
        phase = "exit-infrastructure-ready",
        "product stages pending-x2-x5"
    );
    let result = tokio::time::timeout(Duration::from_secs(180), async {
        let mut visits = Vec::<Visit>::new();
        loop {
            if tokio::fs::try_exists(cli.signals.join("exit-stop")).await? {
                break;
            }
            if let Some(listener) = &listener {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut stream, peer) = accepted?;
                        let mut request = [0; 4096];
                        let count = tokio::time::timeout(IO, stream.read(&mut request)).await??;
                        if count == 0 {
                            continue;
                        }
                        if visits.len() >= 256 {
                            return Err("external fixture access-log bound exceeded".into());
                        }
                        visits.push(Visit {
                            peer,
                            local: stream.local_addr()?,
                            received_at_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().try_into()?,
                        });
                        tokio::fs::write(cli.evidence.join("access.json"), serde_json::to_vec_pretty(&visits)?).await?;
                        tokio::time::timeout(IO, stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nok\n")).await??;
                        stream.shutdown().await?;
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
            } else {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        Ok::<(), crate::Error>(())
    })
    .await;
    result??;
    tracing::info!(phase = "unsupported", "exit product stages pending-x2-x5");
    Err("unsupported: exit product stages pending-x2-x5".into())
}
