use clap::{Parser, Subcommand};
use meshway_agent::generated::{client::HttpClient, types::AgentReport};

#[derive(Parser)]
#[command(
    version,
    about = "Meshway agent: development process reporting, not secure enrollment or a mesh tunnel"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Send one real process report to the controller, then exit.
    Report {
        #[arg(long, default_value = "http://127.0.0.1:8420")]
        controller: String,
        #[arg(long)]
        name: String,
        /// Deterministic development identity; defaults to --name. Not an authenticated identity.
        #[arg(long)]
        node_id: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::Report {
            controller,
            name,
            node_id,
        } => {
            let url = reqwest::Url::parse(&controller)?;
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(
                    "controller URL must be HTTP(S) without credentials, query or fragment".into(),
                );
            }
            let client = HttpClient::new().with_base_url(controller.trim_end_matches('/'));
            let response = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                client.report_agent(AgentReport {
                    node_id: node_id.unwrap_or_else(|| name.clone()),
                    name,
                    agent_version: env!("CARGO_PKG_VERSION").to_owned(),
                }),
            )
            .await??;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
    }
    Ok(())
}
