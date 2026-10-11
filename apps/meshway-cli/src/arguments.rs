use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "meshway", version, about = "Meshway local Node control")]
pub(crate) struct Arguments {
    #[command(flatten)]
    pub logging: meshway_logging::Options,
    #[arg(long, global = true, default_value = meshway_protocol::ipc::DEFAULT_SOCKET)]
    pub socket: std::path::PathBuf,
    /// Print typed JSON instead of text (ping: one event per line).
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Diagnose the authorized full Mesh route using ICMP (5s probes, >=1s interval).
    Ping {
        /// Node hostname or Mesh IP address.
        target: String,
        #[arg(short = 'c', long, default_value_t = 10, value_parser = clap::value_parser!(u16).range(1..=1000))]
        count: u16,
        /// Also print preparation progress and how each channel was acquired.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Request online control; prints the browser URL when approval is needed.
    Up {
        /// One-use enrollment key, @file, or - / @- for stdin. Prefer file/stdin.
        #[arg(long, value_name = "KEY|@FILE|-", allow_hyphen_values = true)]
        enrollment_key: Option<String>,
        /// Enroll the held identity again (same Node and keys) after a missed
        /// Server signing-key rotation; requires approval in the Console.
        #[arg(long, conflicts_with = "enrollment_key")]
        reenroll: bool,
    },
    /// Atomically rotate both identity keys through the daemon's durable owner.
    RotateKey,
    /// Persist offline intent and stop the active Node instance.
    Down,
    /// Delete this Node from its network on the Server, then forget its identity locally.
    Logout,
    /// Read actual daemon, identity, lease, saved configuration and peer state.
    Status {
        #[arg(long)]
        watch: bool,
    },
    /// Check this Node's UDP mappings, STUN observers and Server reachability.
    Netcheck,
    /// Print this build's version and the protocol versions it speaks; does not contact the daemon.
    Version,
}
