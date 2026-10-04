use std::{
    future::{Future, pending},
    pin::Pin,
    sync::Arc,
};

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// Optional host lifetime retained by accepted work, independent of a completion
/// waiter. It restricts I/O only, and does not authorize network traffic. Once
/// invalidated it must remain invalid. Raw write/flush polls check `is_current`;
/// one owned invalidation future wakes a blocked operation (not one per poll).
pub trait SendLifetime: std::fmt::Debug + Send + Sync + 'static {
    fn is_current(&self) -> bool;
    fn invalidated(self: Arc<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

/// Host-supplied operation lifetime, not an authorization token. Dropping the
/// result waiter never cancels this context. Cancel the token for hard
/// invalidation; deadlines are monotonic Tokio instants.
#[derive(Clone, Debug, Default)]
pub struct SendContext {
    pub cancellation: CancellationToken,
    pub deadline: Option<Instant>,
    pub lifetime: Option<Arc<dyn SendLifetime>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelReason {
    ScopeCancelled,
    DeadlineExceeded,
    SessionClosed,
}

impl SendContext {
    pub fn invalid_reason(&self) -> Option<CancelReason> {
        if self.cancellation.is_cancelled()
            || self
                .lifetime
                .as_ref()
                .is_some_and(|lifetime| !lifetime.is_current())
        {
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
                match &self.lifetime {
                    Some(lifetime) => lifetime.clone().invalidated().await,
                    None => pending().await,
                }
            } => CancelReason::ScopeCancelled,
            _ = async {
                match self.deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => pending().await,
                }
            } => CancelReason::DeadlineExceeded,
        }
    }
}
