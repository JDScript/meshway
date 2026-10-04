#![forbid(unsafe_code)]

use std::{io::Write, net::SocketAddr, path::PathBuf, process::ExitCode, time::Duration};

use clap::Parser;
use meshway_server::{App, Settings, http};
use tokio::net::TcpListener;

#[derive(Parser)]
#[command(version, about = "Meshway Server")]
struct Arguments {
    #[command(flatten)]
    logging: meshway_logging::Options,
    /// Absolute path to this instance's private data directory.
    #[arg(long, env = "MESHWAY_DATA_DIR")]
    data_dir: PathBuf,
    #[arg(long, env = "MESHWAY_LISTEN", default_value = "127.0.0.1:8420")]
    listen: SocketAddr,
    #[arg(long, env = "MESHWAY_REGISTRATION_ENABLED", default_value = "true", action = clap::ArgAction::Set)]
    registration_enabled: bool,
    /// Session lifetime in whole seconds (60 through 2592000).
    #[arg(long, env = "MESHWAY_SESSION_TTL_SECONDS", default_value_t = 86400, value_parser = clap::value_parser!(u64).range(60..=2592000))]
    session_ttl_seconds: u64,
}

#[tokio::main]
async fn main() -> ExitCode {
    let arguments = Arguments::parse();
    if let Err(error) = arguments.logging.init() {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    match run(arguments).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            tracing::error!(error = message, "Server stopped with an error");
            ExitCode::FAILURE
        }
    }
}

async fn run(arguments: Arguments) -> Result<(), &'static str> {
    // Install fallible signal handlers before acquiring storage ownership.
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "Signal setup failed.")?;
    #[cfg(unix)]
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .map_err(|_| "Signal setup failed.")?;
    let mut settings = Settings::new(arguments.data_dir);
    settings.registration_enabled = arguments.registration_enabled;
    settings.session_ttl = Duration::from_secs(arguments.session_ttl_seconds);
    let app = App::open(settings)
        .await
        .map_err(|_| "Server storage startup failed.")?;
    let result = async {
        let listener = TcpListener::bind(arguments.listen)
            .await
            .map_err(|_| "Server listen failed.")?;
        let address = listener
            .local_addr()
            .map_err(|_| "Server listen address unavailable.")?;
        writeln!(std::io::stdout().lock(), "Listening on {address}")
            .map_err(|_| "Server status output failed.")?;
        tracing::info!(%address, version = env!("CARGO_PKG_VERSION"), "Server ready");
        let (signal_result, received_signal) = tokio::sync::oneshot::channel();
        let stop_streams = app.stream_shutdown();
        let shutdown = async move {
            #[cfg(unix)]
            let signal_failed = tokio::select! {
                result = interrupt.recv() => result.is_none(),
                result = terminate.recv() => result.is_none(),
            };
            #[cfg(not(unix))]
            let signal_failed = tokio::signal::ctrl_c().await.is_err();
            tracing::info!(signal_failed, "Server shutdown requested");
            stop_streams();
            let _ = signal_result.send(signal_failed);
        };
        axum::serve(listener, http::router(&app))
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|_| "Server serve failed.")?;
        if received_signal.await.unwrap_or(true) {
            return Err("Server signal handling failed.");
        }
        Ok(())
    }
    .await;
    let shutdown = app
        .shutdown()
        .await
        .map_err(|_| "Server storage shutdown failed.");
    result.and(shutdown)?;
    tracing::info!("Server stopped");
    writeln!(std::io::stdout().lock(), "Server stopped.")
        .map_err(|_| "Server status output failed.")?;
    Ok(())
}
