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
    /// Intersection retains every cancellation/lifetime and the earliest
    /// deadline. It never creates or extends host authorization.
    pub fn intersection(contexts: Vec<Self>) -> Self {
        let deadline = contexts.iter().filter_map(|c| c.deadline).min();
        Self {
            deadline,
            lifetime: Some(Arc::new(Intersection(contexts))),
            ..Self::default()
        }
    }
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

#[derive(Debug)]
struct Intersection(Vec<SendContext>);

impl SendLifetime for Intersection {
    fn is_current(&self) -> bool {
        self.0.iter().all(|context| {
            !context.cancellation.is_cancelled()
                && context
                    .lifetime
                    .as_ref()
                    .is_none_or(|lifetime| lifetime.is_current())
        })
    }
    fn invalidated(self: Arc<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            if self.0.is_empty() {
                pending::<()>().await;
            }
            let waits: Vec<_> = self
                .0
                .iter()
                .map(|context| Box::pin(context.invalidated()))
                .collect();
            futures_util::future::select_all(waits).await;
        })
    }
}

/// Identity of one host-owned channel scope. Clones identify that exact scope;
/// a newly created value cannot batch with it. This is not an authorization.
#[derive(Clone, Debug, Default)]
pub struct ChannelScope(Arc<()>);

impl ChannelScope {
    pub fn new() -> Self {
        Self::default()
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A frame and its original restrictions. A control frame should use the
/// single-frame API, never be placed in a business batch by the host.
#[derive(Debug)]
pub struct ScopedFrame {
    pub frame: crate::Frame,
    pub scope: ChannelScope,
    pub context: SendContext,
}
