#![forbid(unsafe_code)]
#[cfg(unix)]
mod client;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "meshway", version, about = "Meshway local Node control")]
struct Arguments {
    #[arg(long, global = true, default_value = meshway_ipc::DEFAULT_SOCKET)]
    socket: std::path::PathBuf,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
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

#[cfg(unix)]
#[tokio::main]
async fn main() -> std::process::ExitCode {
    let arguments = Arguments::parse();
    let result = tokio::select! { biased;
        _ = tokio::signal::ctrl_c() => {
            eprintln!("Stopped waiting. Use status to inspect the daemon.");
            return std::process::ExitCode::from(130);
        },
        result = client::run(arguments) => result,
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
#[cfg(not(unix))]
fn main() {
    eprintln!("This CLI requires Unix local sockets.");
    std::process::exit(1);
}
