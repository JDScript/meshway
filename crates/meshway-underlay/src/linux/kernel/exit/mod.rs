//! Kernel-side exit resources. These projections never issue business authority.
mod api;
mod backend;
mod child;
mod command;
mod conntrack;
mod io;
mod nft;
mod objects;
pub(in crate::linux) mod owner;
mod prerequisites;
mod probe;
mod program;
pub(super) mod recovery;
pub(super) mod routing;
mod storage;
mod tuple;

pub use api::{CtConfirmation, ProjectionAck};
pub use io::{ExitReply, ExitReplyReader, ExitScope};
pub use owner::ExitRole;
pub use probe::ExitProbe;
pub use tuple::{ExitTransport, OriginalTuple};

#[cfg(test)]
mod tests;

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
