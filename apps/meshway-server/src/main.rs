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
    /// Browser Console origin (https, or http on loopback), such as
    /// https://console.example.com. Device-authorization links are built on
    /// it; without it the Server hands devices no link.
    #[arg(long, env = "MESHWAY_CONSOLE_ORIGIN", value_parser = meshway_server::console_origin)]
    console_origin: Option<meshway_server::settings::Url>,
    /// A browser client allowed to obtain delegated sessions, as
    /// `<name>=<origin>` (https, or http on loopback); repeatable. Requests
    /// from these origins to /api/v1 are answered cross-origin.
    #[arg(
        long = "delegation-client",
        env = "MESHWAY_DELEGATION_CLIENTS",
        value_delimiter = ','
    )]
    delegation_clients: Vec<String>,
    /// Origin at which clients reach this Server, handed to delegation clients
    /// with their session.
    #[arg(long, env = "MESHWAY_PUBLIC_ORIGIN", value_parser = meshway_server::console_origin)]
    public_origin: Option<meshway_server::settings::Url>,
    /// Explicit UDP Binding listener; repeat for separate address families.
    #[arg(long)]
    stun_listen: Vec<SocketAddr>,
    /// Advertised hosted or external STUN service, KEY=HOST:PORT (IPv6 in brackets).
    #[arg(long, value_parser = meshway_server::settings::stun_server)]
    stun_server: Vec<(String, meshway_transport_spec::Endpoint)>,
    /// Netmap lease validity in whole seconds (60 through 86400); every
    /// sync answer renews the lease to now plus this validity.
    #[arg(long, env = "MESHWAY_NETMAP_VALIDITY", default_value_t = 86400, value_parser = clap::value_parser!(u64).range(60..=86400))]
    netmap_validity: u64,
    /// Nominal hold of an unchanged sync request in whole seconds (1 through
    /// 54), jittered by ±10%.
    #[arg(long, env = "MESHWAY_NETMAP_SYNC_HOLD", default_value_t = 50, value_parser = clap::value_parser!(u64).range(1..=54))]
    netmap_sync_hold: u64,
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
            tracing::error!(error = %message, "Server stopped with an error");
            ExitCode::FAILURE
        }
    }
}

async fn run(arguments: Arguments) -> Result<(), String> {
    // Install fallible signal handlers before acquiring storage ownership.
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "Signal setup failed.".to_owned())?;
    #[cfg(unix)]
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .map_err(|_| "Signal setup failed.".to_owned())?;
    let mut settings = Settings::new(arguments.data_dir);
    settings.registration_enabled = arguments.registration_enabled;
    settings.session_ttl = Duration::from_secs(arguments.session_ttl_seconds);
    settings.console_origin = arguments.console_origin;
    settings.delegation_clients = meshway_server::accounts::delegation::DelegationClients::parse(
        arguments.delegation_clients.iter().map(String::as_str),
    )
    .map_err(|error| format!("Invalid --delegation-client: {error}."))?;
    settings.public_origin = arguments.public_origin;
    settings.stun_listen = arguments.stun_listen;
    settings.netmap = meshway_server::netmap::SyncTiming {
        validity: Duration::from_secs(arguments.netmap_validity),
        hold: Duration::from_secs(arguments.netmap_sync_hold),
    };
    for (key, endpoint) in arguments.stun_server {
        if settings.stun_servers.insert(key, endpoint).is_some() {
            return Err("Duplicate STUN server key.".into());
        }
    }
    // AppError is a safe category with, at most, schema object names.
    let app = App::open(settings)
        .await
        .map_err(|error| format!("Server storage startup failed: {error}."))?;
    let result = async {
        let listener = TcpListener::bind(arguments.listen)
            .await
            .map_err(|_| "Server listen failed.".to_owned())?;
        let address = listener
            .local_addr()
            .map_err(|_| "Server listen address unavailable.".to_owned())?;
        writeln!(std::io::stdout().lock(), "Listening on {address}")
            .map_err(|_| "Server status output failed.".to_owned())?;
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
            .map_err(|_| "Server serve failed.".to_owned())?;
        if received_signal.await.unwrap_or(true) {
            return Err("Server signal handling failed.".to_owned());
        }
        Ok(())
    }
    .await;
    let shutdown = app
        .shutdown()
        .await
        .map_err(|_| "Server storage shutdown failed.".to_owned());
    result.and(shutdown)?;
    tracing::info!("Server stopped");
    writeln!(std::io::stdout().lock(), "Server stopped.")
        .map_err(|_| "Server status output failed.".to_owned())?;
    Ok(())
}
