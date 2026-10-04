//! Offline host-maintenance command. Shares normal App locking and key checks;
//! the running Server must release ownership before this can open storage.
#![forbid(unsafe_code)]
use clap::Parser;
use meshway_server::{App, Settings};
use std::{io::Write, path::PathBuf, process::ExitCode};

#[derive(Parser)]
struct Arguments {
    #[arg(long)]
    data_dir: PathBuf,
    #[arg(long)]
    email: String,
    /// HTTPS Console origin (http loopback is supported for local development).
    #[arg(long)]
    console_origin: String,
    #[arg(long,default_value_t=900,value_parser=clap::value_parser!(u32).range(60..=86400))]
    expires_in_seconds: u32,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Arguments::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
async fn run(args: Arguments) -> Result<(), &'static str> {
    let origin =
        reqwest::Url::parse(&args.console_origin).map_err(|_| "Invalid Console origin.")?;
    if !(origin.scheme() == "https"
        || (origin.scheme() == "http"
            && matches!(origin.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))))
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
        || origin.path() != "/"
    {
        return Err("Expected a secure Console origin.");
    }
    let app = App::open(Settings::new(args.data_dir))
        .await
        .map_err(|_| "Cannot obtain private Server storage ownership. Stop the Server first.")?;
    let result = async {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|_| "Random source unavailable.")?;
        bytes[6] = (bytes[6] & 15) | 64;
        bytes[8] = (bytes[8] & 63) | 128;
        let id = meshway_model::RequestId::new(uuid::Uuid::from_bytes(bytes))
            .map_err(|_| "Random source unavailable.")?;
        let reset = app
            .accounts()
            .issue_password_reset(args.email, args.expires_in_seconds, id)
            .await
            .map_err(|_| "Password reset could not be issued.")?;
        // This explicit command output is the only original-secret delivery.
        writeln!(
            std::io::stdout().lock(),
            "{}reset-password#token={}",
            origin,
            reset.token.expose()
        )
        .map_err(|_| "Reset link output failed.")
    }
    .await;
    let shutdown = app
        .shutdown()
        .await
        .map_err(|_| "Server storage shutdown failed.");
    result.and(shutdown)
}
