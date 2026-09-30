//! Opt-in static transport experiment, separate from controller-managed identity.

mod config;
mod fixture;
#[cfg(any(target_os = "linux", test))]
mod packet;
mod runner;
mod tun;

use std::{io, net::SocketAddr, path::PathBuf};

use clap::{Subcommand, ValueEnum};
use meshway_protocol::local;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Copy, ValueEnum)]
pub enum SecondLeg {
    Udp,
    Vless,
}

#[derive(Subcommand)]
pub enum Command {
    /// Generate fresh, private nyc/lax/sha experiment configs. Refuses overwrite.
    Fixture {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long, default_value = "127.0.0.1:41000")]
        lax_nyc: SocketAddr,
        #[arg(long, default_value = "127.0.0.1:41001")]
        lax_sha: SocketAddr,
        #[arg(long, value_enum, default_value = "vless")]
        second_leg: SecondLeg,
        /// Include fixed-pair IPv4 TUN configuration for Linux namespaces.
        #[arg(long)]
        tun: bool,
    },
    /// Validate a private lab configuration without opening network interfaces.
    Check {
        #[arg(long)]
        config: PathBuf,
    },
    /// Run authenticated carriers and optional Linux TUN until interrupted.
    Run {
        #[arg(long)]
        config: PathBuf,
        /// Atomically publish private, non-secret experiment status once per second.
        #[arg(long)]
        status_file: Option<PathBuf>,
    },
    /// Exchange endpoint-authenticated echo bytes through the exact ordered path.
    Probe {
        #[arg(long)]
        config: PathBuf,
        #[arg(long, value_delimiter = ',', num_args = 1)]
        path: Vec<String>,
        #[arg(long, default_value_t = 128, value_parser = clap::value_parser!(u16).range(1..=8192))]
        bytes: u16,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..=100))]
        count: u16,
    },
}

pub async fn execute(command: Command) -> Result<()> {
    match command {
        Command::Fixture {
            directory,
            lax_nyc,
            lax_sha,
            second_leg,
            tun,
        } => fixture::generate(
            &directory,
            lax_nyc,
            lax_sha,
            matches!(second_leg, SecondLeg::Vless),
            tun,
        ),
        Command::Check { config } => {
            let config = config::Config::read(&config)?;
            println!(
                "Lab configuration is valid for node {}; no networking applied.",
                config.dataplane.node_id
            );
            Ok(())
        }
        Command::Run {
            config,
            status_file,
        } => {
            let _lock = local::lock(&config.with_extension("lock"))?;
            runner::run(config::Config::read(&config)?, status_file).await
        }
        Command::Probe {
            config,
            path,
            bytes,
            count,
        } => {
            let _lock = local::lock(&config.with_extension("lock"))?;
            let config = config::Config::read(&config)?;
            if path.len() < 2 || !config.dataplane.allowed_paths.contains(&path) {
                return Err(io::Error::other(
                    "probe requires an explicitly authorized ordered path",
                )
                .into());
            }
            runner::probe(config, path, usize::from(bytes), count).await
        }
    }
}
