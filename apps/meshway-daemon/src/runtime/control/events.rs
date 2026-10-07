//! Host-owned single watch task, bounded delivery and independent reconnect delay.
use meshway_control::{Control, EventNotice};
use meshway_ipc::EventStreamState;
use std::time::Duration;
use tokio::sync::watch;
use tokio::{sync::mpsc, task::JoinHandle, time::Instant};

#[derive(Clone, Default)]
pub(in crate::runtime) struct EventStatus {
    pub state: EventStreamState,
    pub last_event_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub(super) struct Events {
    task: Option<JoinHandle<bool>>,
    receiver: mpsc::Receiver<EventNotice>,
    sender: mpsc::Sender<EventNotice>,
    retry_at: Instant,
    backoff: u64,
    status: watch::Sender<EventStatus>,
}

impl Default for Events {
    fn default() -> Self {
        Self::new(watch::channel(EventStatus::default()).0)
    }
}

impl Events {
    pub fn new(status: watch::Sender<EventStatus>) -> Self {
        let (sender, receiver) = mpsc::channel(1);
        Self {
            task: None,
            sender,
            receiver,
            retry_at: Instant::now(),
            backoff: 1,
            status,
        }
    }

    pub async fn advance(&mut self, control: &Control) {
        if self.task.as_ref().is_some_and(JoinHandle::is_finished) {
            self.status
                .send_modify(|value| value.state = EventStreamState::Reconnecting);
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
            let status = self.status.clone();
            status.send_modify(|value| value.state = EventStreamState::Reconnecting);
            self.task = Some(tokio::spawn(async move {
                let started = Instant::now();
                match stream.connect().await {
                    Ok(true) => {
                        status.send_modify(|value| value.state = EventStreamState::Connected)
                    }
                    Ok(false) => return false,
                    Err(notice) => {
                        let _ = sender.send(*notice).await;
                        return false;
                    }
                }
                while let Some(notice) = stream.next().await {
                    if notice.received_event() {
                        status.send_modify(|value| value.last_event_at = Some(chrono::Utc::now()));
                    }
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
                        status.send_modify(|value| value.state = EventStreamState::Reconnecting);
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
                status.send_modify(|value| value.state = EventStreamState::Reconnecting);
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
        self.status
            .send_modify(|value| value.state = EventStreamState::Disconnected);
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
