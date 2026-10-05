//! Standalone measurements; cargo test runs lifecycle checks, never measurements.
#![forbid(unsafe_code)]

mod connection;
mod measurement;

use clap::{Parser, ValueEnum};
use serde::Serialize;
use std::{io::Write, process::ExitCode, time::Duration};

type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Copy, Debug, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum Protocol {
    Tcp,
    Udp,
    VlessTcp,
    VlessWs,
    Trojan,
}

#[derive(Clone, Copy, Debug, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Raw,
    Datagram,
}

#[derive(Parser)]
#[command(about = "Explicit native socket raw/datagram performance measurement")]
struct Arguments {
    #[arg(long, value_enum)]
    protocol: Protocol,
    #[arg(long, value_enum)]
    mode: Mode,
    #[arg(long, default_value_t = 1200, value_parser = clap::value_parser!(u32).range(16..=65000))]
    payload_bytes: u32,
    #[arg(long, default_value_t = 32, value_parser = clap::value_parser!(u32).range(1..=1024))]
    batch_size: u32,
    #[arg(long, default_value_t = 200000, value_parser = clap::value_parser!(u32).range(1..=1000000))]
    messages: u32,
    #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u32).range(0..=100000))]
    warmup: u32,
}

#[tokio::main(worker_threads = 2)]
async fn main() -> ExitCode {
    let args = Arguments::parse();
    let result = tokio::time::timeout(Duration::from_secs(110), async {
        let mut connection = connection::Connection::open(&args).await?;
        let result = measurement::run(&args, &mut connection).await;
        let closed = connection.close().await;
        let metrics = result?;
        closed?;
        let mut output = std::io::stdout().lock();
        serde_json::to_writer(&mut output, &metrics)?;
        writeln!(output)?;
        Ok::<_, Error>(())
    })
    .await;
    match result {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(error)) => {
            eprintln!("Measurement failed: {error}");
            ExitCode::FAILURE
        }
        Err(_) => {
            eprintln!("Measurement deadline expired; no valid performance result.");
            ExitCode::FAILURE
        }
    }
}
