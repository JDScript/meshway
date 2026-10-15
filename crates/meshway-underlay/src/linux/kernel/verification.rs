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

#[cfg(test)]
mod tests {
    use super::*;

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
}
