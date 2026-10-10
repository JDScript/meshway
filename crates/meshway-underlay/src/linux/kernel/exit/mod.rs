//! Kernel-side exit resources. These projections never issue business authority.
use super::tool::{child, command};
mod api;
mod backend;
mod conntrack;
mod firewall;
mod guard;
mod host;
mod incarnation;
mod io;
mod legacy;
mod live_probe;
mod nft;
pub(super) mod objects;
pub(in crate::linux) mod owner;
mod prerequisites;
mod probe;
mod program;
pub(super) mod recovery;
pub(super) mod routing;
mod storage;
mod tuple;

pub use api::{CtConfirmation, ProjectionAck};
pub use guard::{ExitSendGuard, Invalidated};
pub use incarnation::ExitIncarnation;
pub use io::{ExitCutoff, ExitReply, ExitReplyReader, ExitScope};
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
