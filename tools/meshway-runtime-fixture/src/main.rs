//! Real daemon/CLI conformance. Actors never instantiate control or dataplane owners.
#![forbid(unsafe_code)]

mod actor;
mod application;
mod controller;
mod dns;
mod peer;
mod signals;

use clap::{Parser, Subcommand, ValueEnum};
use signals::Role;
use std::{path::PathBuf, time::Duration};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;
const IO: Duration = Duration::from_secs(10);
const STARTUP: Duration = Duration::from_secs(45);
const DEADLINE: Duration = Duration::from_secs(420);
static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

fn remaining(limit: Duration) -> Result<Duration> {
    let elapsed = STARTED.get_or_init(std::time::Instant::now).elapsed();
    // Leave time for CLI down, explicit child termination and joins.
    let remaining = (DEADLINE - Duration::from_secs(30))
        .checked_sub(elapsed)
        .ok_or("fixture deadline exceeded")?;
    Ok(limit.min(remaining))
}

#[derive(Parser)]
struct Cli {
    #[command(flatten)]
    logging: meshway_logging::Options,
    #[arg(long, global = true, default_value = "/public")]
    public: PathBuf,
    #[arg(long, global = true, default_value = "/controller")]
    private: PathBuf,
    #[arg(long, global = true, default_value = "/signals")]
    signals: PathBuf,
    #[arg(long, global = true, default_value = "/state")]
    state: PathBuf,
    #[arg(long, global = true, default_value = "/evidence")]
    evidence: PathBuf,
    #[arg(long, global = true, default_value = "/fixtures")]
    binaries: PathBuf,
    #[arg(long, global = true, value_enum, default_value = "native-tcp")]
    transport: CarrierMode,
    #[arg(long, global = true, value_enum, default_value = "off")]
    dns_backend: DnsBackend,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum DnsBackend {
    Off,
    ResolvConf,
    Resolved,
}

impl DnsBackend {
    fn enabled(self) -> bool {
        self != Self::Off
    }
    fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::ResolvConf => "resolv-conf",
            Self::Resolved => "resolved",
        }
    }
    fn bind(self) -> &'static str {
        match self {
            Self::Resolved => "127.0.0.1:5354",
            _ => "127.0.0.1:53",
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum CarrierMode {
    NativeTcp,
    NativeUdp,
}

impl CarrierMode {
    fn protocol(self) -> meshway_transport_spec::TransportProtocol {
        match self {
            Self::NativeTcp => meshway_transport_spec::TransportProtocol::NativeTcp,
            Self::NativeUdp => meshway_transport_spec::TransportProtocol::NativeUdp,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    Prepare,
    Controller,
    Actor {
        #[arg(long, value_enum)]
        role: Role,
    },
    UnderlayPeer {
        #[arg(long)]
        ipv4: std::net::Ipv4Addr,
        #[arg(long)]
        ipv6: std::net::Ipv6Addr,
    },
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    STARTED.get_or_init(std::time::Instant::now);
    let cli = Cli::parse();
    if cli.logging.init().is_err() {
        eprintln!("invalid runtime fixture logging configuration");
        return std::process::ExitCode::FAILURE;
    }
    let result = match cli.command {
        Command::Prepare => meshway_packet_fixture::prepare_tls(&cli.public, &cli.private).await,
        Command::Controller => controller::run(&cli).await,
        Command::Actor { role } => actor::run(&cli, role).await,
        Command::UnderlayPeer { ipv4, ipv6 } => peer::run(&cli, ipv4, ipv6).await,
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            // Authenticated errors can contain secrets; only static stages are logged.
            record_error(&cli, "error.txt", error.as_ref());
            let role = match cli.command {
                Command::Controller => Some("controller"),
                Command::Actor { role } => Some(role.name()),
                _ => None,
            };
            if let Some(role) = role {
                let _ = signals::Signals::new(&cli.signals)
                    .write(&format!("abort-{role}"), &signals::Signal::Ready)
                    .await;
            }
            tracing::error!("runtime fixture failed; inspect last stage and private evidence");
            std::process::ExitCode::FAILURE
        }
    }
}

fn request() -> Result<meshway_model::RequestId> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| "request entropy unavailable")?;
    Ok(meshway_model::RequestId::new(
        uuid::Builder::from_random_bytes(bytes).into_uuid(),
    )?)
}

fn record_error(cli: &Cli, name: &str, error: &dyn std::error::Error) {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(cli.evidence.join(name))
    {
        let message: String = error.to_string().chars().take(4096).collect();
        let _ = writeln!(file, "{message}");
    }
}
