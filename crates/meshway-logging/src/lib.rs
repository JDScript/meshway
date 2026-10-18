//! Process-wide logging policy. Libraries emit tracing events; only an executable
//! installs a subscriber. stdout remains available for command results/readiness.
#![forbid(unsafe_code)]

pub mod profile;

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
    #[error("Profiling requires MESHWAY_PROFILE=0..65536 and MESHWAY_PROFILE_DETAIL=0..2.")]
    Profile,
}

impl Options {
    /// Validate before opening application storage or starting background tasks.
    /// Invalid directives fail startup instead of silently enabling another level.
    pub fn init(&self) -> Result<(), Error> {
        self.init_with_writer(
            io::stderr,
            io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        )
    }

    /// Embedded process hosts use the same filter and encoding with their
    /// platform log writer. Install once at the entry point, never in a Node.
    pub fn init_with_writer<W>(&self, writer: W, ansi: bool) -> Result<(), Error>
    where
        W: for<'a> tracing_subscriber::fmt::MakeWriter<'a> + Send + Sync + 'static,
    {
        profile::configure()?;
        let filter = EnvFilter::builder()
            .with_regex(false)
            .parse(&self.filter)
            .map_err(|_| Error::Filter)?;
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(writer)
            .with_ansi(ansi);
        match self.format {
            Format::Text => subscriber.finish().try_init(),
            Format::Json => subscriber.with_ansi(false).json().finish().try_init(),
        }
        .map_err(|_| Error::Subscriber)
    }
}
