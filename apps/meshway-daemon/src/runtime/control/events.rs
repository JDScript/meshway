//! Host-owned single watch task, bounded delivery and independent reconnect delay.
use meshway_control::{Control, EventNotice};
use std::time::Duration;
use tokio::{sync::mpsc, task::JoinHandle, time::Instant};

pub(super) struct Events {
    task: Option<JoinHandle<bool>>,
    receiver: mpsc::Receiver<EventNotice>,
    sender: mpsc::Sender<EventNotice>,
    retry_at: Instant,
    backoff: u64,
}

impl Default for Events {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel(1);
        Self {
            task: None,
            sender,
            receiver,
            retry_at: Instant::now(),
            backoff: 1,
        }
    }
}

impl Events {
    pub async fn advance(&mut self, control: &Control) {
        if self.task.as_ref().is_some_and(JoinHandle::is_finished) {
            if self
                .task
                .take()
                .expect("finished task")
                .await
                .unwrap_or(false)
            {
                self.backoff = 1;
            }
            self.retry_at = Instant::now() + Duration::from_secs(self.backoff);
            self.backoff = (self.backoff * 2).min(30);
        }
        if self.task.is_none()
            && Instant::now() >= self.retry_at
            && let Ok(mut stream) = control.watch_node()
        {
            let sender = self.sender.clone();
            self.task = Some(tokio::spawn(async move {
                let started = Instant::now();
                while let Some(notice) = stream.next().await {
                    if notice.refresh_requested()
                        || notice.is_terminal()
                        || matches!(notice.kind(), "sync.reset" | "config.snapshot")
                    {
                        tracing::debug!(
                            event_kind = notice.kind(),
                            refresh = notice.refresh_requested(),
                            "Node event received"
                        );
                    }
                    if notice.is_terminal() {
                        let _ = sender.send(notice).await;
                        break;
                    }
                    if notice.refresh_requested() {
                        match sender.try_send(notice) {
                            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
                            Err(mpsc::error::TrySendError::Closed(_)) => break,
                        }
                    }
                }
                started.elapsed() >= Duration::from_secs(10)
            }));
        }
    }

    pub async fn next(&mut self) -> EventNotice {
        self.receiver.recv().await.expect("host retains sender")
    }

    pub async fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
        while self.receiver.try_recv().is_ok() {}
        self.retry_at = Instant::now();
        self.backoff = 1;
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
