//! Process-wide logging policy. Libraries emit tracing events; only an executable
//! installs a subscriber. stdout remains available for command results/readiness.
#![forbid(unsafe_code)]

use clap::{Args, ValueEnum};
use std::io::{self, IsTerminal};
use tracing_subscriber::{EnvFilter, util::SubscriberInitExt};

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum Format {
    #[default]
    Text,
    Json,
}

#[derive(Clone, Debug, Args)]
pub struct Options {
    /// tracing filter: a level or comma-separated target=level directives.
    #[arg(
        long = "log-filter",
        env = "RUST_LOG",
        default_value = "info",
        global = true
    )]
    pub filter: String,
    /// Log encoding on stderr, independent of command output on stdout.
    #[arg(
        long = "log-format",
        env = "MESHWAY_LOG_FORMAT",
        value_enum,
        default_value = "text",
        global = true
    )]
    pub format: Format,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid log filter; use a level or comma-separated target=level directives.")]
    Filter,
    #[error("Cannot install the process logging subscriber.")]
    Subscriber,
}

impl Options {
    /// Validate before opening application storage or starting background tasks.
    /// Invalid directives fail startup instead of silently enabling another level.
    pub fn init(&self) -> Result<(), Error> {
        let filter = EnvFilter::builder()
            .with_regex(false)
            .parse(&self.filter)
            .map_err(|_| Error::Filter)?;
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(io::stderr)
            .with_ansi(io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none());
        match self.format {
            Format::Text => subscriber.finish().try_init(),
            Format::Json => subscriber.with_ansi(false).json().finish().try_init(),
        }
        .map_err(|_| Error::Subscriber)
    }
}
