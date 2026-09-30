use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};
use meshway_controller::{AppState, openapi_json, router};

#[derive(Parser)]
#[command(
    version,
    about = "Meshway development controller (in-memory, unauthenticated process reports)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the local development control API.
    Serve {
        #[arg(long, default_value = "127.0.0.1:8420")]
        bind: SocketAddr,
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
    }) {
        Command::ExportOpenapi { output } => {
            let document = openapi_json()?;
            match output {
                Some(path) => std::fs::write(path, document)?,
                None => print!("{document}"),
            }
        }
        Command::Serve { bind } => {
            let listener = tokio::net::TcpListener::bind(bind).await?;
            eprintln!(
                "Meshway development controller: http://{} (in-memory process reports; not secure enrollment)",
                listener.local_addr()?
            );
            axum::serve(listener, router(AppState::default()))
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await?;
        }
    }
    Ok(())
}
