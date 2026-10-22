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
}
