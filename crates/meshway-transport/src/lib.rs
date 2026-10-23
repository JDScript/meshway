//! Transport declarations, raw transport I/O and bounded, opaque datagram
//! sessions, with the Native TCP and Native UDP providers.
//!
//! Providers adopt host-supplied carriers. These APIs establish neither peer
//! identity nor authority. A local completion is never a peer acknowledgement.
//! [`spec`] holds the pure declarations shared with configuration.
#![forbid(unsafe_code)]

mod frame;
mod limits;
mod output;
pub mod raw;
mod scope;
mod session;
pub mod spec;
pub mod tcp;
pub mod udp;

pub use frame::{Frame, Payload, PayloadStorage};
pub use limits::{LimitError, Limits};
pub use scope::{CancelReason, ChannelScope, ScopedFrame, SendContext, SendLifetime};
pub use session::{
    Admission, AdmissionStop, BatchCompletion, BatchOutcome, BudgetAvailable, Closing,
    CompletedFrame, Completion, CompletionLost, ReceiveError, Receiver, SendFailure, SendOutcome,
    Sender, Session,
};
