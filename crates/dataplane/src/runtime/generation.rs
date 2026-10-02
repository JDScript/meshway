//! Local packet-admission fence, independent of hop and E2E key lifetimes.
use super::lock;
use crate::{Error, Result};
use std::{
    future::Future,
    sync::{Arc, Mutex, MutexGuard},
};
use tokio::sync::watch;

pub(crate) struct Fence {
    retired: Mutex<bool>,
    changed: watch::Sender<bool>,
}

impl Fence {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            retired: Mutex::new(false),
            changed: watch::channel(false).0,
        })
    }

    pub(super) fn enter(&self) -> Result<MutexGuard<'_, bool>> {
        let guard = lock(&self.retired);
        if *guard {
            Err(Error::Closed)
        } else {
            Ok(guard)
        }
    }

    pub(super) fn retired(&self) -> bool {
        *lock(&self.retired)
    }

    pub(crate) fn retire(&self) {
        // Synchronize with each synchronous dispatch / carrier poll. After this
        // returns no queued old packet can start another underlying I/O poll.
        *lock(&self.retired) = true;
        self.changed.send_replace(true);
    }

    pub(super) async fn run<T>(&self, operation: impl Future<Output = Result<T>>) -> Result<T> {
        let mut changed = self.changed.subscribe();
        tokio::pin!(operation);
        tokio::select! {
            biased;
            _ = async {
                if !*changed.borrow_and_update() { let _ = changed.changed().await; }
            } => Err(Error::Closed),
            result = std::future::poll_fn(|cx| {
                let _guard = match self.enter() {
                    Ok(guard) => guard,
                    Err(error) => return std::task::Poll::Ready(Err(error)),
                };
                operation.as_mut().poll(cx)
            }) => result,
        }
    }

    pub(super) async fn cancelled(&self) {
        let mut changed = self.changed.subscribe();
        if !*changed.borrow_and_update() {
            let _ = changed.changed().await;
        }
    }
}

/// Owned by the unique packet pump, including while a caller drops its future.
/// Retires local queues on Drop; it never destroys the Runtime's E2E caches.
pub(crate) struct Generation(pub(crate) Arc<Fence>);
impl Drop for Generation {
    fn drop(&mut self) {
        self.0.retire();
    }
}
