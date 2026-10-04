//! A process-local UTC high-water mark advanced by monotonic time. Rolling the
//! wall clock back cannot revive an expired grant or lengthen a later refresh.
use crate::{Error, now};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

pub(super) struct Clock {
    born: Instant,
    offset: AtomicU64,
    changed: tokio::sync::Notify,
}
impl Clock {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            born: Instant::now(),
            offset: AtomicU64::new(now()?),
            changed: tokio::sync::Notify::new(),
        })
    }
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
    pub(super) fn now(&self) -> Result<u64, Error> {
        self.observe(now()?, Instant::now())
    }

    pub(super) async fn wait_until(
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

#[derive(Clone)]
pub(super) struct Started {
    wall: u64,
    monotonic: Instant,
    clock: Arc<Clock>,
}
impl Started {
    pub fn now(clock: &Arc<Clock>) -> Result<Self, Error> {
        let monotonic = Instant::now();
        Ok(Self {
            wall: clock.observe(now()?, monotonic)?,
            monotonic,
            clock: Arc::clone(clock),
        })
    }
}
#[derive(Clone)]
pub(super) struct Deadline {
    wall: u64,
    monotonic: Instant,
    clock: Arc<Clock>,
}
impl Deadline {
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
    pub fn valid(&self) -> bool {
        self.clock.now().is_ok_and(|now| now < self.wall) && Instant::now() < self.monotonic
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
