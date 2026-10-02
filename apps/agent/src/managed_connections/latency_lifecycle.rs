//! The managed loop owns exactly one fenced sampler. Aborting and awaiting the
//! child is the explicit cancellation boundary; Drop also requests abortion.
use super::{
    Manager, Phase,
    latency_history::{self, Source, Summary},
};
use meshway_control_client::HttpClient;
use tokio::{
    task::JoinHandle,
    time::{Duration, Instant},
};

#[derive(PartialEq, Eq)]
struct Key {
    epoch: u32,
    publication: u64,
    revision: u32,
    digest: String,
}

struct Task {
    key: Key,
    handle: JoinHandle<Summary>,
}

fn harvest(result: Result<Summary, tokio::task::JoinError>) {
    if let Ok(summary) = result {
        // Harvest without synchronous logging/pipe I/O in the health loop.
        // These counters are partial telemetry diagnostics, never network loss.
        let _counts = (
            summary.sampled,
            summary.queue_dropped,
            summary.report_dropped,
        );
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub(super) struct Owner {
    task: Option<Task>,
    attempted: Option<Key>,
    retry_at: Instant,
    backoff: Duration,
}

impl Default for Owner {
    fn default() -> Self {
        Self {
            task: None,
            attempted: None,
            retry_at: Instant::now(),
            backoff: Duration::from_secs(30),
        }
    }
}

impl Owner {
    pub async fn stop(&mut self) {
        if let Some(mut task) = self.task.take() {
            task.handle.abort();
            // Join completes after probe/report futures and their queues drop.
            harvest((&mut task.handle).await);
        }
        self.attempted = None;
        self.backoff = Duration::from_secs(30);
        self.retry_at = Instant::now();
    }

    pub async fn before_snapshot(&mut self, revision: u32, digest: &str) {
        if self
            .task
            .as_ref()
            .is_some_and(|task| task.key.revision != revision || task.key.digest != digest)
        {
            self.stop().await;
        }
    }

    async fn accepted(&mut self, source: Source, client: HttpClient) {
        let key = Key {
            epoch: source.epoch,
            publication: source.publication,
            revision: source.snapshot.payload.revision,
            digest: source.snapshot.digest.clone(),
        };
        if self
            .task
            .as_ref()
            .is_some_and(|task| task.key == key && !task.handle.is_finished())
        {
            return;
        }
        let same = self.attempted.as_ref() == Some(&key);
        if let Some(mut task) = self.task.take() {
            task.handle.abort();
            harvest((&mut task.handle).await);
        }
        if !same {
            self.backoff = Duration::from_secs(30);
            self.retry_at = Instant::now();
        }
        if same && Instant::now() < self.retry_at {
            return;
        }
        // Even a fast permanent HTTP failure cannot spawn once per heartbeat.
        self.retry_at = Instant::now() + self.backoff;
        self.backoff = (self.backoff * 2).min(Duration::from_secs(300));
        self.attempted = Some(Key {
            epoch: key.epoch,
            publication: key.publication,
            revision: key.revision,
            digest: key.digest.clone(),
        });
        self.task = Some(Task {
            key,
            handle: tokio::spawn(latency_history::run(source, client)),
        });
    }
}

impl Manager {
    /// Only the accepted, still-current Applied report may start/recover a job.
    /// This client is the very one used for the accepted runtime HTTP report.
    pub(crate) async fn latency_report_accepted(
        &mut self,
        epoch: u32,
        state: Phase,
        client: HttpClient,
    ) {
        if state != Phase::Applied {
            self.stop_latency_history().await;
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let Some(source) = self.prober().history_source(snapshot, epoch) else {
            self.stop_latency_history().await;
            return;
        };
        self.latency.accepted(source, client).await;
    }

    pub(crate) async fn stop_latency_history(&mut self) {
        self.latency.stop().await;
    }

    pub(crate) async fn fence_latency_snapshot(
        &mut self,
        snapshot: &meshway_protocol::AgentConnectionsSnapshot,
    ) {
        if !snapshot.payload.valid {
            self.stop_latency_history().await;
            return;
        }
        self.latency
            .before_snapshot(snapshot.payload.revision, &snapshot.digest)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Witness(Arc<AtomicUsize>);
    impl Drop for Witness {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    async fn occupied(counter: Arc<AtomicUsize>) -> Task {
        let (started, ready) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _witness = Witness(counter);
            let _ = started.send(());
            std::future::pending::<()>().await;
            Summary::default()
        });
        ready.await.unwrap();
        Task {
            key: Key {
                epoch: 1,
                publication: 1,
                revision: 1,
                digest: "a".repeat(64),
            },
            handle,
        }
    }
    #[tokio::test]
    async fn explicit_stop_joins_and_owner_drop_does_not_detach_child() {
        let counter = Arc::new(AtomicUsize::new(0));
        let mut owner = Owner {
            task: Some(occupied(counter.clone()).await),
            ..Owner::default()
        };
        owner.stop().await;
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "explicit boundary must have dropped child futures"
        );
        owner.task = Some(occupied(counter.clone()).await);
        drop(owner);
        tokio::time::timeout(Duration::from_secs(1), async {
            while counter.load(Ordering::SeqCst) != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
