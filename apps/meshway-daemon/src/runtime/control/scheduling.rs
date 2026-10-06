//! A routine cycle deadline closes admission, not an already-admitted RPC.
//! The operation owns its original timeout; lifecycle changes can still abort it.
use std::{future::Future, time::Duration};
use tokio::time::Instant;

pub(super) enum Wake<T> {
    Changed,
    Due,
    Work(T),
}

pub(super) enum Report<T> {
    Refresh,
    Sent(T),
}

pub(super) struct Window {
    deadline: Option<Instant>,
    remaining: u8,
}

impl Window {
    /// Only an installed/current application's resource notification takes the
    /// direct State path. This preserves the routine deadline and RPC budget.
    pub async fn report<T>(
        &mut self,
        ready: bool,
        changed: impl Future<Output = ()>,
        report: impl Future<Output = T>,
    ) -> Wake<Report<T>> {
        if !ready {
            return Wake::Work(Report::Refresh);
        }
        match self.serve(changed, report).await {
            Wake::Changed => Wake::Changed,
            Wake::Due => Wake::Due,
            Wake::Work(value) => Wake::Work(Report::Sent(value)),
        }
    }
    pub fn new(delay: Option<Duration>) -> Self {
        Self {
            deadline: delay.map(|delay| Instant::now() + delay),
            remaining: 32,
        }
    }

    pub async fn wait<T>(
        &self,
        changed: impl Future<Output = ()>,
        work: impl Future<Output = T>,
    ) -> Wake<T> {
        if self.remaining == 0 {
            return Wake::Due;
        }
        tokio::select! { biased;
            _ = changed => Wake::Changed,
            _ = async { match self.deadline { Some(deadline) => tokio::time::sleep_until(deadline).await, None => std::future::pending().await } } => Wake::Due,
            result = work => Wake::Work(result),
        }
    }

    pub async fn serve<T>(
        &mut self,
        changed: impl Future<Output = ()>,
        operation: impl Future<Output = T>,
    ) -> Wake<T> {
        self.remaining = self.remaining.saturating_sub(1);
        tokio::select! { biased;
            _ = changed => Wake::Changed,
            result = operation => Wake::Work(result),
        }
    }
}

pub(super) fn reports_enabled(phase: meshway_ipc::Phase) -> bool {
    matches!(phase, meshway_ipc::Phase::ControlReady)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn current_state_notification_reports_before_routine_refresh_without_moving_it() {
        let mut window = Window::new(Some(Duration::from_secs(5)));
        let deadline = window.deadline;
        let sampled = Arc::new(AtomicBool::new(false));
        let sample = sampled.clone();
        let result = window
            .report(true, std::future::pending(), async move {
                sample.store(true, Ordering::SeqCst);
                7u64
            })
            .await;
        assert!(matches!(result, Wake::Work(Report::Sent(7))));
        assert!(sampled.load(Ordering::SeqCst));
        assert_eq!(window.deadline, deadline);
        assert_eq!(window.remaining, 31);
        assert!(matches!(
            window.wait(std::future::pending(), async {}).await,
            Wake::Work(())
        ));
    }

    #[tokio::test]
    async fn uninstalled_or_stale_state_requires_refresh_without_polling_a_sample() {
        let mut window = Window::new(Some(Duration::from_secs(5)));
        let sampled = AtomicBool::new(false);
        assert!(matches!(
            window
                .report(false, std::future::pending(), async {
                    sampled.store(true, Ordering::SeqCst);
                })
                .await,
            Wake::Work(Report::Refresh)
        ));
        assert!(!sampled.load(Ordering::SeqCst));
        assert_eq!(window.remaining, 32);
    }

    struct Aborted {
        marker: Arc<AtomicBool>,
        armed: bool,
    }
    impl Drop for Aborted {
        fn drop(&mut self) {
            if self.armed {
                self.marker.store(true, Ordering::SeqCst);
            }
        }
    }

    #[tokio::test]
    async fn cycle_due_does_not_cancel_an_admitted_operation_or_admit_the_next_one() {
        let mut window = Window::new(Some(Duration::from_millis(20)));
        let due = window.deadline.unwrap();
        let (started, starting) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let marker = cancelled.clone();
        let operation = async move {
            let mut guard = Aborted {
                marker,
                armed: true,
            };
            started.send(()).unwrap();
            let value = tokio::time::timeout(Duration::from_secs(2), released)
                .await
                .unwrap()
                .unwrap();
            guard.armed = false;
            value
        };
        let drive = async {
            starting.await.unwrap();
            tokio::time::sleep_until(due + Duration::from_millis(5)).await;
            assert!(!cancelled.load(Ordering::SeqCst));
            release.send(42).unwrap();
        };
        let (result, ()) = tokio::join!(window.serve(std::future::pending(), operation), drive);
        assert!(matches!(result, Wake::Work(42)));
        assert_eq!(window.deadline, Some(due));
        assert!(matches!(
            window.wait(std::future::pending(), async { 43 }).await,
            Wake::Due
        ));
    }

    #[tokio::test]
    async fn original_rpc_timeout_and_lifecycle_cancellation_still_end_work() {
        let mut window = Window::new(Some(Duration::ZERO));
        let deadline = Instant::now() + Duration::from_millis(20);
        let result = window
            .serve(
                std::future::pending(),
                tokio::time::timeout_at(deadline, std::future::pending::<()>()),
            )
            .await;
        assert!(matches!(result, Wake::Work(Err(_))));
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Aborted {
            marker: dropped.clone(),
            armed: true,
        };
        let operation = async move {
            let _guard = guard;
            std::future::pending::<()>().await
        };
        assert!(matches!(
            window.serve(async {}, operation).await,
            Wake::Changed
        ));
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn immediate_rpc_pressure_cannot_reset_the_window_or_exceed_its_budget() {
        let mut window = Window::new(None);
        for i in 0..32 {
            assert!(matches!(
                window.wait(std::future::pending(), async { i }).await,
                Wake::Work(_)
            ));
            assert!(matches!(
                window.serve(std::future::pending(), async { i }).await,
                Wake::Work(_)
            ));
        }
        assert!(matches!(
            window.wait(std::future::pending(), async {}).await,
            Wake::Due
        ));
    }

    #[test]
    fn dirty_runtime_reports_cannot_poll_enrollment_before_a_reportable_instance() {
        use meshway_ipc::Phase;
        assert!(!reports_enabled(Phase::AwaitingApproval));
        assert!(!reports_enabled(Phase::Starting));
        assert!(!reports_enabled(Phase::Retrying));
        assert!(reports_enabled(Phase::ControlReady));
    }
}
