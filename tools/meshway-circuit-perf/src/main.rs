//! L0: synchronous, full-policy protected packet path. No runtime or socket.
#![forbid(unsafe_code)]

mod measurement;

use clap::Parser;
use meshway_static_authority::Error;
use std::io::Write;

#[derive(Parser)]
#[command(about = "In-process SecureNode chain CPU benchmark (L0)")]
struct Arguments {
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(2..=8))]
    nodes: u8,
    #[arg(long, default_value_t = 1280, value_parser = clap::value_parser!(u32).range(52..=65519))]
    packet_bytes: u32,
    #[arg(long, default_value_t = 10000, value_parser = clap::value_parser!(u32).range(1..=1000000))]
    rounds: u32,
    #[arg(long, default_value_t = 200, value_parser = clap::value_parser!(u32).range(0..=100000))]
    warmup: u32,
    /// Explicit single-in-flight handoff comparison; omitted uses synchronous L0.
    #[arg(long, value_parser = clap::value_parser!(u16).range(0..=256))]
    workers: Option<u16>,
}

fn run() -> Result<(), Error> {
    let args = Arguments::parse();
    meshway_logging::Options {
        filter: "info".into(),
        format: meshway_logging::Format::Json,
    }
    .init()?;
    let report = measurement::run(&args)?;
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, &report)?;
    writeln!(output)?;
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Circuit measurement failed: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
