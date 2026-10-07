use std::{future::poll_fn, io, pin::Pin, sync::Arc};

use thiserror::Error;
use tokio_util::sync::WaitForCancellationFutureOwned;

use super::io::Shared;
use crate::{Frame, raw::DatagramRead};

#[derive(Debug, Error)]
pub enum ReceiveError {
    #[error("carrier closed")]
    Closed,
    #[error("empty frames are forbidden")]
    Empty,
    #[error("stream frame length exceeds the configured maximum")]
    Oversized,
    #[error("datagram was truncated: {0:?}")]
    Truncated(DatagramRead),
    #[error("provider returned inconsistent datagram lengths")]
    InvalidMetadata,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Pull-based receive half: no background receive task or queued payloads.
/// Dropping a `recv` future preserves the stream codec's partial-read state.
pub struct Receiver {
    io: Arc<Shared>,
    stopped: Pin<Box<WaitForCancellationFutureOwned>>,
    closed: bool,
}

impl Receiver {
    /// Pull a bounded immediately available batch. Stream framing drains only
    /// ready frames and retains its cancellation-safe cursor and deferred EOF.
    pub async fn recv_batch(&mut self) -> Result<Vec<Frame>, ReceiveError> {
        if self.closed {
            return Err(ReceiveError::Closed);
        }
        let result = tokio::select! { biased;
            _ = &mut self.stopped => Err(ReceiveError::Closed),
            result = poll_fn(|cx| self.io.poll_recv_batch(cx)) => result,
        };
        if matches!(result, Err(ReceiveError::Closed)) {
            self.closed = true;
        }
        if result.is_err() && self.io.is_stream() {
            self.closed = true;
            self.io.close();
        }
        result
    }
    pub(super) fn new(io: Arc<Shared>) -> Self {
        Self {
            stopped: Box::pin(io.stop.clone().cancelled_owned()),
            io,
            closed: false,
        }
    }

    pub async fn recv(&mut self) -> Result<Frame, ReceiveError> {
        if self.closed {
            return Err(ReceiveError::Closed);
        }
        let result = tokio::select! {
            biased;
            _ = &mut self.stopped => Err(ReceiveError::Closed),
            result = poll_fn(|cx| self.io.poll_recv(cx)) => result,
        };
        if matches!(result, Err(ReceiveError::Closed)) {
            self.closed = true;
        }
        if result.is_err() && self.io.is_stream() {
            self.closed = true;
            self.io.close();
        }
        result
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.io.close();
    }
}
