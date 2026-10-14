use super::*;

// Kept across content and resource changes: churn cannot reset admission.
// Acquisition is bounded to 30 starts in any half-open minute, paced at 250 ms.
const QUERY_INTERVAL: Duration = Duration::from_millis(250);
/// Every traversal socket observes its mapping on its own cadence, whether or
/// not any peer is interested: ~30 s idle, ~10 s while it carries a Native
/// UDP channel or an attempt, each with a fixed per-socket jitter of ±10 %.
const STUN_IDLE: Duration = Duration::from_secs(30);
const STUN_ACTIVE: Duration = Duration::from_secs(10);
/// A round without a validated response retries after 2 s, doubling up to
/// the idle cadence; a metadata query that yields no STUN context backs off
/// the same way. Neither consumes the acquisition budget.
const STUN_RETRY: Duration = Duration::from_secs(2);

pub(super) struct QuerySchedule {
    next: Instant,
    observation_at: Instant,
    observation_last: Instant,
    observation_backoff: Duration,
    acquisitions: std::collections::VecDeque<Instant>,
}
impl QuerySchedule {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            next: now,
            observation_at: now,
            observation_last: now,
            observation_backoff: STUN_RETRY,
            acquisitions: std::collections::VecDeque::with_capacity(30),
        }
    }

    pub(super) fn admit(&mut self, now: Instant, acquisition: bool, observation: bool) -> bool {
        if now < self.next {
            return false;
        }
        while self
            .acquisitions
            .front()
            .is_some_and(|at| now.duration_since(*at) >= Duration::from_secs(60))
        {
            self.acquisitions.pop_front();
        }
        if observation && now >= self.observation_at {
            self.observation_last = now;
            self.observation_at = now + self.observation_backoff;
        } else if acquisition && self.acquisitions.len() < 30 {
            self.acquisitions.push_back(now);
        } else {
            return false;
        }
        self.next = now + QUERY_INTERVAL;
        true
    }

    /// Whether the last admitted observation query produced a usable STUN
    /// context; its outcome paces the next one.
    pub(super) fn observed(&mut self, usable: bool) {
        self.observation_backoff = if usable {
            STUN_RETRY
        } else {
            (self.observation_backoff * 2).min(STUN_IDLE)
        };
        self.observation_at = self.observation_last + self.observation_backoff;
    }
}

/// One socket's observation rounds: when the last one was sent and how many
/// in a row ended without a validated response.
#[derive(Clone, Copy)]
pub(super) struct Round {
    pub(super) sent: Instant,
    pub(super) failures: u32,
}

/// The socket's fixed cadence: idle or active base with its own jitter.
pub(super) fn cadence(id: &[u8; 32], active: bool) -> Duration {
    let base = if active { STUN_ACTIVE } else { STUN_IDLE };
    base.mul_f64(0.9 + 0.2 * f64::from(id[0]) / 255.0)
}

/// Due when the cadence since the last validated response elapsed (or none
/// arrived yet), and not before the retry delay of an unanswered round.
pub(super) fn observation_due(
    confirmed: Option<Instant>,
    round: Option<Round>,
    cadence: Duration,
    now: Instant,
) -> bool {
    if let Some(round) = round
        && confirmed.is_none_or(|at| at < round.sent)
        && now < round.sent + retry(round.failures)
    {
        return false;
    }
    confirmed.is_none_or(|at| now >= at + cadence)
}

fn retry(failures: u32) -> Duration {
    STUN_RETRY
        .saturating_mul(1 << failures.min(4))
        .min(STUN_IDLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unneeded_queries_are_suppressed_and_churn_cannot_reset_the_budget() {
        let start = Instant::now();
        let mut schedule = QuerySchedule::new(start);
        let mut queries = 0;
        for tick in 0..2400 {
            let now = start + Duration::from_millis(tick * 250);
            assert!(
                !schedule.admit(now, false, false),
                "neither acquisition nor observation due"
            );
            // Content/resource changes share this schedule, including a retry
            // immediately after a previous request failed or was cancelled.
            queries += usize::from(schedule.admit(now, true, false));
            assert!(!schedule.admit(now, true, false));
        }
        assert_eq!(queries, 300, "at most 30 requests/minute for ten minutes");
    }

    #[test]
    fn observation_queries_survive_saturated_acquisition_and_back_off_without_stun() {
        let start = Instant::now();
        let mut schedule = QuerySchedule::new(start);
        for tick in 0..30 {
            assert!(schedule.admit(start + Duration::from_millis(tick * 250), true, false));
        }
        let due = start + Duration::from_secs(8);
        assert!(!schedule.admit(due, true, false));
        assert!(schedule.admit(due, false, true));
        assert!(!schedule.admit(due + Duration::from_secs(1), true, true));
        // A Server without STUN: 4, 8, 16, then the idle cadence.
        let mut at = due;
        for wait in [4, 8, 16, 30, 30] {
            schedule.observed(false);
            assert!(!schedule.admit(at + Duration::from_secs(wait) - QUERY_INTERVAL, false, true));
            at += Duration::from_secs(wait);
            assert!(schedule.admit(at, false, true));
        }
        schedule.observed(true);
        assert!(schedule.admit(at + STUN_RETRY, false, true));
        assert!(schedule.admit(start + Duration::from_secs(600), true, false));
    }

    #[test]
    fn every_socket_observes_on_its_own_cadence_without_any_interest() {
        let start = Instant::now();
        let id = [128; 32];
        let idle = cadence(&id, false);
        let active = cadence(&id, true);
        assert!(idle > Duration::from_secs(27) && idle < Duration::from_secs(33));
        assert!(active > Duration::from_secs(9) && active < Duration::from_secs(11));
        assert_ne!(
            cadence(&[0; 32], false),
            cadence(&[255; 32], false),
            "jittered"
        );
        // A new socket is due at once; a confirmation defers it by its cadence.
        assert!(observation_due(None, None, idle, start));
        let sent = Round {
            sent: start,
            failures: 0,
        };
        let confirmed = start + Duration::from_millis(40);
        assert!(!observation_due(
            Some(confirmed),
            Some(sent),
            idle,
            confirmed + idle / 2
        ));
        assert!(observation_due(
            Some(confirmed),
            Some(sent),
            idle,
            confirmed + idle
        ));
        assert!(!observation_due(
            Some(confirmed),
            Some(sent),
            active,
            confirmed + active / 2
        ));
        assert!(observation_due(
            Some(confirmed),
            Some(sent),
            active,
            confirmed + active
        ));
        // An unanswered round retries after 2 s, doubling up to the idle cadence.
        for (failures, wait) in [(0, 2), (1, 4), (2, 8), (3, 16), (4, 30), (9, 30)] {
            let round = Round {
                sent: start,
                failures,
            };
            let wait = Duration::from_secs(wait);
            assert!(!observation_due(
                None,
                Some(round),
                idle,
                start + wait - QUERY_INTERVAL
            ));
            assert!(observation_due(None, Some(round), idle, start + wait));
            // An older confirmation does not answer a later round.
            assert!(!observation_due(
                Some(start - Duration::from_secs(60)),
                Some(round),
                idle,
                start + wait - QUERY_INTERVAL
            ));
        }
    }
}
