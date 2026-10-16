//! One read-back of an owned kernel object against its receipt, and how
//! long ownership may stay unproved. A read that fails or is abandoned proves
//! nothing about ownership; only a read-back that differs ends it at once.
use crate::Error;
use std::time::{Duration, Instant};

/// One read-back of an owned object against its receipt.
#[derive(Debug)]
pub(super) enum Verification {
    /// Read back and identical.
    Holds,
    /// The read failed; ownership is unknown, not lost.
    Unavailable(Error),
    /// Read back and different, or gone: what changed.
    Changed(&'static str),
}

impl std::fmt::Display for Verification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Holds => f.write_str("read back and identical"),
            Self::Unavailable(error) => write!(f, "read back failed: {error}"),
            Self::Changed(what) => write!(f, "read back and different: {what}"),
        }
    }
}

impl Verification {
    /// The ownership proof as a result: unknown keeps its read error.
    pub fn result(self) -> crate::Result<()> {
        match self {
            Self::Holds => Ok(()),
            Self::Unavailable(error) => Err(error),
            Self::Changed(_) => Err(Error::Conflict),
        }
    }
}

/// How long an owned object may stay unreadable before its owner fails
/// closed: the DNS owner's recovery horizon.
pub(super) const UNVERIFIED_LIMIT: Duration = Duration::from_secs(30);

/// The owner whose periodic verifications are counted (its log lines).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Owner {
    #[default]
    Egress,
    Exit,
}

/// Consecutive periodic verifications that could not read the object.
#[derive(Debug, Default)]
pub(super) struct Unverified {
    owner: Owner,
    since: Option<Instant>,
    failures: u32,
}

impl Unverified {
    pub fn new(owner: Owner) -> Self {
        Self {
            owner,
            ..Self::default()
        }
    }

    /// Record one verification: `Err` names why ownership ends now.
    pub fn record(
        &mut self,
        verification: &Verification,
        now: Instant,
    ) -> std::result::Result<(), &'static str> {
        match verification {
            Verification::Holds => {
                if let Some(since) = self.since.take() {
                    let failures = self.failures;
                    let unverified_ms = now.duration_since(since).as_millis() as u64;
                    match self.owner {
                        Owner::Egress => tracing::info!(
                            failures,
                            unverified_ms,
                            "Egress capture verified again; ownership kept"
                        ),
                        Owner::Exit => tracing::info!(
                            failures,
                            unverified_ms,
                            "Exit role audit verified again; ownership kept"
                        ),
                    }
                }
                self.failures = 0;
                Ok(())
            }
            Verification::Unavailable(error) => {
                let since = *self.since.get_or_insert(now);
                self.failures = self.failures.saturating_add(1);
                if self.failures == 1 {
                    let limit_s = UNVERIFIED_LIMIT.as_secs();
                    match self.owner {
                        Owner::Egress => tracing::warn!(
                            error = %error,
                            limit_s,
                            "Egress capture verification unavailable; retrying, nothing new installed meanwhile"
                        ),
                        Owner::Exit => tracing::warn!(
                            error = %error,
                            limit_s,
                            "Exit role audit unavailable; retrying, the role and its flows kept"
                        ),
                    }
                }
                if now.duration_since(since) >= UNVERIFIED_LIMIT {
                    Err("verify_unavailable")
                } else {
                    Ok(())
                }
            }
            Verification::Changed(_) => Err("verify"),
        }
    }

    pub fn unknown(&self) -> bool {
        self.since.is_some()
    }
}

/// What the owner reads back after one of its kernel changes failed. A failed
/// change proves neither outcome: the tool may have timed out after the
/// kernel applied it, or the kernel may never have seen it.
#[derive(Debug)]
pub(super) enum Reread {
    /// Read back at the change's target: it completed. Running it again only
    /// reads and records it.
    Done,
    /// Read back at its start, or between start and target on the change's
    /// own path (each object at one of the two): it is due again.
    Start,
    /// The read failed: the outcome is still unknown.
    Unavailable(Error),
    /// Read back as neither: what differs. This is an inconsistency.
    Changed(&'static str),
}

impl Reread {
    /// A read-back of the whole owned object set, where only `Holds` says
    /// nothing about the change itself: it is still due.
    pub fn of(verification: Verification, done: bool) -> Self {
        match verification {
            Verification::Holds if done => Self::Done,
            Verification::Holds => Self::Start,
            Verification::Unavailable(error) => Self::Unavailable(error),
            Verification::Changed(what) => Self::Changed(what),
        }
    }
}

impl std::fmt::Display for Reread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Done => f.write_str("target"),
            Self::Start => f.write_str("start"),
            Self::Unavailable(error) => write!(f, "unavailable: {error}"),
            Self::Changed(what) => write!(f, "changed: {what}"),
        }
    }
}

/// First wait before a failed change runs again, and the longest.
const CHANGE_RETRY: Duration = Duration::from_secs(1);
const CHANGE_RETRY_MAX: Duration = Duration::from_secs(60);

/// One failed kernel change its owner retries: from its first failure it
/// runs again after each reread that allows it (1 s, doubling, capped at
/// 60 s), for at most [`UNVERIFIED_LIMIT`]. Then, or at once on a read-back
/// that is neither start nor target, the owner fails closed as before.
#[derive(Debug)]
pub(super) struct Retry {
    owner: Owner,
    operation: &'static str,
    since: Instant,
    attempts: u32,
    wait: Duration,
    reason: String,
    retry_at: Option<Instant>,
    /// A reread found the target already once.
    done: bool,
}

impl Retry {
    /// The first failed attempt of `operation`.
    pub fn new(owner: Owner, operation: &'static str, error: &Error, now: Instant) -> Self {
        Self {
            owner,
            operation,
            since: now,
            attempts: 1,
            wait: CHANGE_RETRY,
            reason: error.to_string(),
            retry_at: None,
            done: false,
        }
    }

    /// Another attempt failed with `error`.
    pub fn failed(&mut self, error: &Error) {
        self.attempts = self.attempts.saturating_add(1);
        self.reason = error.to_string();
    }

    /// Record the reread after the last failed attempt. `Ok(wait)`: run the
    /// change again after `wait`; `Err(check)`: the owner fails closed.
    pub fn record(
        &mut self,
        reread: &Reread,
        now: Instant,
    ) -> std::result::Result<Duration, &'static str> {
        let elapsed = now.duration_since(self.since);
        let left = UNVERIFIED_LIMIT.saturating_sub(elapsed);
        let wait = match reread {
            Reread::Changed(_) => return Err("verify"),
            Reread::Done | Reread::Start if left.is_zero() => return Err("change_unavailable"),
            // Recorded at once the first time; a recording that fails again
            // waits like any other attempt.
            Reread::Done if !std::mem::replace(&mut self.done, true) => Duration::ZERO,
            Reread::Unavailable(_) if left.is_zero() => return Err("verify_unavailable"),
            Reread::Done | Reread::Start | Reread::Unavailable(_) => {
                let wait = self.wait.min(left);
                self.wait =
                    meshway_model::Backoff::new(CHANGE_RETRY, CHANGE_RETRY_MAX).next(self.wait);
                wait
            }
        };
        if self.attempts == 1 || self.attempts.is_power_of_two() {
            let (operation, attempts, reason) = (self.operation, self.attempts, &self.reason);
            let retry_ms = wait.as_millis() as u64;
            match self.owner {
                Owner::Egress => {
                    tracing::warn!(operation, attempts, error = %reason, %reread, retry_ms,
                    "Egress capture change failed; read back, retrying")
                }
                Owner::Exit => {
                    tracing::warn!(operation, attempts, error = %reason, %reread, retry_ms,
                    "Exit role change failed; read back, retrying")
                }
            }
        }
        self.retry_at = Some(now + wait);
        Ok(wait)
    }

    /// The change completed after `attempts` failed ones.
    pub fn completed(&self) {
        let (operation, attempts) = (self.operation, self.attempts);
        let elapsed_ms = self.since.elapsed().as_millis() as u64;
        match self.owner {
            Owner::Egress => tracing::info!(
                operation,
                attempts,
                elapsed_ms,
                "Egress capture change completed after retrying"
            ),
            Owner::Exit => tracing::info!(
                operation,
                attempts,
                elapsed_ms,
                "Exit role change completed after retrying"
            ),
        }
    }

    /// The Status fact of this retry.
    pub fn pending(&self) -> crate::KernelChangePending {
        let retry_at = self
            .retry_at
            .map(|at| at.saturating_duration_since(Instant::now()))
            .unwrap_or_default();
        crate::KernelChangePending {
            owner: match self.owner {
                Owner::Egress => crate::KernelChangeOwner::EgressCapture,
                Owner::Exit => crate::KernelChangeOwner::Exit,
            },
            operation: self.operation.to_owned(),
            retry: crate::RetryPending {
                reason: self.reason.clone(),
                attempts: self.attempts,
                retry_at: std::time::SystemTime::now() + retry_at,
            },
        }
    }
}

/// One step of an owner's change or reread, borrowing its state.
pub(super) type Boxed<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Run `change` until it succeeds; after each failure `reread` decides:
/// target → done (the change runs once more, only reading and recording);
/// start or on its own path → due again after the backoff, within
/// [`UNVERIFIED_LIMIT`]; read failed → unknown, retried within the same
/// limit; anything else → the error and the failed check, at once. The
/// pending retry is published on `changes` while it lasts; any of `stops`
/// ends the wait with the last error. A deterministic refusal (no kernel
/// operation's outcome is in doubt) is returned at once as `refused`.
pub(super) async fn retried<S, F, R>(
    state: &mut S,
    owner: Owner,
    operation: &'static str,
    changes: &tokio::sync::watch::Sender<Option<crate::KernelChangePending>>,
    stops: &[&tokio_util::sync::CancellationToken],
    mut change: F,
    mut reread: R,
) -> std::result::Result<(), (Error, &'static str)>
where
    F: for<'a> FnMut(&'a mut S) -> Boxed<'a, crate::Result<()>>,
    R: for<'a> FnMut(&'a mut S) -> Boxed<'a, Reread>,
{
    let error = match change(state).await {
        Ok(()) => return Ok(()),
        Err(error) if !uncertain(&error) => return Err((error, "refused")),
        Err(error) => error,
    };
    let stopped = async {
        futures_util::future::select_all(stops.iter().map(|stop| Box::pin(stop.cancelled()))).await;
    };
    tokio::pin!(stopped);
    let mut retry = Retry::new(owner, operation, &error, Instant::now());
    let mut last = error;
    loop {
        let observed = reread(state).await;
        #[cfg(test)]
        eprintln!("{operation} failed: {last}; read back {observed}");
        let wait = match retry.record(&observed, Instant::now()) {
            Ok(wait) => wait,
            Err(check) => {
                clear(changes, owner);
                return Err((last, check));
            }
        };
        changes.send_replace(Some(retry.pending()));
        if !wait.is_zero() {
            tokio::select! { biased;
                () = &mut stopped => {
                    clear(changes, owner);
                    return Err((last, "stopped"));
                }
                () = tokio::time::sleep(wait) => {}
            }
        }
        match change(state).await {
            Ok(()) => {
                retry.completed();
                clear(changes, owner);
                return Ok(());
            }
            Err(error) if !uncertain(&error) => {
                clear(changes, owner);
                return Err((error, "refused"));
            }
            Err(error) => {
                retry.failed(&error);
                last = error;
            }
        }
    }
}

/// Withdraw this owner's pending change; another owner's stays.
fn clear(changes: &tokio::sync::watch::Sender<Option<crate::KernelChangePending>>, owner: Owner) {
    let owner = match owner {
        Owner::Egress => crate::KernelChangeOwner::EgressCapture,
        Owner::Exit => crate::KernelChangeOwner::Exit,
    };
    changes.send_if_modified(|current| {
        let mine = current
            .as_ref()
            .is_some_and(|pending| pending.owner == owner);
        if mine {
            *current = None;
        }
        mine
    });
}

/// Whether a failed change leaves its kernel outcome in doubt: a kernel
/// tool or netlink operation failed, timed out, or read back unexpectedly.
/// Refusals before any kernel work (expired authority, target, capacity)
/// are certain.
pub(super) fn uncertain(error: &Error) -> bool {
    matches!(error, Error::Netlink | Error::Conflict | Error::Io(_))
}

/// A periodic duty of an owner loop (an audit, maintenance): due a full
/// period after its last run ended, never at once again after a run that
/// took longer than the period. Requests queued behind a slow but successful
/// duty are therefore served between its runs.
pub(super) struct Cadence {
    period: Duration,
    at: tokio::time::Instant,
}

impl Cadence {
    /// The first run is due now.
    pub fn new(period: Duration) -> Self {
        Self {
            period,
            at: tokio::time::Instant::now(),
        }
    }
    pub async fn due(&self) {
        tokio::time::sleep_until(self.at).await;
    }
    /// The run that was due ended now.
    pub fn ran(&mut self) {
        self.at = tokio::time::Instant::now() + self.period;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F6a B2: an audit that always takes 1.5 s against a 1 s period, ahead
    /// of requests in a biased owner loop (stop first, then the audit, then
    /// requests): every request is still answered, one between audits.
    #[tokio::test(start_paused = true)]
    async fn requests_are_served_between_slow_but_successful_audits() {
        let (sender, mut requests) =
            tokio::sync::mpsc::channel::<tokio::sync::oneshot::Sender<()>>(8);
        let stop = tokio_util::sync::CancellationToken::new();
        let owner = tokio::spawn({
            let stop = stop.clone();
            async move {
                let mut audit = Cadence::new(Duration::from_secs(1));
                let mut audits = 0u32;
                loop {
                    tokio::select! { biased;
                        _ = stop.cancelled() => break,
                        _ = audit.due() => {
                            tokio::time::sleep(Duration::from_millis(1500)).await;
                            audit.ran();
                            audits += 1;
                        },
                        request = requests.recv() => {
                            let Some(reply) = request else { break };
                            let _ = reply.send(());
                        },
                    }
                }
                audits
            }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        for _ in 0..5 {
            let (reply, answered) = tokio::sync::oneshot::channel();
            sender.send(reply).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), answered)
                .await
                .expect("a request queued behind a slow audit is answered")
                .unwrap();
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
        stop.cancel();
        assert!(
            owner.await.unwrap() >= 2,
            "the audits kept running meanwhile"
        );
    }

    /// Failed reads keep ownership and are retried; a consistent read-back
    /// clears them. Only a differing read-back, or reads failing for the
    /// whole limit, ends ownership.
    #[test]
    fn unreadable_verifications_keep_ownership_until_the_limit_and_a_change_ends_it() {
        for owner in [Owner::Egress, Owner::Exit] {
            let start = Instant::now();
            let mut unverified = Unverified::new(owner);
            for second in 0..5 {
                let at = start + Duration::from_secs(second);
                let unavailable = Verification::Unavailable(Error::Netlink);
                assert_eq!(unverified.record(&unavailable, at), Ok(()));
                assert!(unverified.unknown());
            }
            let at = start + Duration::from_secs(5);
            assert_eq!(unverified.record(&Verification::Holds, at), Ok(()));
            assert!(!unverified.unknown());
            let changed = Verification::Changed("objects_changed");
            assert_eq!(unverified.record(&changed, at), Err("verify"));

            let mut unverified = Unverified::new(owner);
            let unavailable = Verification::Unavailable(Error::Netlink);
            assert_eq!(unverified.record(&unavailable, start), Ok(()));
            let before = start + UNVERIFIED_LIMIT - Duration::from_millis(1);
            assert_eq!(unverified.record(&unavailable, before), Ok(()));
            assert_eq!(
                unverified.record(&unavailable, start + UNVERIFIED_LIMIT),
                Err("verify_unavailable")
            );
        }
    }

    /// X12: after a failed change, a read-back at the target is recorded at
    /// once; at the start (or on its path) or unreadable, it runs again
    /// after 1 s, doubling, until 30 s from the first failure; a read-back
    /// that is neither fails closed at once.
    #[test]
    fn a_failed_change_is_classified_by_its_reread_and_bounded() {
        let start = Instant::now();
        let error = Error::Netlink;
        let mut retry = Retry::new(Owner::Egress, "pause", &error, start);
        assert_eq!(retry.record(&Reread::Done, start), Ok(Duration::ZERO));
        let waits: Vec<_> = (0..5)
            .map(|second| {
                let reread = if second % 2 == 0 {
                    Reread::Start
                } else {
                    Reread::Unavailable(Error::Netlink)
                };
                retry
                    .record(&reread, start + Duration::from_secs(second))
                    .unwrap()
                    .as_secs()
            })
            .collect();
        assert_eq!(waits, [1, 2, 4, 8, 16]);
        // Bounded by what is left of the limit; a second target read waits.
        let late = start + Duration::from_secs(25);
        assert_eq!(
            retry.record(&Reread::Done, late),
            Ok(Duration::from_secs(5))
        );
        let end = start + UNVERIFIED_LIMIT;
        assert_eq!(retry.record(&Reread::Start, end), Err("change_unavailable"));
        let unread = Reread::Unavailable(Error::Netlink);
        assert_eq!(retry.record(&unread, end), Err("verify_unavailable"));
        assert_eq!(retry.record(&Reread::Done, end), Err("change_unavailable"));
        let mut retry = Retry::new(Owner::Exit, "retire", &error, start);
        assert_eq!(
            retry.record(&Reread::Changed("objects_changed"), start),
            Err("verify")
        );
        let pending = retry.pending();
        assert_eq!(pending.owner, crate::KernelChangeOwner::Exit);
        assert_eq!(
            (pending.operation.as_str(), pending.retry.attempts),
            ("retire", 1)
        );
        assert!(uncertain(&Error::Netlink) && uncertain(&Error::Conflict));
        assert!(!uncertain(&Error::Expired) && !uncertain(&Error::Target));
    }

    /// The owner's loop: a change that failed although the kernel applied
    /// it completes on the target read-back; one the kernel never saw runs
    /// again and completes; a read-back that differs and a certain refusal
    /// end it at once. The pending fact is published while it lasts.
    #[tokio::test]
    async fn retried_changes_complete_from_the_target_or_the_start_and_publish_their_retry() {
        struct Kernel {
            applied: bool,
            fail: u32,
            runs: u32,
            reread: fn(&Kernel) -> Reread,
        }
        fn run(kernel: &mut Kernel) -> Boxed<'_, crate::Result<()>> {
            Box::pin(async move {
                kernel.runs += 1;
                if kernel.fail > 0 {
                    kernel.fail -= 1;
                    return Err(Error::Netlink);
                }
                kernel.applied = true;
                Ok(())
            })
        }
        fn refuse(_: &mut Kernel) -> Boxed<'_, crate::Result<()>> {
            Box::pin(async { Err(Error::Expired) })
        }
        fn reread(kernel: &mut Kernel) -> Boxed<'_, Reread> {
            let reread = (kernel.reread)(kernel);
            Box::pin(async move { reread })
        }
        fn at(kernel: &Kernel) -> Reread {
            if kernel.applied {
                Reread::Done
            } else {
                Reread::Start
            }
        }
        let (changes, mut seen) = tokio::sync::watch::channel(None);
        let stop = tokio_util::sync::CancellationToken::new();
        let stops = [&stop];
        // Failed, applied anyway (the tool's answer was lost): one reading run.
        let mut lost = Kernel {
            applied: true,
            fail: 1,
            runs: 0,
            reread: at,
        };
        let started = Instant::now();
        let result = retried(
            &mut lost,
            Owner::Egress,
            "pause",
            &changes,
            &stops,
            run,
            reread,
        );
        assert!(result.await.is_ok());
        assert_eq!(lost.runs, 2);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "recorded at once"
        );
        // Failed, never applied: due again after 1 s.
        let mut unseen = Kernel {
            applied: false,
            fail: 1,
            runs: 0,
            reread: at,
        };
        let result = retried(
            &mut unseen,
            Owner::Egress,
            "stage",
            &changes,
            &stops,
            run,
            reread,
        );
        let waiter = async {
            seen.changed().await.unwrap();
            seen.borrow().clone().unwrap()
        };
        let (result, pending) = tokio::join!(result, waiter);
        assert!(result.is_ok() && unseen.applied);
        assert_eq!(
            (pending.operation.as_str(), pending.retry.attempts),
            ("stage", 1)
        );
        assert!(changes.borrow().is_none(), "withdrawn once completed");
        // Read back as neither: fails closed at once.
        let mut foreign = Kernel {
            applied: false,
            fail: 1,
            runs: 0,
            reread: |_| Reread::Changed("objects_changed"),
        };
        let result = retried(
            &mut foreign,
            Owner::Exit,
            "retire",
            &changes,
            &stops,
            run,
            reread,
        );
        assert!(matches!(result.await, Err((Error::Netlink, "verify"))));
        assert_eq!(foreign.runs, 1);
        // A certain refusal is not retried.
        let mut kernel = Kernel {
            applied: false,
            fail: 0,
            runs: 0,
            reread: at,
        };
        let result = retried(
            &mut kernel,
            Owner::Exit,
            "renew",
            &changes,
            &stops,
            refuse,
            reread,
        );
        assert!(matches!(result.await, Err((Error::Expired, "refused"))));
        // Stopping ends the wait.
        stop.cancel();
        let mut kernel = Kernel {
            applied: false,
            fail: 1,
            runs: 0,
            reread: at,
        };
        let result = retried(
            &mut kernel,
            Owner::Egress,
            "switch",
            &changes,
            &stops,
            run,
            reread,
        );
        assert!(matches!(result.await, Err((Error::Netlink, "stopped"))));
        assert!(changes.borrow().is_none());
    }
}
