use std::{path::PathBuf, time::Duration};

use clap::{Parser, Subcommand};
use meshway_agent::{Agent, ControlSyncReceipts, SyncReceipt, connections_status, status};

#[derive(Parser)]
#[command(
    version,
    about = "Meshway agent: persistent identity, authenticated heartbeats and saved control intent"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Isolated experimental carriers, static authorization and Linux TUN.
    #[cfg(feature = "transport-lab")]
    TransportLab {
        #[command(subcommand)]
        command: meshway_agent::transport_lab::Command,
    },
    /// Persist a proposed credential, then enroll (safe to retry the same binding).
    Enroll {
        #[arg(long)]
        controller: String,
        #[arg(long)]
        token: String,
        #[arg(long)]
        name: String,
        #[arg(long, default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
    /// Send one authenticated process heartbeat, recovering a pending ID if needed.
    Heartbeat {
        #[arg(long, default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
    /// Validate and save current control intent, then ACK its receipt (not applied networking).
    Sync {
        #[arg(long, default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
    /// Publish connection identity or save node-scoped connection intent (not applied networking).
    Connections {
        #[command(subcommand)]
        command: ConnectionsCommand,
    },
    /// Heartbeat and sync immediately, then periodically; retry transient failures with backoff.
    Run {
        #[arg(long, default_value = ".meshway/agent.json")]
        state: PathBuf,
        #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..=86400))]
        interval_seconds: u64,
        /// Also publish the connection key/capabilities and save a separate connections receipt.
        #[arg(long)]
        connections: bool,
        /// Apply freshly saved adjacency intent and authenticated neighbor probes.
        #[cfg(feature = "managed-connections")]
        #[arg(long)]
        apply_connections: bool,
        /// Private local TLS/VLESS profiles, resolved by credential_ref.
        #[cfg(feature = "managed-connections")]
        #[arg(long, requires = "apply_connections")]
        connection_profiles: Option<PathBuf>,
        /// Config freshness deadline, independent of HTTP/report success.
        #[cfg(feature = "managed-connections")]
        #[arg(long, default_value_t = 45, value_parser = clap::value_parser!(u64).range(10..=300))]
        connection_lease_seconds: u64,
    },
    /// Read local identity metadata without exposing credentials or claiming liveness.
    Status {
        #[arg(long, default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
}

#[derive(Subcommand)]
enum ConnectionsCommand {
    /// Durably create/reuse the private key, then bind its public key and compiled capabilities.
    Publish {
        #[arg(long, default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
    /// Publish identity, validate and save connection intent, then ACK saved (never applied).
    Sync {
        #[arg(long, default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
    /// Read local public key and saved connection revision without making network requests.
    Status {
        #[arg(long, default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
}

#[derive(serde::Serialize)]
#[serde(untagged)]
enum RunReceipt {
    Configuration(SyncReceipt),
    WithConnections(ControlSyncReceipts),
}

#[tokio::main]
async fn main() {
    if let Err(error) = execute(Cli::parse().command).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn execute(command: Command) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        #[cfg(feature = "transport-lab")]
        Command::TransportLab { command } => {
            meshway_agent::transport_lab::execute(command)
                .await
                .map_err(|error| -> Box<dyn std::error::Error> { error })?;
        }
        Command::Enroll {
            controller,
            token,
            name,
            state,
        } => {
            let mut agent = Agent::prepare_enrollment(state, &controller, &token, &name)?;
            let response = agent.enroll(&token).await?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        Command::Heartbeat { state } => {
            let response = Agent::load(state)?.heartbeat().await?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        Command::Sync { state } => {
            let receipt = Agent::load(state)?.sync().await?;
            println!("{}", serde_json::to_string_pretty(&receipt)?);
        }
        Command::Connections { command } => match command {
            ConnectionsCommand::Publish { state } => {
                let receipt = Agent::load(state)?.publish_connection_identity().await?;
                println!("{}", serde_json::to_string_pretty(&receipt)?);
            }
            ConnectionsCommand::Sync { state } => {
                let receipt = Agent::load(state)?.sync_connections().await?;
                println!("{}", serde_json::to_string_pretty(&receipt)?);
            }
            ConnectionsCommand::Status { state } => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&connections_status(state)?)?
                );
            }
        },
        Command::Status { state } => println!("{}", serde_json::to_string_pretty(&status(state)?)?),
        Command::Run {
            state,
            interval_seconds,
            connections,
            #[cfg(feature = "managed-connections")]
            apply_connections,
            #[cfg(feature = "managed-connections")]
            connection_profiles,
            #[cfg(feature = "managed-connections")]
            connection_lease_seconds,
        } => {
            let mut agent = Agent::load(state)?;
            #[cfg(feature = "managed-connections")]
            if apply_connections {
                agent
                    .run_managed_connections(
                        connection_profiles,
                        Duration::from_secs(interval_seconds),
                        Duration::from_secs(connection_lease_seconds),
                    )
                    .await?;
                return Ok(());
            }
            let mut backoff = 1_u64;
            loop {
                let result = tokio::select! {
                    result = async {
                        agent.heartbeat().await?;
                        if connections {
                            agent.sync_with_connections().await.map(RunReceipt::WithConnections)
                        } else {
                            agent.sync().await.map(RunReceipt::Configuration)
                        }
                    } => result,
                    _ = tokio::signal::ctrl_c() => return Ok(()),
                };
                let delay = match result {
                    Ok(response) => {
                        backoff = 1;
                        println!("{}", serde_json::to_string(&response)?);
                        interval_seconds
                    }
                    Err(error) if error.retryable() => {
                        let delay = backoff;
                        backoff = (backoff * 2).min(60);
                        eprintln!("{error}; retrying in {delay}s");
                        delay
                    }
                    Err(error) => return Err(error.into()),
                };
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(delay)) => {},
                    _ = tokio::signal::ctrl_c() => return Ok(()),
                }
            }
        }
    }
    Ok(())
}
