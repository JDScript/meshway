//! DNS command formatting; typed public results have one Rust DTO source.
use clap::Subcommand;
use meshway_agent::diagnostics::{
    SCHEMA_VERSION,
    dns::{self, Error, QueryState, QueryType},
};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum Command {
    /// Read a point-in-time DNS Guard snapshot (not query success or a cached ACK).
    Status {
        #[arg(long, env = "MESHWAY_STATE", default_value = ".meshway/agent.json")]
        state: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Query the daemon's actual loopback listener; IP input defaults to PTR.
    Query {
        name_or_ip: String,
        #[arg(long = "type", value_enum)]
        query_type: Option<QueryType>,
        #[arg(long)]
        tcp: bool,
        #[arg(long, env = "MESHWAY_STATE", default_value = ".meshway/agent.json")]
        state: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Serialize)]
#[serde(tag = "status", content = "result", rename_all = "snake_case")]
enum Outcome<T> {
    Ok(T),
    Error(Error),
}

#[derive(Serialize)]
struct ResultEnvelope<T> {
    schema_version: u32,
    outcome: Outcome<T>,
}

#[derive(Serialize)]
struct FailedQuery {
    schema_version: u32,
    outcome: Outcome<()>,
    observation: dns::Observation,
}

fn print<T: Serialize>(
    result: Result<T, Error>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match result {
        Ok(value) => {
            if json {
                println!(
                    "{}",
                    super::cli_live::escaped_json(&ResultEnvelope {
                        schema_version: SCHEMA_VERSION,
                        outcome: Outcome::Ok(value)
                    })?
                );
            } else {
                println!("{}", super::cli_live::escaped_json(&value)?);
            }
            Ok(())
        }
        Err(error) => {
            if json {
                println!(
                    "{}",
                    super::cli_live::escaped_json(&ResultEnvelope::<()> {
                        schema_version: SCHEMA_VERSION,
                        outcome: Outcome::Error(error)
                    })?
                );
            }
            Err(error.into())
        }
    }
}

pub async fn execute(command: Command) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Command::Status { state, json } => print(dns::status(&state).await, json),
        Command::Query {
            name_or_ip,
            query_type,
            tcp,
            state,
            json,
        } => {
            // Input errors use the same exit code as clap usage errors, with JSON
            // kept separate from the diagnostic on stderr.
            if let Err(error) = dns::question(&name_or_ip, query_type) {
                let _ = print::<()>(Err(error), json);
                eprintln!("{error}");
                std::process::exit(2);
            }
            match dns::query(&state, &name_or_ip, query_type, tcp).await {
                Ok(observation) => {
                    let failure = match observation.state {
                        QueryState::Refused => Some(Error::Refused),
                        QueryState::Servfail => Some(Error::Servfail),
                        _ => None,
                    };
                    if let Some(error) = failure {
                        if json {
                            println!(
                                "{}",
                                super::cli_live::escaped_json(&FailedQuery {
                                    schema_version: SCHEMA_VERSION,
                                    outcome: Outcome::Error(error),
                                    observation,
                                })?
                            );
                        } else {
                            println!("{}", super::cli_live::escaped_json(&observation)?);
                        }
                        return Err(error.into());
                    }
                    print(Ok(observation), json)?;
                    Ok(())
                }
                Err(error) => print::<()>(Err(error), json),
            }
        }
    }
}
