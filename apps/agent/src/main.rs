use std::{path::PathBuf, time::Duration};

use clap::{Parser, Subcommand};
use meshway_agent::{Agent, status};

#[derive(Parser)]
#[command(
    version,
    about = "Meshway agent: persistent control-plane identity and authenticated heartbeats"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Persist a proposed credential, then enroll (safe to retry the same binding).
    Enroll {
        #[arg(long)]
        controller: String,
        #[arg(long)]
        token: String,
        #[arg(long)]
        name: String,
        #[arg(long, default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
    /// Send one authenticated process heartbeat, recovering a pending ID if needed.
    Heartbeat {
        #[arg(long, default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
    /// Heartbeat immediately, then periodically; retry transient failures with backoff.
    Run {
        #[arg(long, default_value = ".meshway/agent.json")]
        state: PathBuf,
        #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..=86400))]
        interval_seconds: u64,
    },
    /// Read local identity metadata without exposing credentials or claiming liveness.
    Status {
        #[arg(long, default_value = ".meshway/agent.json")]
        state: PathBuf,
    },
}

#[tokio::main]
async fn main() {
    if let Err(error) = execute(Cli::parse().command).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn execute(command: Command) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Command::Enroll {
            controller,
            token,
            name,
            state,
        } => {
            let mut agent = Agent::prepare_enrollment(state, &controller, &token, &name)?;
            let response = agent.enroll(&token).await?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        Command::Heartbeat { state } => {
            let response = Agent::load(state)?.heartbeat().await?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        Command::Status { state } => println!("{}", serde_json::to_string_pretty(&status(state)?)?),
        Command::Run {
            state,
            interval_seconds,
        } => {
            let mut agent = Agent::load(state)?;
            let mut backoff = 1_u64;
            loop {
                let result = tokio::select! {
                    result = agent.heartbeat() => result,
                    _ = tokio::signal::ctrl_c() => return Ok(()),
                };
                let delay = match result {
                    Ok(response) => {
                        backoff = 1;
                        println!("{}", serde_json::to_string(&response)?);
                        interval_seconds
                    }
                    Err(error) if error.retryable() => {
                        let delay = backoff;
                        backoff = (backoff * 2).min(60);
                        eprintln!("{error}; retrying in {delay}s");
                        delay
                    }
                    Err(error) => return Err(error.into()),
                };
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(delay)) => {},
                    _ = tokio::signal::ctrl_c() => return Ok(()),
                }
            }
        }
    }
    Ok(())
}
