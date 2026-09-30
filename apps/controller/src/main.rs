use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};
use meshway_controller::{AppState, admin_token, openapi_json, router};

#[derive(Parser)]
#[command(version, about = "Meshway persistent control-plane controller")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the authenticated control API, initializing a new private data directory if needed.
    Serve {
        #[arg(long, default_value = "127.0.0.1:8420")]
        bind: SocketAddr,
        #[arg(long, default_value = ".meshway/controller")]
        data_dir: PathBuf,
    },
    /// Explicitly reveal the existing bootstrap admin credential (never initializes state).
    AdminToken {
        #[arg(long, default_value = ".meshway/controller")]
        data_dir: PathBuf,
    },
    /// Export the live router's OpenAPI contract without starting a service.
    ExportOpenapi {
        /// Write to a file instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command.unwrap_or(Command::Serve {
        bind: SocketAddr::from(([127, 0, 0, 1], 8420)),
        data_dir: PathBuf::from(".meshway/controller"),
    }) {
        Command::ExportOpenapi { output } => {
            let document = openapi_json()?;
            match output {
                Some(path) => std::fs::write(path, document)?,
                None => print!("{document}"),
            }
        }
        Command::AdminToken { data_dir } => println!("{}", admin_token(data_dir)?),
        Command::Serve { bind, data_dir } => {
            let state = AppState::open(data_dir)?;
            let listener = tokio::net::TcpListener::bind(bind).await?;
            eprintln!(
                "Meshway controller: http://{} (authenticated control-plane contact)",
                listener.local_addr()?
            );
            axum::serve(listener, router(state))
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await?;
        }
    }
    Ok(())
}
