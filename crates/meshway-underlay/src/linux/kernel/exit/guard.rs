/// The original packet's producer has invalidated this output.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("exit packet guard invalidated")]
pub struct Invalidated;

/// A packet-specific synchronous authorization check, owned through the final
/// TUN poll. Implementations retain the producer's original packet scope.
pub trait ExitSendGuard: Send + Sync + 'static {
    fn check(&self) -> Result<(), Invalidated>;
}

pub(super) async fn checked<F: std::future::Future>(
    guard: &dyn ExitSendGuard,
    future: F,
) -> Result<F::Output, Invalidated> {
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(|cx| {
        if let Err(error) = guard.check() {
            return std::task::Poll::Ready(Err(error));
        }
        future.as_mut().poll(cx).map(Ok)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        task::Poll,
    };
    struct Guard(Arc<AtomicBool>);
    impl ExitSendGuard for Guard {
        fn check(&self) -> Result<(), Invalidated> {
            if self.0.load(Ordering::SeqCst) {
                Err(Invalidated)
            } else {
                Ok(())
            }
        }
    }
    #[tokio::test]
    async fn original_packet_guard_precedes_every_write_poll() {
        let revoked = Arc::new(AtomicBool::new(false));
        let polls = Arc::new(AtomicUsize::new(0));
        let inner = polls.clone();
        let write = std::future::poll_fn(move |_| {
            inner.fetch_add(1, Ordering::SeqCst);
            Poll::<()>::Pending
        });
        let guard = Guard(revoked.clone());
        let mut operation = std::pin::pin!(checked(&guard, write));
        assert!(futures_util::poll!(operation.as_mut()).is_pending());
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        revoked.store(true, Ordering::SeqCst);
        assert!(matches!(
            futures_util::poll!(operation.as_mut()),
            Poll::Ready(Err(Invalidated))
        ));
        assert_eq!(
            polls.load(Ordering::SeqCst),
            1,
            "invalidated packet cannot poll the writer again"
        );
        let new_guard = Guard(Arc::new(AtomicBool::new(false)));
        assert!(checked(&new_guard, std::future::ready(())).await.is_ok());
        assert!(
            guard.check().is_err(),
            "a new projection cannot renew an old packet's guard"
        );
    }
}
