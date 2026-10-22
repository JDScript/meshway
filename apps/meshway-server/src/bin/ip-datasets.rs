//! Host-maintenance command for the deployment-owned GeoIP / ASN datasets.
//! Shares normal App locking and key checks; the running Server must release
//! ownership before this can open storage, and it loads the new dataset when
//! it starts again.
#![forbid(unsafe_code)]
use clap::{Parser, Subcommand};
use meshway_model::resources::{IpInfo, LookupResult};
use meshway_server::resources::datasets::IpDatasetKind;
use meshway_server::{App, Settings, command::Failure};
use std::{io::Write, net::IpAddr, path::PathBuf, process::ExitCode};

/// Datasets larger than this are not real MaxMind databases.
const MAX_DATASET_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Parser)]
struct Arguments {
    #[command(flatten)]
    logging: meshway_logging::Options,
    #[arg(long)]
    data_dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Install a MaxMind DB file (GeoLite2/GeoIP2 City or Country for `geoip`,
    /// ASN for `asn`) as the current dataset of its kind.
    Import {
        #[arg(long)]
        kind: IpDatasetKind,
        #[arg(long)]
        file: PathBuf,
        /// Recorded with the artifact; defaults to the file name.
        #[arg(long)]
        source: Option<String>,
    },
    /// Print the installed dataset artifact IDs.
    Status,
    /// Look an address up in the installed datasets.
    Lookup { ip: IpAddr },
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
        Err(failure) => {
            tracing::error!(error = %failure.message, "IP dataset command failed");
            failure.code
        }
    }
}

async fn run(arguments: Arguments) -> Result<(), Failure> {
    let app = App::open(Settings::new(arguments.data_dir))
        .await
        .map_err(Failure::open)?;
    let result = execute(&app, arguments.command).await;
    let shutdown = app
        .shutdown()
        .await
        .map_err(|_| "Server storage shutdown failed.".to_owned());
    result.and(shutdown).map_err(Failure::from)
}

async fn execute(app: &App, command: Command) -> Result<(), String> {
    let mut stdout = std::io::stdout().lock();
    match command {
        Command::Import { kind, file, source } => {
            let size = std::fs::metadata(&file)
                .map_err(|_| "Dataset file is not readable.".to_owned())?
                .len();
            if size == 0 || size > MAX_DATASET_BYTES {
                return Err("Dataset file size is outside the accepted range.".to_owned());
            }
            let bytes =
                std::fs::read(&file).map_err(|_| "Dataset file is not readable.".to_owned())?;
            let source = source.unwrap_or_else(|| {
                file.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "dataset".to_owned())
            });
            let id = app
                .ip_datasets()
                .install(kind, source, bytes)
                .await
                .map_err(|error| format!("Dataset was not installed: {error}."))?;
            writeln!(stdout, "Installed {kind:?} dataset {id}.")
                .map_err(|_| "Status output failed.".to_owned())
        }
        Command::Status => {
            let status = app
                .ip_datasets()
                .status()
                .map_err(|error| format!("Status unavailable: {error}."))?;
            let show = |id: Option<meshway_model::ArtifactId>| {
                id.map_or_else(|| "none".to_owned(), |id| id.to_string())
            };
            writeln!(
                stdout,
                "geoip: {}\nasn: {}",
                show(status.geoip),
                show(status.asn)
            )
            .map_err(|_| "Status output failed.".to_owned())
        }
        Command::Lookup { ip } => {
            let info: IpInfo = app
                .ip_datasets()
                .lookup(ip)
                .map_err(|error| format!("Lookup unavailable: {error}."))?;
            let geoip = match &info.geoip {
                LookupResult::Available { data, .. } => format!(
                    "country={} city={} position={}",
                    data.country.as_deref().unwrap_or("-"),
                    data.city.as_deref().unwrap_or("-"),
                    data.position.as_ref().map_or_else(
                        || "-".to_owned(),
                        |p| format!("{},{}", p.latitude(), p.longitude())
                    ),
                ),
                other => describe(other),
            };
            let asn = match &info.asn {
                LookupResult::Available { data, .. } => format!(
                    "AS{} {}",
                    data.number,
                    data.organization.as_deref().unwrap_or("-")
                ),
                other => describe(other),
            };
            writeln!(stdout, "geoip: {geoip}\nasn: {asn}")
                .map_err(|_| "Lookup output failed.".to_owned())
        }
    }
}

fn describe<T>(result: &LookupResult<T>) -> String {
    match result {
        LookupResult::Available { .. } => "available".to_owned(),
        LookupResult::NotFound { .. } => "not in dataset".to_owned(),
        LookupResult::Unavailable { reason } => format!("unavailable ({reason:?})"),
        LookupResult::NotApplicable { reason } => format!("not applicable ({reason:?})"),
    }
}
