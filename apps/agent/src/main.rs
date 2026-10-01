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
    /// Fresh authenticated direct-neighbor probe through the active managed daemon (not ICMP or mesh trace).
    #[cfg(all(feature = "managed-connections", unix))]
    Ping {
        /// Exact inventory name or full 64-character node ID; no automatic routing.
        peer: String,
        #[arg(long, env = "MESHWAY_STATE", default_value = ".meshway/agent.json")]
        state: PathBuf,
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=20))]
        count: u8,
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u64).range(1..=10))]
        timeout_seconds: u64,
    },
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
        #[arg(long, env = "MESHWAY_STATE", default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
    /// Send one authenticated process heartbeat, recovering a pending ID if needed.
    Heartbeat {
        #[arg(long, env = "MESHWAY_STATE", default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
    /// Validate and save current control intent, then ACK its receipt (not applied networking).
    Sync {
        #[arg(long, env = "MESHWAY_STATE", default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
    /// Publish connection identity or save node-scoped connection intent (not applied networking).
    Connections {
        #[command(subcommand)]
        command: ConnectionsCommand,
    },
    /// Heartbeat and sync immediately, then periodically; retry transient failures with backoff.
    Run {
        #[arg(long, env = "MESHWAY_STATE", default_value = ".meshway/agent.json")]
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
        /// Apply coherent directional mesh IPv4 policy through an owned Linux TUN (also manages adjacencies).
        #[cfg(feature = "managed-connections")]
        #[arg(long)]
        apply_dataplane: bool,
        #[cfg(feature = "managed-connections")]
        #[arg(long, default_value = "meshway0", requires = "apply_dataplane")]
        tun_name: String,
        #[cfg(feature = "managed-connections")]
        #[arg(long, default_value_t = 1000, requires = "apply_dataplane", value_parser = clap::value_parser!(u16).range(576..=1400))]
        tun_mtu: u16,
        /// Private local TLS/VLESS profiles, resolved by credential_ref.
        #[cfg(feature = "managed-connections")]
        #[arg(long)]
        connection_profiles: Option<PathBuf>,
        /// Config freshness deadline, independent of HTTP/report success.
        #[cfg(feature = "managed-connections")]
        #[arg(long, default_value_t = 45, value_parser = clap::value_parser!(u64).range(10..=300))]
        connection_lease_seconds: u64,
    },
    /// Read local identity metadata without exposing credentials or claiming liveness.
    Status {
        #[arg(long, env = "MESHWAY_STATE", default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
}

#[derive(Subcommand)]
enum ConnectionsCommand {
    /// Durably create/reuse the private key, then bind its public key and compiled capabilities.
    Publish {
        #[arg(long, env = "MESHWAY_STATE", default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
    /// Publish identity, validate and save connection intent, then ACK saved (never applied).
    Sync {
        #[arg(long, env = "MESHWAY_STATE", default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
    /// Read local public key and saved connection revision without making network requests.
    Status {
        #[arg(long, env = "MESHWAY_STATE", default_value = ".meshway/agent.json")]
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
        #[cfg(all(feature = "managed-connections", unix))]
        Command::Ping {
            peer,
            state,
            count,
            timeout_seconds,
        } => {
            println!("authenticated neighbor probe / 认证邻接探测 (非ICMP，非mesh trace)");
            for sequence in 1..=count {
                let observation = meshway_agent::diagnostics::probe(
                    &state,
                    &peer,
                    Duration::from_secs(timeout_seconds),
                )
                .await?;
                let direction = if observation.initiator_node_id == observation.local_node_id {
                    "outgoing (locally initiated)"
                } else {
                    "accepted (peer initiated)"
                };
                let transport = match observation.transport {
                    meshway_protocol::ConnectionTransport::NativeUdp => "native_udp",
                    meshway_protocol::ConnectionTransport::NativeTcp => "native_tcp",
                    meshway_protocol::ConnectionTransport::Vless => "vless (TLS)",
                };
                println!(
                    "seq={sequence} peer={} name={} direction={direction} transport={transport} initiator={} RTT={}.{:03} µs",
                    observation.peer_node_id,
                    // JSON quoting escapes inventory-controlled terminal characters.
                    serde_json::to_string(&observation.peer_name)?,
                    observation.initiator_node_id,
                    observation.rtt_ns / 1000,
                    observation.rtt_ns % 1000,
                );
                if sequence < count {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
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
            apply_dataplane,
            #[cfg(feature = "managed-connections")]
            tun_name,
            #[cfg(feature = "managed-connections")]
            tun_mtu,
            #[cfg(feature = "managed-connections")]
            connection_profiles,
            #[cfg(feature = "managed-connections")]
            connection_lease_seconds,
        } => {
            let mut agent = Agent::load(state)?;
            #[cfg(feature = "managed-connections")]
            if apply_dataplane {
                agent
                    .run_managed_dataplane(
                        connection_profiles,
                        Duration::from_secs(interval_seconds),
                        Duration::from_secs(connection_lease_seconds),
                        meshway_agent::TunOptions {
                            name: tun_name,
                            mtu: tun_mtu,
                        },
                    )
                    .await?;
                return Ok(());
            }
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
            #[cfg(feature = "managed-connections")]
            if connection_profiles.is_some() {
                return Err(
                    "--connection-profiles requires --apply-connections or --apply-dataplane"
                        .into(),
                );
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

#[cfg(all(test, feature = "managed-connections"))]
mod tests {
    use super::*;

    #[test]
    fn packet_application_is_explicit_and_adjacency_only_does_not_require_it() {
        let cli = Cli::try_parse_from(["meshway-agent", "run", "--apply-connections"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Run {
                apply_connections: true,
                apply_dataplane: false,
                ..
            }
        ));
        let cli = Cli::try_parse_from(["meshway-agent", "run", "--apply-dataplane"]).unwrap();
        assert!(
            matches!(cli.command, Command::Run { apply_dataplane: true, tun_mtu: 1000, ref tun_name, .. } if tun_name == "meshway0")
        );
        for args in [
            vec!["meshway-agent", "run", "--tun-name", "meshway1"],
            vec!["meshway-agent", "run", "--tun-mtu", "1000"],
            vec![
                "meshway-agent",
                "run",
                "--apply-dataplane",
                "--tun-mtu",
                "575",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
        assert!(
            Cli::try_parse_from([
                "meshway-agent",
                "run",
                "--apply-dataplane",
                "--tun-mtu",
                "576"
            ])
            .is_ok()
        );
    }
}
