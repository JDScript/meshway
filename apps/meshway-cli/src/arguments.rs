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
