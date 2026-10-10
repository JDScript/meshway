//! Real daemon/CLI conformance. Actors never instantiate control or dataplane owners.
#![forbid(unsafe_code)]

mod actor;
mod application;
mod control_requests;
mod controller;
mod diagnostics;
mod dns;
mod exit;
mod fallback;
mod peer;
mod ping;
mod ping_cli;
mod ping_management;
mod recovery;
mod signals;
mod traversal;

use clap::{Parser, Subcommand, ValueEnum};
use signals::Role;
use std::{path::PathBuf, time::Duration};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;
const IO: Duration = Duration::from_secs(10);
const STARTUP: Duration = Duration::from_secs(45);
// Bounded diagnostic admission/lifecycle checks precede the packet window
// (125s acceptance / 30s diagnostic) and actual instance-expiry/recovery
// sequence, or a netmap scenario's (up to 10-minute) Server outage.
const DEADLINE: Duration = Duration::from_secs(1800);
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
    #[arg(long, global = true, value_enum, default_value = "off")]
    traversal: TraversalMode,
    #[arg(long, global = true)]
    traversal_fallback: bool,
    /// UDP/off fixture with no configured endpoints and no STUN service.
    #[arg(long, global = true)]
    reported_endpoints: bool,
    /// Explicit Server lease for expiry conformance; production defaults to 24h.
    #[arg(long, global = true, default_value = "60s")]
    netmap_validity: meshway_transport::spec::PositiveDuration,
    /// Diagnostic traffic window only; does not establish 125-second renewal acceptance.
    #[arg(long, global = true)]
    short: bool,
    /// One netmap lifecycle row after the shared startup and persistent traffic,
    /// instead of the main revocation/outage/recovery sequence.
    #[arg(long, global = true, value_enum)]
    netmap_scenario: Option<NetmapScenario>,
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    fn traffic_duration(&self) -> Duration {
        Duration::from_secs(if self.short { 30 } else { 125 })
    }

    fn fallback_delay(&self) -> Duration {
        Duration::from_secs(if self.short { 5 } else { 30 })
    }

    fn traffic_wait(&self) -> Duration {
        self.traffic_duration() + Duration::from_secs(125)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum NetmapScenario {
    /// Controller unreachable well inside the lease: everything keeps working.
    Offline,
    /// New serials (port, key, membership) reach the data plane within 5 s.
    Revoke,
    /// Controller unreachable past the lease: fail closed, then recover.
    Expiry,
    /// Daemon restarts with the Controller unreachable: cached netmap serves.
    Restart,
}

impl NetmapScenario {
    fn name(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Revoke => "revoke",
            Self::Expiry => "expiry",
            Self::Restart => "restart",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum TraversalMode {
    Off,
    Nat,
    Exit,
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
    Vless,
    Trojan,
}

impl CarrierMode {
    fn protocol(self) -> meshway_model::TransportProtocol {
        match self {
            Self::NativeTcp => meshway_model::TransportProtocol::NativeTcp,
            Self::NativeUdp => meshway_model::TransportProtocol::NativeUdp,
            Self::Vless => meshway_model::TransportProtocol::Vless,
            Self::Trojan => meshway_model::TransportProtocol::Trojan,
        }
    }

    fn streaming(self) -> bool {
        matches!(self, Self::Vless | Self::Trojan)
    }
}

#[derive(Subcommand)]
enum Command {
    /// Isolated exit Server, real daemon, or external application service.
    ExitActor {
        #[arg(long, value_enum)]
        role: exit::Role,
    },
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
    if (cli.traversal == TraversalMode::Exit) != matches!(cli.command, Command::ExitActor { .. })
        || (cli.traversal == TraversalMode::Exit && cli.dns_backend != DnsBackend::Off)
    {
        eprintln!("--traversal exit requires exit-actor and --dns-backend off");
        return std::process::ExitCode::FAILURE;
    }
    if cli.reported_endpoints
        && (cli.traversal != TraversalMode::Off
            || !matches!(cli.transport, CarrierMode::NativeUdp)
            || cli.dns_backend != DnsBackend::Off)
    {
        eprintln!("--reported-endpoints requires native-udp with traversal and DNS off");
        return std::process::ExitCode::FAILURE;
    }
    if cli.traversal_fallback && cli.traversal != TraversalMode::Nat {
        eprintln!("--traversal-fallback requires --traversal nat");
        return std::process::ExitCode::FAILURE;
    }
    if cli.traversal == TraversalMode::Nat && !matches!(cli.transport, CarrierMode::NativeUdp) {
        eprintln!("--traversal nat requires --transport native-udp");
        return std::process::ExitCode::FAILURE;
    }
    if cli.netmap_scenario.is_some()
        && (cli.traversal != TraversalMode::Off || cli.dns_backend != DnsBackend::Off)
    {
        eprintln!("--netmap-scenario requires traversal and DNS off");
        return std::process::ExitCode::FAILURE;
    }
    if cli.logging.init().is_err() {
        eprintln!("invalid runtime fixture logging configuration");
        return std::process::ExitCode::FAILURE;
    }
    let result = match cli.command {
        Command::ExitActor { role } => exit::run(&cli, role).await,
        Command::Prepare => prepare(&cli).await,
        Command::Controller => controller::run(&cli).await,
        Command::Actor { role } => actor::run(&cli, role).await,
        Command::UnderlayPeer { ipv4, ipv6 } => peer::run(&cli, ipv4, ipv6).await,
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            // Authenticated errors can contain secrets; ordinary logs use only
            // typed/static failure metadata, never arbitrary Display messages.
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
            diagnostics::log(error.as_ref());
            std::process::ExitCode::FAILURE
        }
    }
}

async fn prepare(cli: &Cli) -> Result<()> {
    meshway_packet_fixture::prepare_tls(&cli.public, &cli.private).await?;
    if cli.transport.streaming() {
        meshway_packet_fixture::prepare_tls(
            &cli.public.join("carrier"),
            &cli.private.join("carrier"),
        )
        .await?;
    }
    Ok(())
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
