//! One bounded expiry producer; accepted cleanup jobs remain SQLite-owner work.
use crate::{state::service::Runtime, store::Handle};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

pub(crate) struct Maintenance {
    stop: CancellationToken,
    worker: tokio::task::JoinHandle<()>,
}
impl Maintenance {
    pub(crate) fn start(handle: Handle, runtime: Arc<Mutex<Runtime>>) -> Self {
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let worker = tokio::spawn(async move {
            let mut timer = tokio::time::interval(std::time::Duration::from_secs(1));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    _ = cancelled.cancelled() => break,
                    _ = timer.tick() => {}
                }
                let runtime = runtime.clone();
                // Cancellation drops only the reply/admission waiter. A job
                // already accepted by Handle stays owned by the SQLite worker.
                let result = tokio::select! {
                    biased;
                    _ = cancelled.cancelled() => break,
                    result = handle.execute(move |_| {
                        if let Ok(mut runtime) = runtime.lock() {
                            let now = chrono::Utc::now().timestamp_millis();
                            if let Ok(now) = u64::try_from(now) {
                                runtime.traversal.gc(now);
                            }
                        }
                    }) => result,
                };
                if result.is_err() {
                    break;
                }
            }
        });
        Self { stop, worker }
    }
    pub(crate) async fn shutdown(mut self) -> Result<(), tokio::task::JoinError> {
        self.stop.cancel();
        (&mut self.worker).await
    }
}
impl Drop for Maintenance {
    fn drop(&mut self) {
        self.stop.cancel();
        self.worker.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Executor, Options};
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };

    #[tokio::test]
    async fn shutdown_joins_with_sql_admission_or_reply_blocked_and_accepted_work_drains() {
        // One slot keeps GC waiting for admission; two admit its job behind the
        // queued write. Both waits must respond to cancellation without aborting SQL.
        for capacity in [1, 2] {
            let directory = tempfile::tempdir().unwrap();
            let (executor, ()) = Executor::open(
                directory
                    .path()
                    .canonicalize()
                    .unwrap()
                    .join("maintenance.sqlite3"),
                Options {
                    queue_capacity: capacity,
                    busy_timeout: Duration::from_secs(1),
                },
                (),
                |db, _| db.execute_batch("CREATE TABLE accepted_work(value INTEGER)"),
            )
            .await
            .unwrap();
            let handle = executor.handle();
            let runtime = Arc::new(Mutex::new(
                Runtime::new(std::sync::Arc::new(crate::state::facts::NoNetmaps)).unwrap(),
            ));
            let before = Arc::strong_count(&runtime);
            let (entered, started) = tokio::sync::oneshot::channel();
            let (release, blocked) = std::sync::mpsc::channel();
            let committed = Arc::new(AtomicBool::new(false));
            let completed = committed.clone();
            let blocker_handle = handle.clone();
            let blocker = tokio::spawn(async move {
                blocker_handle
                    .execute(move |db| {
                        let tx = db.transaction().unwrap();
                        tx.execute("INSERT INTO accepted_work VALUES(1)", [])
                            .unwrap();
                        entered.send(()).unwrap();
                        blocked.recv_timeout(Duration::from_secs(10)).unwrap();
                        tx.commit().unwrap();
                        completed.store(true, Ordering::Release);
                    })
                    .await
                    .unwrap();
            });
            started.await.unwrap();
            let queued_handle = handle.clone();
            let (submitting, submitted) = tokio::sync::oneshot::channel();
            let queued = tokio::spawn(async move {
                submitting.send(()).unwrap();
                queued_handle
                    .execute(|db| {
                        let tx = db.transaction().unwrap();
                        tx.execute("INSERT INTO accepted_work VALUES(2)", [])
                            .unwrap();
                        tx.commit().unwrap();
                    })
                    .await
                    .unwrap();
            });
            submitted.await.unwrap();
            let maintenance = Maintenance::start(handle.clone(), runtime.clone());
            let worker = maintenance.worker.abort_handle();
            tokio::time::timeout(Duration::from_secs(3), async {
                while Arc::strong_count(&runtime) != before + 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("GC waiting on SQL");
            tokio::time::timeout(Duration::from_secs(3), maintenance.shutdown())
                .await
                .expect("GC joins before releasing SQL")
                .unwrap();
            assert!(worker.is_finished());
            assert!(!committed.load(Ordering::Acquire));
            assert_eq!(
                Arc::strong_count(&runtime),
                before + usize::from(capacity == 2)
            );
            release.send(()).unwrap();
            blocker.await.unwrap();
            queued.await.unwrap();
            let rows = handle
                .execute(|db| {
                    db.query_row("SELECT count(*) FROM accepted_work", [], |row| {
                        row.get::<_, u32>(0)
                    })
                    .unwrap()
                })
                .await
                .unwrap();
            assert_eq!(rows, 2);
            assert!(committed.load(Ordering::Acquire));
            assert_eq!(Arc::strong_count(&runtime), before);
            executor.shutdown().await.unwrap();
        }
    }
}
