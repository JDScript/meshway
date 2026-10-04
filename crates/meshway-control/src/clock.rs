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
}
impl Clock {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            born: Instant::now(),
            offset: AtomicU64::new(now()?),
        })
    }
    fn observe(&self, wall: u64, monotonic: Instant) -> Result<u64, Error> {
        let elapsed: u64 = monotonic
            .duration_since(self.born)
            .as_millis()
            .try_into()
            .map_err(|_| Error::Expired)?;
        let candidate = wall.saturating_sub(elapsed);
        let offset = self
            .offset
            .fetch_max(candidate, Ordering::AcqRel)
            .max(candidate);
        offset.checked_add(elapsed).ok_or(Error::Expired)
    }
    fn now(&self) -> Result<u64, Error> {
        self.observe(now()?, Instant::now())
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
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rollback_does_not_revive_a_grant_or_extend_a_refresh() {
        let clock = Arc::new(Clock::new().unwrap());
        let start = Started::now(&clock).unwrap();
        let wall = start.wall;
        let deadline = Deadline::from_response(start, wall + 60_000).unwrap();
        assert!(deadline.valid());
        clock.observe(wall + 120_000, Instant::now()).unwrap();
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
