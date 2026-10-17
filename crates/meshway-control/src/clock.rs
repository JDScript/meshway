//! A process-local UTC high-water mark advanced by monotonic time, for the
//! Server deadlines Control owns (session, instance, observation and
//! coordination). Rolling the wall clock back cannot revive an expired
//! deadline or lengthen a later refresh. The netmap lease is Engine's.
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Error {
    #[error("the deadline has expired")]
    Expired,
    #[error("the Server deadline is not representable")]
    Contract,
}

/// Wall-clock milliseconds. Combine with a `Clock` so that a backwards
/// adjustment cannot revive an expired deadline.
pub(crate) fn now() -> Result<u64, Error> {
    chrono::Utc::now()
        .timestamp_millis()
        .try_into()
        .map_err(|_| Error::Expired)
}

pub struct Clock {
    born: Instant,
    offset: AtomicU64,
    changed: tokio::sync::Notify,
}
impl Clock {
    pub fn new() -> Result<Self, Error> {
        let wall = now()?;
        Ok(Self {
            born: Instant::now(),
            offset: AtomicU64::new(wall),
            changed: tokio::sync::Notify::new(),
        })
    }
    /// Never report a UTC time before `floor_ms` (a time observed earlier,
    /// possibly before a restart).
    pub(crate) fn raise_floor(&self, floor_ms: u64) {
        let _ = self.observe(floor_ms, Instant::now());
    }

    /// `monotonic` must be read after `wall`: the offset then never exceeds
    /// the true one, so a delay between the two reads (preemption, CPU quota)
    /// cannot leave this clock permanently ahead of the wall clock.
    fn observe(&self, wall: u64, monotonic: Instant) -> Result<u64, Error> {
        let elapsed: u64 = monotonic
            .duration_since(self.born)
            .as_millis()
            .try_into()
            .map_err(|_| Error::Expired)?;
        let candidate = wall.saturating_sub(elapsed);
        let previous = self.offset.fetch_max(candidate, Ordering::AcqRel);
        let offset = previous.max(candidate);
        if offset > previous {
            self.changed.notify_waiters();
        }
        offset.checked_add(elapsed).ok_or(Error::Expired)
    }
    pub fn now(&self) -> Result<u64, Error> {
        self.now_with_instant().map(|(wall, _)| wall)
    }

    /// One wall-clock and one monotonic read, so a caller checking several
    /// deadlines evaluates them all at the same instant.
    pub fn now_with_instant(&self) -> Result<(u64, Instant), Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::detail("control.clock", 2);
        let _profile = PROFILE.enter();
        let wall = now()?;
        let monotonic = Instant::now();
        Ok((self.observe(wall, monotonic)?, monotonic))
    }

    pub async fn wait_until(
        &self,
        monotonic: Instant,
        wall: u64,
        cancelled: &tokio_util::sync::CancellationToken,
    ) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let Ok(now_ms) = self.now() else {
                return;
            };
            let Some(remaining) = wall.checked_sub(now_ms).filter(|remaining| *remaining > 0)
            else {
                return;
            };
            let deadline = Instant::now()
                .checked_add(Duration::from_millis(remaining))
                .map_or(monotonic, |deadline| deadline.min(monotonic));
            tokio::select! { biased;
                _ = cancelled.cancelled() => return,
                _ = tokio::time::sleep_until(deadline.into()) => return,
                _ = &mut changed => {},
            }
        }
    }
}

/// The moment a request left, so a response deadline excludes transit time.
#[derive(Clone)]
pub struct Started {
    wall: u64,
    monotonic: Instant,
    clock: Arc<Clock>,
}
impl Started {
    pub fn now(clock: &Arc<Clock>) -> Result<Self, Error> {
        let wall = now()?;
        let monotonic = Instant::now();
        Ok(Self {
            wall: clock.observe(wall, monotonic)?,
            monotonic,
            clock: Arc::clone(clock),
        })
    }
}
#[derive(Clone)]
pub struct Deadline {
    wall: u64,
    monotonic: Instant,
    clock: Arc<Clock>,
}
impl Deadline {
    pub async fn ended(&self, cancelled: &tokio_util::sync::CancellationToken) {
        self.clock
            .wait_until(self.monotonic, self.wall, cancelled)
            .await;
    }
    pub fn from_response(started: Started, expires: u64) -> Result<Self, Error> {
        let remaining = expires.checked_sub(started.wall).ok_or(Error::Expired)?;
        let monotonic = started
            .monotonic
            .checked_add(Duration::from_millis(remaining))
            .ok_or(Error::Contract)?;
        let deadline = Self {
            wall: expires,
            monotonic,
            clock: started.clock,
        };
        if !deadline.valid() {
            return Err(Error::Expired);
        }
        Ok(deadline)
    }
    /// The issuer clock's current UTC high-water reading.
    pub fn now_ms(&self) -> Result<u64, Error> {
        self.clock.now()
    }
    pub fn valid(&self) -> bool {
        self.clock
            .now_with_instant()
            .is_ok_and(|(now, instant)| self.valid_at(now, instant))
    }

    /// Evaluate against readings taken from this deadline's own clock.
    pub fn valid_at(&self, now_ms: u64, instant: Instant) -> bool {
        now_ms < self.wall && instant < self.monotonic
    }

    pub fn expires_at(&self) -> Instant {
        self.monotonic
    }

    pub fn expires_at_ms(&self) -> u64 {
        self.wall
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn partial_forward_clock_adjustment_shortens_an_already_waiting_deadline() {
        use std::{
            future::{Future, poll_fn},
            task::Poll,
        };
        let clock = Clock::new().unwrap();
        let wall = clock.now().unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let wait = clock.wait_until(
            Instant::now() + Duration::from_secs(10),
            wall + 10_000,
            &cancel,
        );
        tokio::pin!(wait);
        poll_fn(|cx| {
            assert!(wait.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        clock.observe(wall + 9_900, Instant::now()).unwrap();
        // No more reads/clock observations after this partial adjustment. The
        // existing waiter must wake for its remaining 100ms, not the old 10s.
        tokio::time::timeout(Duration::from_secs(2), wait)
            .await
            .unwrap();
        assert!(clock.now().unwrap() >= wall + 10_000);
    }
    #[tokio::test]
    async fn rollback_does_not_revive_a_grant_or_extend_a_refresh() {
        let clock = Arc::new(Clock::new().unwrap());
        let start = Started::now(&clock).unwrap();
        let wall = start.wall;
        let deadline = Deadline::from_response(start, wall + 60_000).unwrap();
        assert!(deadline.valid());
        let notification = clock.changed.notified();
        tokio::pin!(notification);
        notification.as_mut().enable();
        clock.observe(wall + 120_000, Instant::now()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), notification)
            .await
            .unwrap();
        assert!(!deadline.valid());
        clock.observe(wall, Instant::now()).unwrap();
        assert!(!deadline.valid());
        assert!(Deadline::from_response(Started::now(&clock).unwrap(), wall + 60_000).is_err());
    }
    #[test]
    fn a_delay_between_the_two_reads_never_puts_the_clock_ahead() {
        let clock = Clock::new().unwrap();
        // Preempted after the wall read: the monotonic read comes 50 ms later.
        let wall = now().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        clock.observe(wall, Instant::now()).unwrap();
        let reading = clock.now().unwrap();
        assert!(reading <= now().unwrap(), "clock ahead of the wall clock");
    }
    #[test]
    fn response_transit_time_is_not_added_to_its_deadline() {
        let clock = Arc::new(Clock::new().unwrap());
        let started = Started {
            wall: now().unwrap(),
            monotonic: Instant::now() - Duration::from_secs(2),
            clock,
        };
        let expires = started.wall + 1000;
        assert!(Deadline::from_response(started, expires).is_err());
    }
}
