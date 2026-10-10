//! Lease scheduling belongs only to Engine. This module exports no deadline,
//! guard, transferable permission, cancellation ancestry or renewal handle.
use super::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::Instant;

/// Control's verified and durably saved input. Signature verification remains
/// at that boundary; Engine does not repeat it or send the lease to components.
#[derive(Clone)]
pub struct Netmap {
    pub content: Arc<CompiledNetmap>,
    pub lease: NetmapLease,
}

pub(super) struct Clock {
    at: Instant,
    wall: u64,
}
impl Clock {
    pub fn new() -> Self {
        Self {
            at: Instant::now(),
            wall: wall(),
        }
    }
    pub fn now(&mut self) -> u64 {
        let now = Instant::now();
        self.wall = self
            .wall
            .saturating_add(
                now.duration_since(self.at)
                    .as_millis()
                    .try_into()
                    .unwrap_or(u64::MAX),
            )
            .max(wall());
        self.at = now;
        self.wall
    }
}
fn wall() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| elapsed.as_millis().try_into().ok())
        .unwrap_or(u64::MAX)
}

pub(super) struct Active {
    pub at: Instant,
    expires_at_ms: u64,
}
impl Active {
    pub fn accept(
        next: &Netmap,
        latest: Option<NetmapLease>,
        clock: &mut Clock,
    ) -> Result<Self, Error> {
        next.lease.validate().map_err(|_| Error::Lease)?;
        if next.lease.netmap != next.content.reference() {
            return Err(Error::Reference);
        }
        if latest.is_some_and(|old| regresses(&next.lease, &old)) {
            return Err(Error::Reference);
        }
        let remaining = next
            .lease
            .expires_at_ms
            .checked_sub(clock.now())
            .filter(|left| *left > 0)
            .ok_or(Error::Lease)?;
        let at = Instant::now()
            .checked_add(Duration::from_millis(remaining))
            .ok_or(Error::Lease)?;
        Ok(Self {
            at,
            expires_at_ms: next.lease.expires_at_ms,
        })
    }
    pub fn expired(&self, clock: &mut Clock) -> bool {
        Instant::now() >= self.at || clock.now() >= self.expires_at_ms
    }
}

/// An older serial, or the same serial with other content or an earlier end.
pub(crate) fn regresses(next: &NetmapLease, old: &NetmapLease) -> bool {
    next.netmap.serial < old.netmap.serial
        || (next.netmap.serial == old.netmap.serial
            && (next.netmap != old.netmap || next.expires_at_ms < old.expires_at_ms))
}
