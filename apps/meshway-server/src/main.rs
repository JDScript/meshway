#![forbid(unsafe_code)]

use std::{
    io::Write, net::SocketAddr, path::PathBuf, process::ExitCode, sync::LazyLock, time::Duration,
};

use clap::{Parser, Subcommand};
use meshway_server::{App, Migration, Settings, command::Failure, http};
use tokio::net::TcpListener;

/// `--version`: the build identity, and the schema range an upgrade checks a
/// data directory against before it stops the running Server.
static LONG_VERSION: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{}\nschema: {} (migrates from {})\ntarget: {}",
        meshway_build::identity(),
        meshway_server::SCHEMA_VERSION,
        meshway_server::MIN_MIGRATABLE_SCHEMA,
        meshway_build::TARGET,
    )
});

#[derive(Parser)]
#[command(
    version,
    long_version = LONG_VERSION.as_str(),
    about = "Meshway Server",
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
struct Arguments {
    #[command(flatten)]
    logging: meshway_logging::Options,
    #[command(subcommand)]
    command: Option<Command>,
    /// Absolute path to this instance's private data directory.
    #[arg(long, env = "MESHWAY_DATA_DIR", required = true)]
    data_dir: Option<PathBuf>,
    #[arg(long, env = "MESHWAY_LISTEN", default_value = "127.0.0.1:8420")]
    listen: SocketAddr,
    #[arg(long, env = "MESHWAY_REGISTRATION_ENABLED", default_value = "true", action = clap::ArgAction::Set)]
    registration_enabled: bool,
    /// Session lifetime in whole seconds (60 through 2592000).
    #[arg(long, env = "MESHWAY_SESSION_TTL_SECONDS", default_value_t = 86400, value_parser = clap::value_parser!(u64).range(60..=2592000))]
    session_ttl_seconds: u64,
    /// Browser Console origin (https, or http on loopback), such as
    /// https://console.example.com. Device-authorization links are built on
    /// it; without it the Server hands devices no link. Requests from it to
    /// /api/v1 are answered cross-origin.
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
    stun_server: Vec<(String, meshway_transport::spec::Endpoint)>,
    /// Netmap lease validity in whole seconds (60 through 86400); every
    /// sync answer renews the lease to now plus this validity.
    #[arg(long, env = "MESHWAY_NETMAP_VALIDITY", default_value_t = 86400, value_parser = clap::value_parser!(u64).range(60..=86400))]
    netmap_validity: u64,
    /// Nominal hold of an unchanged sync request in whole seconds (1 through
    /// 54), jittered by ±10%.
    #[arg(long, env = "MESHWAY_NETMAP_SYNC_HOLD", default_value_t = 50, value_parser = clap::value_parser!(u64).range(1..=54))]
    netmap_sync_hold: u64,
}

/// Offline host-maintenance commands. They share normal storage locking and
/// key checks: the serving process must release the data directory first.
/// Only `migrate` converts a database of an earlier schema version; the
/// others refuse one.
#[derive(Subcommand)]
enum Command {
    /// Migrate the database to this build's schema version (keeping the old
    /// file as a backup in the data directory), compile every stored netmap
    /// with this build, and exit without serving. A current database is
    /// checked and left as it is.
    Migrate {
        /// Absolute path to this instance's private data directory.
        #[arg(long, env = "MESHWAY_DATA_DIR")]
        data_dir: PathBuf,
    },
    /// Copy the database and the master key into the new private directory
    /// OUTPUT (0700, files 0600), itself a data directory a Server can be
    /// started on. Together they are every secret of this Server, its
    /// netmap signing keys included: protect the copy like the host.
    Backup {
        /// Absolute path to this instance's private data directory.
        #[arg(long, env = "MESHWAY_DATA_DIR")]
        data_dir: PathBuf,
        /// Absolute path of the backup directory to create; it must not exist.
        #[arg(long)]
        output: PathBuf,
    },
    /// Start a netmap signing-key rotation: a new key of the next generation
    /// becomes current, and for the window every sync answer carries it with
    /// an endorsement by the replaced key. Rejected while a window is open.
    RotateNetmapKey {
        /// Absolute path to this instance's private data directory.
        #[arg(long, env = "MESHWAY_DATA_DIR")]
        data_dir: PathBuf,
        /// Handover window, such as 48h, 90m or 120s (60 s through 30 days);
        /// defaults to twice the longest lease, 48h.
        #[arg(long, value_parser = window)]
        window: Option<Duration>,
    },
}

fn window(value: &str) -> Result<Duration, String> {
    let split = value
        .find(|c: char| !c.is_ascii_digit())
        .ok_or("expected a number followed by s, m, h or d")?;
    let (number, unit) = value.split_at(split);
    let number: u64 = number.parse().map_err(|_| "expected a whole number")?;
    let seconds = match unit {
        "s" => Some(number),
        "m" => number.checked_mul(60),
        "h" => number.checked_mul(60 * 60),
        "d" => number.checked_mul(24 * 60 * 60),
        _ => None,
    }
    .ok_or("expected a number followed by s, m, h or d")?;
    let window = Duration::from_secs(seconds);
    if !(meshway_server::netmap::signing::MIN_ROTATION_WINDOW
        ..=meshway_server::netmap::signing::MAX_ROTATION_WINDOW)
        .contains(&window)
    {
        return Err("the window must be between 60 seconds and 30 days".into());
    }
    Ok(window)
}

#[tokio::main]
async fn main() -> ExitCode {
    let mut arguments = Arguments::parse();
    if let Err(error) = arguments.logging.init() {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    if let Some(command) = arguments.command.take() {
        return match maintain(command).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(failure) => {
                tracing::error!(error = %failure.message, "Server maintenance command failed");
                failure.code
            }
        };
    }
    match run(arguments).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            tracing::error!(error = %failure.message, "Server stopped with an error");
            failure.code
        }
    }
}

async fn maintain(command: Command) -> Result<(), Failure> {
    let (data_dir, window) = match command {
        Command::Migrate { data_dir } => return migrate(data_dir).await,
        Command::Backup { data_dir, output } => return backup(data_dir, output).await,
        Command::RotateNetmapKey { data_dir, window } => (data_dir, window),
    };
    let window = window.unwrap_or(meshway_server::netmap::signing::DEFAULT_ROTATION_WINDOW);
    let app = App::open(Settings::new(data_dir))
        .await
        .map_err(Failure::open)?;
    let result = app
        .rotate_netmap_key(window)
        .await
        .map_err(|error| match error {
            meshway_server::error::Error::StateConflict => {
                "A netmap signing-key rotation window is still open; wait until it ends.".to_owned()
            }
            error => format!("Netmap signing key was not rotated: {error}."),
        });
    let shutdown = app
        .shutdown()
        .await
        .map_err(|_| "Server storage shutdown failed.".to_owned());
    let rotated = result.and_then(|rotated| shutdown.map(|()| rotated))?;
    let ends = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
        i64::try_from(rotated.window_ends_at_ms).unwrap_or(i64::MAX),
    )
    .map_or_else(|| "unknown".to_owned(), |at| at.to_rfc3339());
    writeln!(
        std::io::stdout().lock(),
        "Netmap signing key rotated: generation {} ({}) is current; generation {} ({}) endorses it until {ends}.",
        rotated.next.key_generation.get(),
        rotated.next.key_id,
        rotated.previous.key_generation.get(),
        rotated.previous.key_id,
    )
    .map_err(|_| "Status output failed.".to_owned().into())
}

/// Besides its sentence, `migrate` prints one `Migration state:` line that a
/// caller such as `install.sh` acts on, read back from the data directory
/// after a failure: `current`, `published <backup>`, `unpublished` or
/// `unknown`.
async fn migrate(data_dir: PathBuf) -> Result<(), Failure> {
    let since = chrono::Utc::now().timestamp_millis();
    let mut settings = Settings::new(data_dir.clone());
    settings.migration = Migration::Automatic;
    let state = |line: String| {
        let _ = writeln!(std::io::stdout().lock(), "Migration state: {line}");
    };
    let app = match App::open(settings).await {
        Ok(app) => app,
        Err(error) => {
            let outcome = tokio::task::spawn_blocking(move || {
                meshway_server::migration_outcome(&data_dir, since)
            })
            .await
            .unwrap_or(meshway_server::MigrationOutcome::Unknown);
            state(match outcome {
                meshway_server::MigrationOutcome::Unpublished { .. } => "unpublished".into(),
                meshway_server::MigrationOutcome::Published { backup, .. } => {
                    format!("published {}", backup.display())
                }
                meshway_server::MigrationOutcome::Unknown => "unknown".into(),
            });
            return Err(Failure::open(error));
        }
    };
    let (report, line) = match app.migrated() {
        Some(migrated) => (
            format!(
                "Database migrated from schema {} to {} in {} ms; the previous database is kept as {}.",
                migrated.from,
                migrated.to,
                migrated.elapsed.as_millis(),
                migrated.backup.display(),
            ),
            format!("published {}", migrated.backup.display()),
        ),
        None => (
            "Database is already at this build's schema; nothing migrated.".to_owned(),
            "current".to_owned(),
        ),
    };
    let shutdown = app.shutdown().await;
    state(line);
    shutdown.map_err(|_| "Server storage shutdown failed.".to_owned())?;
    writeln!(std::io::stdout().lock(), "{report}")
        .map_err(|_| "Status output failed.".to_owned().into())
}

async fn backup(data_dir: PathBuf, output: PathBuf) -> Result<(), Failure> {
    let backup = meshway_server::backup::backup(data_dir, output)
        .await
        .map_err(|error| match error {
            meshway_server::AppError::BackupOutput => {
                Failure::from(format!("Backup failed: {error}."))
            }
            error => Failure::open(error),
        })?;
    writeln!(
        std::io::stdout().lock(),
        "Backed up the database (schema {}, {} bytes) and the master key to {}.",
        backup.schema,
        backup.database_bytes,
        backup.output.display(),
    )
    .map_err(|_| "Status output failed.".into())
}

async fn run(arguments: Arguments) -> Result<(), Failure> {
    // Install fallible signal handlers before acquiring storage ownership.
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "Signal setup failed.".to_owned())?;
    #[cfg(unix)]
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .map_err(|_| "Signal setup failed.".to_owned())?;
    let mut settings = Settings::new(
        arguments
            .data_dir
            .ok_or_else(|| "--data-dir is required.".to_owned())?,
    );
    settings.migration = Migration::Automatic;
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
            return Err("Duplicate STUN server key.".to_owned().into());
        }
    }
    let app = App::open(settings).await.map_err(Failure::open)?;
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
        meshway_service::notify(&format!("READY=1\nSTATUS=Serving on {address}"));
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
            meshway_service::notify("STOPPING=1");
            stop_streams();
            let _ = signal_result.send(signal_failed);
        };
        // The direct peer address limits failed logins (`accounts::throttle`).
        axum::serve(
            listener,
            http::router(&app).into_make_service_with_connect_info::<SocketAddr>(),
        )
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
