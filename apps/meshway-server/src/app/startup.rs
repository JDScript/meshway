//! The service manager's start timeout while storage opens (migration and
//! the startup netmap compilation). One renewal task owns every extension:
//! it renews the timeout by [`BUDGET`] at least every [`RENEWAL`] for as long
//! as the opening runs, however long one step takes, and stops (joined) when
//! the opening completes or fails. Steps only change the status text.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// What one renewal grants: the start timeout runs again from then.
const BUDGET: Duration = Duration::from_secs(120);
/// How often the renewal task renews, well inside [`BUDGET`].
const RENEWAL: Duration = Duration::from_secs(30);

type Sink = Arc<dyn Fn(Duration, &str) + Send + Sync>;

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

struct State {
    status: String,
    changed: bool,
    stopped: bool,
}

/// The renewal task of one storage opening; dropping it stops and joins it.
pub(crate) struct Startup {
    shared: Arc<Shared>,
    task: Option<JoinHandle<()>>,
}

/// Updates the status the renewal task reports. [`Progress::none`] reports
/// nothing (offline commands without a service manager, tests).
#[derive(Clone, Default)]
pub(crate) struct Progress(Option<Arc<Shared>>);

impl Progress {
    pub(crate) fn none() -> Self {
        Self(None)
    }

    pub(crate) fn status(&self, status: &str) {
        let Some(shared) = &self.0 else {
            return;
        };
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.status != status {
            state.status = status
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            state.changed = true;
            shared.wake.notify_one();
        }
    }
}

impl Startup {
    /// Renews through `meshway_service::extend_startup` (nothing outside a
    /// service manager).
    pub(crate) fn start(status: &str) -> Self {
        Self::with(
            Arc::new(|budget, status: &str| meshway_service::extend_startup(budget, status)),
            BUDGET,
            RENEWAL,
            status,
        )
    }

    fn with(sink: Sink, budget: Duration, renewal: Duration, status: &str) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                status: status.to_owned(),
                changed: false,
                stopped: false,
            }),
            wake: Condvar::new(),
        });
        let task_shared = Arc::clone(&shared);
        let task = std::thread::Builder::new()
            .name("meshway-startup".into())
            .spawn(move || renew(&task_shared, sink.as_ref(), budget, renewal))
            .map_err(|error| tracing::warn!(%error, "Startup timeout renewal not started"))
            .ok();
        Self { shared, task }
    }

    pub(crate) fn progress(&self) -> Progress {
        Progress(Some(Arc::clone(&self.shared)))
    }
}

fn renew(
    shared: &Shared,
    sink: &(dyn Fn(Duration, &str) + Send + Sync),
    budget: Duration,
    renewal: Duration,
) {
    let mut state = shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    loop {
        if state.stopped {
            return;
        }
        state.changed = false;
        let status = state.status.clone();
        drop(state);
        sink(budget, &status);
        state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let deadline = std::time::Instant::now() + renewal;
        while !state.stopped && !state.changed {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            state = shared
                .wake
                .wait_timeout(state, left)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }
}

impl Drop for Startup {
    fn drop(&mut self) {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stopped = true;
        self.shared.wake.notify_one();
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// One step longer than the budget (here 1 s against 200 ms, no status
    /// change meanwhile) is renewed throughout: no gap between renewals, from
    /// the start or until the end, reaches the budget. After the opening
    /// ends nothing renews any more.
    #[test]
    fn a_step_longer_than_the_budget_is_renewed_throughout() {
        let calls = Arc::new(Mutex::new(Vec::<(Instant, String)>::new()));
        let record = Arc::clone(&calls);
        let budget = Duration::from_millis(200);
        let started = Instant::now();
        let startup = Startup::with(
            Arc::new(move |granted, status: &str| {
                assert_eq!(granted, budget);
                record
                    .lock()
                    .unwrap()
                    .push((Instant::now(), status.to_owned()));
            }),
            budget,
            Duration::from_millis(50),
            "opening",
        );
        let progress = startup.progress();
        progress.status("migrating: copying a large table");
        std::thread::sleep(Duration::from_secs(1));
        progress.status("compiling netmaps");
        std::thread::sleep(Duration::from_millis(100));
        drop(startup);
        let ended = Instant::now();
        let after = calls.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(200));
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), after, "renewed after the opening ended");
        let mut last = started;
        for (at, _) in calls.iter() {
            assert!(*at - last < budget, "a renewal gap reached the budget");
            last = *at;
        }
        assert!(ended - last < budget);
        assert!(
            calls
                .iter()
                .any(|(_, status)| status == "migrating: copying a large table")
        );
        assert_eq!(calls.last().unwrap().1, "compiling netmaps");
        // The 1 s step alone was renewed many times, not once per step.
        assert!(calls.len() >= 10, "{}", calls.len());
    }

    #[test]
    fn progress_without_a_task_reports_nothing() {
        Progress::none().status("ignored");
    }
}
