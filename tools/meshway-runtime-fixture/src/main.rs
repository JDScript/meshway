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
mod fault;
mod peer;
mod ping;
mod ping_cli;
mod ping_management;
mod recovery;
mod roam;
mod signals;
mod supervised;
mod traversal;

use clap::{Parser, Subcommand, ValueEnum};
use signals::Role;
use std::{path::PathBuf, time::Duration};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;
const IO: Duration = Duration::from_secs(10);
const STARTUP: Duration = Duration::from_secs(45);
// Bounded diagnostic admission/lifecycle checks precede the packet window
// (70 s acceptance / 30 s diagnostic) and actual instance-expiry/recovery
// sequence, or a netmap scenario's (up to 3-minute) Server outage.
/// Acceptance traffic window, anchored to netmap constants: longer than one
/// sync renewal cycle (50 s hold plus margin) and than one 60 s lease
/// validity (`--netmap-validity`), so the same sockets cross a renewal and
/// the end of the lease they started under.
const TRAFFIC_SECONDS: u64 = 70;
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
    /// Diagnostic traffic window only; does not establish the 70-second renewal acceptance.
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
    /// `exit/off` or `exit6/off`: the exit topology and its actors.
    fn exit_fixture(&self) -> bool {
        matches!(self.traversal, TraversalMode::Exit | TraversalMode::Exit6)
    }

    /// `nat/off`: A–R is an on-demand NAT–NAT edge. R can also dial A
    /// (Native TCP to A's unpublished TCP listener, unreachable through A's
    /// NAT), so neither end holds the edge and only A has business toward R;
    /// R, a relay with no need of its own, takes part through the Server's
    /// traversal request in its sync answer.
    fn on_demand_nat(&self) -> bool {
        self.traversal == TraversalMode::Nat
            && !self.traversal_fallback
            && self.dns_backend == DnsBackend::Off
    }

    /// `udp/off`: another socket holds B's listener port while its daemon
    /// first binds it; the Node reports the listener unavailable, retries,
    /// and serves it once the port is free, with no restart.
    fn held_listener_port(&self) -> bool {
        self.reported_endpoints && self.netmap_scenario.is_none()
    }

    /// `udp/off`: A–R also has a Native TCP carrier after Native UDP. A
    /// blocks its outgoing UDP until A–R is on TCP, then lifts it; A–R must
    /// return to Native UDP (CM-1b), keeping the circuit.
    fn carrier_upgrade(&self) -> bool {
        self.held_listener_port()
    }

    fn traffic_duration(&self) -> Duration {
        Duration::from_secs(if self.short { 30 } else { TRAFFIC_SECONDS })
    }

    fn fallback_delay(&self) -> Duration {
        Duration::from_secs(if self.short { 5 } else { 30 })
    }

    fn traffic_wait(&self) -> Duration {
        // The carrier return stage runs before the persistent window: its
        // fallback, hold and return bounds.
        let upgrade = if self.carrier_upgrade() { 120 } else { 0 };
        self.traffic_duration() + Duration::from_secs(125 + upgrade)
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
    /// `--traversal nat` only: A–B becomes an allowed direct edge that A's NAT
    /// router drops; new A→B traffic relays through R at once.
    Relay,
    /// `--traversal nat` only: A's router switches to endpoint-dependent
    /// mapping and R's admits inbound UDP; the punched A–R channel recovers
    /// within a minute of a NAT mapping loss after a real Server restart, of
    /// R's daemon restart and of A's daemon restart.
    Reconnect,
}

impl NetmapScenario {
    fn name(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Revoke => "revoke",
            Self::Expiry => "expiry",
            Self::Restart => "restart",
            Self::Relay => "relay",
            Self::Reconnect => "reconnect",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum TraversalMode {
    Off,
    Nat,
    Exit,
    /// `exit6/off`: the exit topology dual-stack, E carrying IPv6 (NAT66).
    Exit6,
    /// `roam/direct`: A changes uplinks under live traffic.
    Roam,
    /// `fault/throttle`: A→R→B and B's exit E under external pressure.
    Fault,
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
    /// Isolated roaming Server, real daemon, or NAT router.
    RoamActor {
        #[arg(long, value_enum)]
        role: roam::Role,
    },
    /// Isolated fault-injection Server or real daemon.
    FaultActor {
        #[arg(long, value_enum)]
        role: fault::Role,
    },
    Prepare,
    Controller,
    Actor {
        #[arg(long, value_enum)]
        role: Role,
    },
    /// nat/relay only: an additional public relay (`x` or `y`).
    RelayActor {
        #[arg(long)]
        name: String,
    },
    UnderlayPeer {
        #[arg(long)]
        ipv4: std::net::Ipv4Addr,
        #[arg(long)]
        ipv6: std::net::Ipv6Addr,
        /// How long the supervisor may take to signal the stop: its whole
        /// kernel test sequence (the stop follows the last test).
        #[arg(long)]
        stop_within_seconds: u64,
    },
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    STARTED.get_or_init(std::time::Instant::now);
    let cli = Cli::parse();
    let exit = matches!(cli.traversal, TraversalMode::Exit | TraversalMode::Exit6);
    if exit != matches!(cli.command, Command::ExitActor { .. })
        || (exit && cli.dns_backend != DnsBackend::Off)
    {
        eprintln!("--traversal exit/exit6 requires exit-actor and --dns-backend off");
        return std::process::ExitCode::FAILURE;
    }
    if (cli.traversal == TraversalMode::Roam) != matches!(cli.command, Command::RoamActor { .. })
        || (cli.traversal == TraversalMode::Roam
            && (cli.dns_backend != DnsBackend::Off
                || !matches!(cli.transport, CarrierMode::NativeUdp)))
    {
        eprintln!("--traversal roam requires roam-actor, native-udp and --dns-backend off");
        return std::process::ExitCode::FAILURE;
    }
    if (cli.traversal == TraversalMode::Fault) != matches!(cli.command, Command::FaultActor { .. })
        || (cli.traversal == TraversalMode::Fault
            && (cli.dns_backend != DnsBackend::Off
                || !matches!(cli.transport, CarrierMode::NativeUdp)
                || cli.netmap_scenario.is_some()))
    {
        eprintln!(
            "--traversal fault requires fault-actor, native-udp, --dns-backend off and no scenario"
        );
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
    if let Some(scenario) = cli.netmap_scenario {
        let traversal = if matches!(scenario, NetmapScenario::Relay | NetmapScenario::Reconnect) {
            cli.traversal == TraversalMode::Nat && !cli.traversal_fallback
        } else {
            cli.traversal == TraversalMode::Off
        };
        // The expiry row runs resolved too: its DNS owner fails and recovers
        // past the lease end (SP1).
        let dns = cli.dns_backend == DnsBackend::Off
            || (scenario == NetmapScenario::Expiry && cli.dns_backend == DnsBackend::Resolved);
        if !traversal || !dns {
            eprintln!(
                "--netmap-scenario relay/reconnect require --traversal nat (no fallback), the others traversal off; DNS off (expiry: off or resolved)"
            );
            return std::process::ExitCode::FAILURE;
        }
    }
    if cli.logging.init().is_err() {
        eprintln!("invalid runtime fixture logging configuration");
        return std::process::ExitCode::FAILURE;
    }
    let result = match cli.command {
        Command::ExitActor { role } => exit::run(&cli, role).await,
        Command::RoamActor { role } => roam::run(&cli, role).await,
        Command::FaultActor { role } => fault::run(&cli, role).await,
        Command::Prepare => prepare(&cli).await,
        Command::Controller => controller::run(&cli).await,
        Command::Actor { role } => actor::run(&cli, role).await,
        Command::RelayActor { ref name } => {
            if cli.netmap_scenario != Some(NetmapScenario::Relay) {
                eprintln!("relay-actor requires --netmap-scenario relay");
                return std::process::ExitCode::FAILURE;
            }
            actor::relay::run(&cli, name).await
        }
        Command::UnderlayPeer {
            ipv4,
            ipv6,
            stop_within_seconds,
        } => peer::run(&cli, ipv4, ipv6, Duration::from_secs(stop_within_seconds)).await,
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
