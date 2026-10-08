//! Kernel-side exit resources. These projections never issue business authority.
mod command;
mod nft;
mod probe;

pub use probe::ExitProbe;

use meshway_model::exit::ExitUnavailableReason;

/// A concrete prerequisite failure; command output is deliberately not public.
#[derive(Debug, thiserror::Error)]
#[error("exit prerequisite unavailable: {reason:?}")]
pub struct ExitError {
    pub reason: ExitUnavailableReason,
}

impl From<ExitUnavailableReason> for ExitError {
    fn from(reason: ExitUnavailableReason) -> Self {
        Self { reason }
    }
}
