use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "meshway", version, about = "Meshway local Node control")]
pub(crate) struct Arguments {
    #[command(flatten)]
    pub logging: meshway_logging::Options,
    #[arg(long, global = true, default_value = meshway_ipc::DEFAULT_SOCKET)]
    pub socket: std::path::PathBuf,
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
    },
    /// Request online control; prints the browser URL when approval is needed.
    Up,
    /// Persist offline intent and stop the active Node instance.
    Down,
    /// Read actual daemon, identity, lease and saved configuration state.
    Status {
        #[arg(long)]
        watch: bool,
    },
}
