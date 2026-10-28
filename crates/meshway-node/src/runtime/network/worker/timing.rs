//! The worker's timers: its own recovery of a failed installation of the
//! same netmap, the lease end, and an optional deadline.
use super::*;
use meshway_model::recovery::{Health, Horizon, Observation, Policy, Recovery};

/// First retry of a transient installation failure of the same netmap, and
/// the longest wait between two.
const APPLY_RETRY: Duration = Duration::from_secs(5);
const APPLY_RETRY_MAX: Duration = Duration::from_secs(60);

/// The worker's own recovery of an installation failure (owner #13): a
/// transient one (a component that failed to install, a failed read) is
/// retried 5 s after it, doubling to 60 s, without a total limit (it never
/// runs out into `blocked`); a deterministic refusal of the netmap
/// (validation, the data plane) is blocked until another offer arrives.
pub(super) fn apply_recovery() -> Recovery {
    Recovery::new(Policy {
        backoff: meshway_model::Backoff::new(APPLY_RETRY, APPLY_RETRY_MAX),
        horizon: Horizon::Unbounded,
        reread_blocked: None,
    })
}

/// Record one failed installation.
pub(super) fn apply_failed(recovery: &mut Recovery, failure: ApplyFailure) {
    let observation = match failure {
        ApplyFailure::Validation | ApplyFailure::Dataplane => {
            Observation::proven("apply", format!("{failure:?}"))
        }
        failure => Observation::unknown("apply", format!("{failure:?}")),
    };
    recovery.observe(observation, std::time::Instant::now());
}

/// When the failed installation runs again; `None` while none failed or a
/// refusal waits for another offer.
pub(super) fn apply_retry_at(recovery: &Recovery) -> Option<tokio::time::Instant> {
    match recovery.health() {
        Health::Retrying(retrying) => Some(tokio::time::Instant::from_std(retrying.retry_at)),
        _ => None,
    }
}

/// A reinstall that finds nothing to install ends the worker's own retry:
/// Control offers no netmap (the lease ended or was withdrawn), the offered
/// lease has already ended, or a Mesh pool overlap holds the netmap. The due
/// retry is consumed here and the same fact is not retried again; the next
/// offer (or the overlap's end) installs. Returns whether a retry or a
/// refusal was pending, i.e. whether the report's `apply` changes.
pub(super) fn nothing_to_install(recovery: &mut Recovery) -> bool {
    let pending = !recovery.health().is_healthy();
    *recovery = apply_recovery();
    pending
}

pub(super) fn lease_end(
    lease: &meshway_model::NetmapLease,
) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::from_timestamp_millis(i64::try_from(lease.expires_at_ms).ok()?)
}

pub(super) async fn sleep_until(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RC-1 M7 (G1): a transient installation failure keeps being retried
    /// (5 s doubling to 60 s), never running out; a validation refusal waits
    /// for another offer.
    #[test]
    fn a_transient_installation_failure_is_retried_without_running_out() {
        let mut recovery = apply_recovery();
        let mut waits = Vec::new();
        for _ in 0..8 {
            apply_failed(&mut recovery, ApplyFailure::Tun);
            let Health::Retrying(retrying) = recovery.health() else {
                panic!("retried")
            };
            waits.push(
                retrying
                    .retry_at
                    .saturating_duration_since(std::time::Instant::now())
                    .as_secs_f64()
                    .round() as u64,
            );
        }
        assert_eq!(waits, [5, 10, 20, 40, 60, 60, 60, 60]);
        assert!(apply_retry_at(&recovery).is_some(), "the 8th still retried");
        let mut refused = apply_recovery();
        apply_failed(&mut refused, ApplyFailure::Validation);
        assert!(refused.health().is_blocked());
        assert!(apply_retry_at(&refused).is_none());
    }

    /// SP1 (dev 2026-10-28, `meshway-vm` alpha.23: one tokio thread at 100 %
    /// for 6.5 h while the Server was unreachable): the DNS owner failed
    /// after the lease had ended, and its retry of the installation fell
    /// due once Control offered no netmap. The worker's reinstall found
    /// nothing to install and left the retry as it was: its past deadline
    /// stayed ready, came first in the biased select and the worker turned
    /// without end (a later offer was never taken). Now that turn ends the
    /// retry: in 60 s of virtual time it fires once, and Control's next
    /// offer is still taken.
    #[tokio::test(start_paused = true)]
    async fn a_retry_due_with_nothing_offered_ends_instead_of_turning() {
        let mut retry = apply_recovery();
        apply_failed(&mut retry, ApplyFailure::Dns);
        let (offers, mut offered) = tokio::sync::watch::channel::<Option<()>>(None);
        offered.borrow_and_update();
        let end = tokio::time::Instant::now() + Duration::from_secs(60);
        let later = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            offers.send_replace(Some(()));
            offers
        });
        let (mut turns, mut retries, mut taken) = (0, 0, 0);
        loop {
            turns += 1;
            assert!(
                turns <= 8,
                "the worker turns without end: {retries} retries"
            );
            // The worker's order: its own retry before Control's offers.
            tokio::select! { biased;
                _ = sleep_until(apply_retry_at(&retry)) => {
                    retries += 1;
                    if offered.borrow().is_none() {
                        nothing_to_install(&mut retry);
                    }
                }
                changed = offered.changed() => {
                    changed.expect("offers open");
                    offered.borrow_and_update();
                    taken += 1;
                }
                () = tokio::time::sleep_until(end) => break,
            }
        }
        assert_eq!((retries, taken), (1, 1));
        assert!(
            retry.health().is_healthy(),
            "no reinstall is scheduled without a netmap"
        );
        drop(later.await.expect("offer task"));
    }
}
