use std::{path::PathBuf, time::Duration};

#[cfg(feature = "managed-connections")]
use clap::CommandFactory;
use clap::{Parser, Subcommand};
use meshway_agent::{Agent, ControlSyncReceipts, SyncReceipt, connections_status, status};

mod cli_netcheck;

#[cfg(all(feature = "managed-connections", unix))]
mod cli_dns;
#[cfg(all(feature = "managed-connections", unix))]
mod cli_live;

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
    /// Measure explicit STUN observers using an independent temporary UDP socket.
    Netcheck(cli_netcheck::Args),
    /// Observe current daemon DNS metadata or perform a real loopback DNS query.
    #[cfg(all(feature = "managed-connections", unix))]
    Dns {
        #[command(subcommand)]
        command: cli_dns::Command,
    },
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
        #[arg(long)]
        json: bool,
    },
    /// Resolve public identity from the managed daemon's current trusted inventory (not DNS).
    #[cfg(all(feature = "managed-connections", unix))]
    Whois {
        /// Exact name, full 64-character node ID, or mesh IPv4/IPv6 address.
        query: String,
        #[arg(long, env = "MESHWAY_STATE", default_value = ".meshway/agent.json")]
        state: PathBuf,
        #[arg(long)]
        json: bool,
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
    #[cfg_attr(feature = "managed-connections", command(group(clap::ArgGroup::new("managed_apply").args(["apply_connections", "apply_dataplane"]).multiple(true))))]
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
        /// Serve controller-assigned node names on explicit unprivileged loopback UDP+TCP. Does not change OS DNS.
        #[cfg(feature = "managed-connections")]
        #[arg(long, value_parser = parse_dns_listen)]
        dns_listen: Option<std::net::SocketAddr>,
        /// Observe actual managed UDP sockets; repeat up to four numeric unicast observers.
        #[cfg(feature = "managed-connections")]
        #[arg(long, requires = "managed_apply", value_parser = meshway_agent::parse_stun_server)]
        stun_server: Vec<std::net::SocketAddr>,
    },
    /// Read local identity metadata without exposing credentials or claiming liveness.
    Status {
        #[arg(long, env = "MESHWAY_STATE", default_value = ".meshway/agent.json")]
        state: PathBuf,
        /// Read the current managed daemon through its private control socket.
        #[arg(long)]
        live: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, requires = "live")]
        peers: bool,
        #[arg(long, requires = "peers", default_value_t = 0)]
        peer_offset: u32,
        #[arg(long, requires = "peers", default_value_t = 64, value_parser = clap::value_parser!(u32).range(1..=64))]
        peer_limit: u32,
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
        Command::Netcheck(args) => cli_netcheck::execute(args).await?,
        #[cfg(all(feature = "managed-connections", unix))]
        Command::Dns { command } => cli_dns::execute(command).await?,
        #[cfg(all(feature = "managed-connections", unix))]
        Command::Ping {
            peer,
            state,
            count,
            timeout_seconds,
            json,
        } => {
            cli_live::ping(&state, &peer, count, timeout_seconds, json).await?;
        }
        #[cfg(all(feature = "managed-connections", unix))]
        Command::Whois { query, state, json } => cli_live::whois(&state, &query, json).await?,
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
        Command::Status {
            state,
            live,
            json,
            peers,
            peer_offset,
            peer_limit,
        } => {
            if live {
                #[cfg(all(feature = "managed-connections", unix))]
                cli_live::status(&state, json, peers, peer_offset, peer_limit).await?;
                #[cfg(not(all(feature = "managed-connections", unix)))]
                {
                    let _ = (json, peers, peer_offset, peer_limit);
                    return Err("live status requires a Unix managed-connections build and a running --apply-connections/--apply-dataplane daemon".into());
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&status(state)?)?);
            }
        }
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
            #[cfg(feature = "managed-connections")]
            dns_listen,
            #[cfg(feature = "managed-connections")]
            stun_server,
        } => {
            #[cfg(feature = "managed-connections")]
            let discovery = {
                if !stun_server.is_empty() && !apply_connections && !apply_dataplane {
                    Cli::command()
                        .error(
                            clap::error::ErrorKind::ArgumentConflict,
                            "--stun-server requires --apply-connections or --apply-dataplane",
                        )
                        .exit();
                }
                meshway_agent::EndpointDiscoveryOptions::new(stun_server).unwrap_or_else(|error| {
                    Cli::command()
                        .error(clap::error::ErrorKind::ValueValidation, error)
                        .exit()
                })
            };
            let mut agent = Agent::load(state)?;
            #[cfg(feature = "managed-connections")]
            let mut dns_service = if let Some(listen) = dns_listen {
                Some(
                    agent
                        .dns_service(
                            listen,
                            Duration::from_secs(interval_seconds),
                            Duration::from_secs(connection_lease_seconds),
                        )
                        .await?,
                )
            } else {
                None
            };
            #[cfg(all(feature = "managed-connections", unix))]
            if !apply_connections
                && !apply_dataplane
                && let Some(service) = &mut dns_service
            {
                service.enable_diagnostics(&agent).await?;
            }
            let run = async {
                #[cfg(feature = "managed-connections")]
                if apply_dataplane {
                    agent
                        .run_managed_dataplane_with_discovery(
                            connection_profiles,
                            Duration::from_secs(interval_seconds),
                            Duration::from_secs(connection_lease_seconds),
                            meshway_agent::TunOptions {
                                name: tun_name,
                                mtu: tun_mtu,
                            },
                            discovery,
                        )
                        .await?;
                    return Ok::<(), Box<dyn std::error::Error>>(());
                }
                #[cfg(feature = "managed-connections")]
                if apply_connections {
                    agent
                        .run_managed_connections_with_discovery(
                            connection_profiles,
                            Duration::from_secs(interval_seconds),
                            Duration::from_secs(connection_lease_seconds),
                            discovery,
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
                        _ = tokio::signal::ctrl_c() => return Ok::<(), Box<dyn std::error::Error>>(()),
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
            };
            #[cfg(feature = "managed-connections")]
            if let Some(service) = dns_service {
                tokio::select! {
                    result = run => result?,
                    result = service.run() => result?,
                }
            } else {
                run.await?;
            }
            #[cfg(not(feature = "managed-connections"))]
            run.await?;
        }
    }
    Ok(())
}

#[cfg(feature = "managed-connections")]
fn parse_dns_listen(value: &str) -> Result<std::net::SocketAddr, String> {
    let address: std::net::SocketAddr = value
        .parse()
        .map_err(|_| "DNS listen requires an IP:port socket address".to_owned())?;
    if !address.ip().is_loopback() || address.port() < 1024 {
        return Err("DNS listen requires a loopback IP and port >= 1024".to_owned());
    }
    Ok(address)
}

#[cfg(all(test, feature = "managed-connections"))]
mod tests {
    use super::*;

    #[test]
    fn stun_is_opt_in_numeric_and_requires_managed_application() {
        let cli = Cli::try_parse_from(["meshway-agent", "run", "--apply-connections"]).unwrap();
        assert!(matches!(cli.command, Command::Run { stun_server, .. } if stun_server.is_empty()));
        assert!(
            Cli::try_parse_from(["meshway-agent", "run", "--stun-server", "127.0.0.1:3478"])
                .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "meshway-agent",
                "run",
                "--apply-connections",
                "--stun-server",
                "stun.example:3478"
            ])
            .is_err()
        );
        let cli = Cli::try_parse_from([
            "meshway-agent",
            "run",
            "--apply-connections",
            "--dns-listen",
            "127.0.0.1:5353",
            "--stun-server",
            "127.0.0.1:3478",
            "--stun-server",
            "[::1]:3478",
        ])
        .unwrap();
        assert!(
            matches!(cli.command, Command::Run { stun_server, dns_listen: Some(_), .. } if stun_server.len() == 2)
        );
    }

    #[test]
    fn dns_is_explicit_loopback_and_can_share_dataplane_daemon() {
        let cli = Cli::try_parse_from(["meshway-agent", "run"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Run {
                dns_listen: None,
                ..
            }
        ));
        for ip in ["127.0.0.1:5353", "[::1]:5353"] {
            let cli = Cli::try_parse_from([
                "meshway-agent",
                "run",
                "--apply-dataplane",
                "--dns-listen",
                ip,
            ])
            .unwrap();
            assert!(matches!(
                cli.command,
                Command::Run {
                    apply_dataplane: true,
                    dns_listen: Some(_),
                    ..
                }
            ));
        }
        for ip in [
            "0.0.0.0:5353",
            "[::]:5353",
            "192.0.2.1:5353",
            "127.0.0.1:53",
            "localhost:5353",
        ] {
            assert!(Cli::try_parse_from(["meshway-agent", "run", "--dns-listen", ip]).is_err());
        }
    }

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
