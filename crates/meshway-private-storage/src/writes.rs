use tokio::task::JoinHandle;

/// One accepted private write at a time. Cancelling a waiter retains its worker;
/// the owner must settle it before shutdown or admitting the next write.
/// Serialization, validation and directory ownership belong to the caller.
pub struct Writes<E> {
    pending: Option<JoinHandle<Result<(), E>>>,
    failed: bool,
    policy: Failure<E>,
}

enum Failure<E> {
    Poison(fn() -> E),
    PerWrite(fn() -> E),
}

impl<E: Send + 'static> Writes<E> {
    /// Any worker/write failure permanently returns the owner's storage error.
    pub fn poisoning(error: fn() -> E) -> Self {
        Self {
            pending: None,
            failed: false,
            policy: Failure::Poison(error),
        }
    }

    /// Preserve each write's error. A worker join failure is terminal and leaves
    /// the worker handle retained, matching owners that cannot recover that worker.
    pub fn per_write(worker_error: fn() -> E) -> Self {
        Self {
            pending: None,
            failed: false,
            policy: Failure::PerWrite(worker_error),
        }
    }

    pub async fn settle(&mut self) -> Result<(), E> {
        if let Some(pending) = &mut self.pending {
            let result = pending.await;
            if let Failure::PerWrite(error) = self.policy {
                let result = result.map_err(|_| error())?;
                self.pending = None;
                return result;
            }
            self.pending = None;
            self.failed = !matches!(result, Ok(Ok(())));
        }
        if self.failed
            && let Failure::Poison(error) = self.policy
        {
            return Err(error());
        }
        Ok(())
    }

    pub async fn run(
        &mut self,
        work: impl FnOnce() -> Result<(), E> + Send + 'static,
    ) -> Result<(), E> {
        self.settle().await?;
        self.pending = Some(tokio::task::spawn_blocking(work));
        self.settle().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_waiter_preserves_accepted_write_and_orders_the_next_write() {
        let mut writes = Writes::poisoning(|| "storage");
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let first = completed.clone();
        {
            let run = writes.run(move || {
                entered.send(()).unwrap();
                wait.recv().unwrap();
                first.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            });
            tokio::pin!(run);
            tokio::select! { _ = started => {}, _ = &mut run => panic!("write must be blocked") }
        }
        release.send(()).unwrap();
        writes.settle().await.unwrap();
        writes
            .run(move || {
                assert!(completed.load(std::sync::atomic::Ordering::SeqCst));
                Ok(())
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn failure_policy_preserves_poisoning_and_per_write_errors() {
        let mut sticky = Writes::poisoning(|| "storage");
        assert_eq!(sticky.run(|| Err("write")).await, Err("storage"));
        assert_eq!(sticky.settle().await, Err("storage"));
        let admitted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let attempted = admitted.clone();
        assert_eq!(
            sticky
                .run(move || {
                    attempted.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                })
                .await,
            Err("storage")
        );
        assert!(!admitted.load(std::sync::atomic::Ordering::SeqCst));
        let mut retryable = Writes::per_write(|| "worker");
        assert_eq!(retryable.run(|| Err("write")).await, Err("write"));
        retryable.run(|| Ok(())).await.unwrap();
    }
}
