//! One fail-closed owner's recovery state: how its judgements of read-backs
//! and changes become a published [`Health`], and when it reads again.
//!
//! An owner judges each read-back or change itself ([`Observation`]): only
//! the owner knows what differs from what it owns. This state machine holds
//! no I/O and no clock; the owner passes the time in. It carries no
//! authority: a [`Health`] never grants, extends or narrows a permission or
//! lease, it only says whether the owner can prove its resources its own.
//!
//! - A read that failed, timed out, or a change whose outcome is unknown is
//!   [`Observation::Unknown`]: the owner is [`Health::Retrying`] and reads
//!   again after a bounded backoff. An owner with a parent (the egress
//!   capture under kernel ownership) hands an unknown that lasted its whole
//!   [`Horizon::Escalate`] to that parent, as unknown; the others retry
//!   until it holds.
//! - A read-back that differs from what the owner owns is
//!   [`Observation::Proven`]: the owner is [`Health::Blocked`]. It never
//!   overwrites what it read; it may read again slowly (`reread_blocked`)
//!   and recovers once the evidence is gone. A failed read while blocked
//!   proves nothing and keeps it blocked.
use crate::Backoff;
use std::time::{Duration, Instant, SystemTime};

/// One owner's judgement of one read-back or change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observation {
    /// Read back as owned.
    Holds,
    /// The read failed, timed out, or a change's outcome is unknown.
    Unknown { check: &'static str, reason: String },
    /// Read back and different from what this owner owns.
    Proven { check: &'static str, reason: String },
}

impl Observation {
    pub fn unknown(check: &'static str, reason: impl std::fmt::Display) -> Self {
        Self::Unknown {
            check,
            reason: reason.to_string(),
        }
    }
    pub fn proven(check: &'static str, reason: impl std::fmt::Display) -> Self {
        Self::Proven {
            check,
            reason: reason.to_string(),
        }
    }
}

/// How long an owner stays [`Health::Retrying`] on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Horizon {
    /// Unknown for this long, from the first failure, is handed to the
    /// parent owner as unknown ([`Step::Escalate`]); never blocked.
    Escalate(Duration),
    /// Retried until it holds (owners without a parent).
    Unbounded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// The pause before each further read after an unknown one.
    pub backoff: Backoff,
    pub horizon: Horizon,
    /// While blocked, read again this often (read only, never repairing);
    /// `None`: the owner reads again only when asked.
    pub reread_blocked: Option<Duration>,
}

/// An owner's unknown that it retries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retrying {
    /// The check that could not be read or settled, e.g. `verify_unavailable`.
    pub check: &'static str,
    /// The last attempt's error.
    pub reason: String,
    /// The first failure.
    pub since: Instant,
    /// Reads judged unknown so far.
    pub attempts: u32,
    pub retry_at: Instant,
}

/// An owner's proven inconsistency: it stays failed closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blocked {
    pub check: &'static str,
    /// What differs, from the last read that proved it.
    pub reason: String,
    /// When it was first proven.
    pub since: Instant,
    /// Reads since it was first proven (unknown or proven again).
    pub attempts: u32,
    /// The next read-only re-read, if the owner reads again by itself.
    pub reread_at: Option<Instant>,
}

/// What an owner publishes about its resources. It carries no authority.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Health {
    #[default]
    Healthy,
    Retrying(Retrying),
    Blocked(Blocked),
}

impl Health {
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Healthy)
    }
    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked(_))
    }
    /// The failed check, unless healthy.
    pub fn check(&self) -> Option<&'static str> {
        match self {
            Self::Healthy => None,
            Self::Retrying(retrying) => Some(retrying.check),
            Self::Blocked(blocked) => Some(blocked.check),
        }
    }
    /// When the failure began, unless healthy.
    pub fn since(&self) -> Option<Instant> {
        match self {
            Self::Healthy => None,
            Self::Retrying(retrying) => Some(retrying.since),
            Self::Blocked(blocked) => Some(blocked.since),
        }
    }
    /// The Status projection of a failure the owner reads again by itself:
    /// its reason, attempts and next read. `None` while healthy, and while
    /// blocked without a re-read of its own.
    pub fn pending(&self) -> Option<RetryPending> {
        let (reason, attempts, at) = match self {
            Self::Healthy => return None,
            Self::Retrying(retrying) => (&retrying.reason, retrying.attempts, retrying.retry_at),
            Self::Blocked(blocked) => (&blocked.reason, blocked.attempts, blocked.reread_at?),
        };
        Some(RetryPending {
            reason: reason.clone(),
            attempts,
            retry_at: wall(at),
        })
    }
}

/// A failed operation its owner retries, as Status shows it: the last
/// attempt's error, attempts so far (the first one included) and when the
/// next one runs. The owner reads again before each attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryPending {
    pub reason: String,
    pub attempts: u32,
    pub retry_at: SystemTime,
}

/// The wall-clock time of a monotonic instant of this process.
pub fn wall(at: Instant) -> SystemTime {
    let now = Instant::now();
    match at.checked_duration_since(now) {
        Some(ahead) => SystemTime::now() + ahead,
        None => SystemTime::now() - now.duration_since(at),
    }
}

/// What the owner does after an observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Healthy: nothing until the owner's next scheduled read.
    Hold,
    /// Read and judge again at this instant.
    RetryAt(Instant),
    /// The horizon passed: hand `check` to the parent owner as unknown.
    Escalate(&'static str),
    /// Proven (or still blocked): stay failed closed; read again only at
    /// the given instant, if any.
    Block(Option<Instant>),
}

/// One owner's recovery state.
#[derive(Clone, Debug)]
pub struct Recovery {
    policy: Policy,
    health: Health,
    transitioned: bool,
}

impl Recovery {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            health: Health::Healthy,
            transitioned: false,
        }
    }

    pub fn policy(&self) -> Policy {
        self.policy
    }

    pub fn health(&self) -> &Health {
        &self.health
    }

    /// Whether the last call changed the kind of health (healthy →
    /// retrying, retrying → blocked, any failure → healthy): an owner logs
    /// exactly there, so a repeated failure is logged once.
    pub fn transitioned(&self) -> bool {
        self.transitioned
    }

    /// Unknown reads judged since the failure began (0 while healthy).
    pub fn attempts(&self) -> u32 {
        match &self.health {
            Health::Healthy => 0,
            Health::Retrying(retrying) => retrying.attempts,
            Health::Blocked(blocked) => blocked.attempts,
        }
    }

    /// A failure began at `now` (a change failed) before anything was read
    /// back: the horizon runs from here, and the first read waits like the
    /// first retry. Already failed: nothing changes.
    pub fn begin(&mut self, check: &'static str, reason: impl std::fmt::Display, now: Instant) {
        self.transitioned = self.health.is_healthy();
        if self.transitioned {
            self.health = Health::Retrying(Retrying {
                check,
                reason: reason.to_string(),
                since: now,
                attempts: 0,
                retry_at: now,
            });
        }
    }

    /// Judge one read-back or change at `now`.
    pub fn observe(&mut self, observation: Observation, now: Instant) -> Step {
        match observation {
            Observation::Holds => {
                self.transitioned = !self.health.is_healthy();
                self.health = Health::Healthy;
                Step::Hold
            }
            Observation::Unknown { check, reason } => match &mut self.health {
                // Nothing read proves the evidence gone: still blocked.
                Health::Blocked(blocked) => {
                    self.transitioned = false;
                    blocked.attempts = blocked.attempts.saturating_add(1);
                    blocked.reread_at = self.policy.reread_blocked.map(|every| now + every);
                    Step::Block(blocked.reread_at)
                }
                health => {
                    let (since, attempts) = match health {
                        Health::Retrying(retrying) => {
                            (retrying.since, retrying.attempts.saturating_add(1))
                        }
                        _ => (now, 1),
                    };
                    self.transitioned = health.is_healthy();
                    let elapsed = now.saturating_duration_since(since);
                    let wait = self.policy.backoff.delay(attempts);
                    let (step, wait) = match self.policy.horizon {
                        Horizon::Escalate(limit) if elapsed >= limit => {
                            (Some(Step::Escalate(check)), Duration::ZERO)
                        }
                        Horizon::Escalate(limit) => (None, wait.min(limit - elapsed)),
                        Horizon::Unbounded => (None, wait),
                    };
                    let retry_at = now + wait;
                    *health = Health::Retrying(Retrying {
                        check,
                        reason,
                        since,
                        attempts,
                        retry_at,
                    });
                    step.unwrap_or(Step::RetryAt(retry_at))
                }
            },
            Observation::Proven { check, reason } => {
                let reread_at = self.policy.reread_blocked.map(|every| now + every);
                match &mut self.health {
                    Health::Blocked(blocked) => {
                        self.transitioned = false;
                        blocked.check = check;
                        blocked.reason = reason;
                        blocked.attempts = blocked.attempts.saturating_add(1);
                        blocked.reread_at = reread_at;
                    }
                    health => {
                        self.transitioned = true;
                        *health = Health::Blocked(Blocked {
                            check,
                            reason,
                            since: now,
                            attempts: 1,
                            reread_at,
                        });
                    }
                }
                Step::Block(reread_at)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    fn policy(horizon: Horizon) -> Policy {
        Policy {
            backoff: Backoff::new(SECOND, 60 * SECOND),
            horizon,
            reread_blocked: Some(60 * SECOND),
        }
    }

    fn unknown() -> Observation {
        Observation::unknown("verify_unavailable", "kernel netlink operation failed")
    }

    /// Unknown within the horizon keeps retrying with the backoff, capped
    /// by what is left; the horizon escalates exactly when it passes, never
    /// blocks; a holding read ends it.
    #[test]
    fn an_unknown_retries_within_its_horizon_and_escalates_only_after_it() {
        let start = Instant::now();
        let mut recovery = Recovery::new(policy(Horizon::Escalate(30 * SECOND)));
        assert_eq!(
            recovery.observe(unknown(), start),
            Step::RetryAt(start + SECOND)
        );
        assert!(recovery.transitioned(), "healthy → retrying");
        let waits: Vec<_> = (1..5)
            .map(|second| {
                let at = start + second * SECOND;
                match recovery.observe(unknown(), at) {
                    Step::RetryAt(next) => (next - at).as_secs(),
                    step => panic!("{step:?}"),
                }
            })
            .collect();
        assert_eq!(waits, [2, 4, 8, 16]);
        assert!(!recovery.transitioned(), "logged once");
        let late = start + 25 * SECOND;
        assert_eq!(
            recovery.observe(unknown(), late),
            Step::RetryAt(start + 30 * SECOND),
            "capped by the horizon"
        );
        let just_before = start + 30 * SECOND - Duration::from_millis(1);
        assert!(matches!(
            recovery.observe(unknown(), just_before),
            Step::RetryAt(_)
        ));
        assert_eq!(
            recovery.observe(
                Observation::unknown("change_unavailable", "outcome unknown"),
                start + 30 * SECOND
            ),
            Step::Escalate("change_unavailable")
        );
        assert!(matches!(recovery.health(), Health::Retrying(r) if r.since == start));
        assert_eq!(
            recovery.observe(Observation::Holds, start + 31 * SECOND),
            Step::Hold
        );
        assert!(recovery.transitioned() && recovery.health().is_healthy());
        assert!(!recovery.health().is_blocked());
    }

    /// A proven read blocks at once and re-reads only at the slow interval;
    /// a failed read while blocked proves nothing and keeps it blocked; the
    /// evidence gone, it holds again.
    #[test]
    fn a_proven_read_blocks_and_only_a_holding_read_ends_it() {
        let start = Instant::now();
        let mut recovery = Recovery::new(policy(Horizon::Unbounded));
        recovery.observe(unknown(), start);
        let proven = Observation::proven("verify", "objects_changed");
        assert_eq!(
            recovery.observe(proven.clone(), start + SECOND),
            Step::Block(Some(start + 61 * SECOND))
        );
        assert!(recovery.transitioned(), "retrying → blocked");
        assert!(matches!(recovery.health(), Health::Blocked(b) if b.since == start + SECOND));
        assert_eq!(
            recovery.observe(unknown(), start + 61 * SECOND),
            Step::Block(Some(start + 121 * SECOND)),
            "an unknown read while blocked"
        );
        assert!(recovery.health().is_blocked() && !recovery.transitioned());
        recovery.observe(proven, start + 121 * SECOND);
        assert!(!recovery.transitioned());
        assert_eq!(recovery.attempts(), 3);
        let pending = recovery.health().pending().expect("re-read by itself");
        assert_eq!(pending.reason, "objects_changed");
        assert_eq!(
            recovery.observe(Observation::Holds, start + 181 * SECOND),
            Step::Hold
        );
        assert!(recovery.transitioned());
        // Without a slow re-read, blocked has no pending projection.
        let mut once = Recovery::new(Policy {
            reread_blocked: None,
            ..policy(Horizon::Unbounded)
        });
        assert_eq!(
            once.observe(Observation::proven("verify", "x"), start),
            Step::Block(None)
        );
        assert!(once.health().pending().is_none());
    }

    /// An unbounded owner retries forever at the capped interval; `begin`
    /// starts the horizon at a failed change before the first read.
    #[test]
    fn an_unbounded_owner_caps_its_interval_and_begin_starts_the_horizon() {
        let start = Instant::now();
        let mut recovery = Recovery::new(policy(Horizon::Unbounded));
        let mut at = start;
        for _ in 0..12 {
            match recovery.observe(unknown(), at) {
                Step::RetryAt(next) => at = next,
                step => panic!("{step:?}"),
            }
        }
        assert_eq!(recovery.attempts(), 12);
        let pending = recovery.health().pending().unwrap();
        assert_eq!(pending.attempts, 12);
        let mut change = Recovery::new(policy(Horizon::Escalate(30 * SECOND)));
        change.begin("change_unavailable", "netlink", start);
        assert!(change.transitioned() && change.attempts() == 0);
        change.begin("change_unavailable", "again", start + SECOND);
        assert!(!change.transitioned());
        assert_eq!(
            change.observe(unknown(), start + 30 * SECOND),
            Step::Escalate("verify_unavailable")
        );
    }

    #[test]
    fn wall_time_follows_the_monotonic_offset() {
        let now = Instant::now();
        let ahead = wall(now + 10 * SECOND)
            .duration_since(SystemTime::now())
            .unwrap();
        assert!(ahead > 9 * SECOND && ahead <= 10 * SECOND);
    }
}
