//! Raw transport I/O and bounded, opaque datagram sessions.
//!
//! Providers adopt host-supplied carriers. These APIs establish neither peer
//! identity nor authority. A local completion is never a peer acknowledgement.
#![forbid(unsafe_code)]

mod frame;
mod limits;
mod output;
pub mod raw;
mod scope;
mod session;

pub use frame::{Frame, Payload, PayloadStorage};
pub use limits::{LimitError, Limits};
pub use scope::{CancelReason, ChannelScope, ScopedFrame, SendContext, SendLifetime};
pub use session::{
    Admission, AdmissionStop, BatchCompletion, BatchOutcome, BudgetAvailable, CompletedFrame,
    Completion, CompletionLost, ReceiveError, Receiver, SendFailure, SendOutcome, Sender, Session,
};
