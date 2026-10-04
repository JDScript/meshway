mod io;
mod receive;
mod send;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::{Limits, raw};

pub use receive::{ReceiveError, Receiver};
pub use send::{
    Admission, AdmissionStop, BatchCompletion, BudgetAvailable, CompletedFrame, Completion,
    CompletionLost, SendFailure, SendOutcome, Sender,
};

/// A full-duplex carrier and its single send owner. Constructors must run in a
/// Tokio runtime. Drop closes the I/O resources synchronously, then aborts the
/// worker; `shutdown` additionally joins it and resolves accepted completions.
pub struct Session {
    sender: Sender,
    receiver: Receiver,
}

impl Session {
    pub fn stream<S: raw::Stream>(stream: S, peer: std::net::SocketAddr, limits: Limits) -> Self {
        Self::start(io::Shared::stream(stream, peer, limits), limits)
    }

    pub fn datagram<D: raw::Datagram>(endpoint: D, limits: Limits) -> Self {
        Self::start(io::Shared::datagram(endpoint, limits), limits)
    }

    fn start(io: io::Shared, limits: Limits) -> Self {
        let io = Arc::new(io);
        let sender = Sender::start(io.clone(), limits);
        Self {
            sender,
            receiver: Receiver::new(io),
        }
    }

    pub fn sender(&mut self) -> &mut Sender {
        &mut self.sender
    }

    pub fn receiver(&mut self) -> &mut Receiver {
        &mut self.receiver
    }

    /// The sender owns the worker; either half closing invalidates the whole
    /// carrier. Both halves can be used concurrently without per-frame tasks.
    pub fn into_split(self) -> (Sender, Receiver) {
        (self.sender, self.receiver)
    }

    pub async fn shutdown(&mut self) -> Result<(), tokio::task::JoinError> {
        self.sender.shutdown().await
    }
}
