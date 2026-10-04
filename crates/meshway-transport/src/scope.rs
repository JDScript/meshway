use std::future::pending;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// Host-supplied operation lifetime, not an authorization token. Dropping the
/// result waiter never cancels this context. Cancel the token for hard
/// invalidation; deadlines are monotonic Tokio instants.
#[derive(Clone, Debug, Default)]
pub struct SendContext {
    pub cancellation: CancellationToken,
    pub deadline: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelReason {
    ScopeCancelled,
    DeadlineExceeded,
    SessionClosed,
}

impl SendContext {
    pub fn invalid_reason(&self) -> Option<CancelReason> {
        if self.cancellation.is_cancelled() {
            Some(CancelReason::ScopeCancelled)
        } else if self
            .deadline
            .is_some_and(|deadline| deadline <= Instant::now())
        {
            Some(CancelReason::DeadlineExceeded)
        } else {
            None
        }
    }

    pub(crate) async fn invalidated(&self) -> CancelReason {
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => CancelReason::ScopeCancelled,
            _ = async {
                match self.deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => pending().await,
                }
            } => CancelReason::DeadlineExceeded,
        }
    }
}
