//! The worker's timers: bounded retry of a transient installation failure,
//! the lease end, and an optional deadline.
use super::*;

/// Bounded retry of a transient installation failure of the same netmap.
#[derive(Default)]
pub(super) struct Retry {
    failures: u8,
    pub(super) at: Option<tokio::time::Instant>,
}
impl Retry {
    pub(super) fn failed(&mut self, failure: ApplyFailure) {
        self.failures = self.failures.saturating_add(1);
        let transient = !matches!(failure, ApplyFailure::Validation | ApplyFailure::Dataplane);
        self.at = (transient && self.failures < 3).then(|| {
            tokio::time::Instant::now()
                + Duration::from_secs(if self.failures == 1 { 5 } else { 15 })
        });
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
