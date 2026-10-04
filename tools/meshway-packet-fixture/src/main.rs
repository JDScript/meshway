//! Isolated Linux conformance actor. Build on the host; run in separate namespaces.
#![forbid(unsafe_code)]

mod application;
mod controller;
mod node;
mod runtime;
mod signals;
mod tls;
mod wire;

use clap::{Parser, Subcommand};
use std::{path::PathBuf, time::Duration};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;
const STARTUP: Duration = Duration::from_secs(30);
const IO: Duration = Duration::from_secs(10);
const ORIGIN: &str = "https://meshway-packet-controller:8443/";

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
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Prepare,
    Controller,
    Node {
        #[arg(long, value_enum)]
        role: signals::Role,
    },
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    if cli.logging.init().is_err() {
        eprintln!("invalid fixture logging configuration");
        return std::process::ExitCode::FAILURE;
    }
    let result = match cli.command {
        Command::Prepare => tls::prepare(&cli).await,
        Command::Controller => controller::run(&cli).await,
        Command::Node { role } => node::run(&cli, role).await,
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => {
            // Errors may originate in authenticated APIs. Do not print request
            // bodies, private configuration or credentials through Debug chains.
            tracing::error!("packet fixture failed; inspect last completed phase");
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
