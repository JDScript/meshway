use std::time::Duration;

/// Bounded exponential backoff: `base` after the first failure, doubling with
/// each further consecutive failure, never longer than `cap`.
///
/// It holds no state: each owner keeps its own failure count and when it
/// resets, and classifies its own errors (a cause that cannot clear by
/// itself may wait `cap` at once, a total deadline may shorten a pause).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    base: Duration,
    cap: Duration,
}

impl Backoff {
    pub const fn new(base: Duration, cap: Duration) -> Self {
        Self { base, cap }
    }

    /// The pause after `failures` consecutive failures: `base · 2^(failures − 1)`,
    /// at most `cap`. No failure yet pauses like the first.
    pub fn delay(self, failures: u32) -> Duration {
        let factor = 2u32
            .checked_pow(failures.saturating_sub(1))
            .unwrap_or(u32::MAX);
        self.base.saturating_mul(factor).min(self.cap)
    }

    /// The pause following one of `previous`: doubled, at most `cap`.
    pub fn next(self, previous: Duration) -> Duration {
        previous.saturating_mul(2).min(self.cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_from_base_up_to_cap() {
        let backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
        let waits: Vec<_> = (0..=9).map(|n| backoff.delay(n).as_secs()).collect();
        assert_eq!(waits, [1, 1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(backoff.delay(u32::MAX), Duration::from_secs(60));
        assert_eq!(backoff.next(Duration::from_secs(1)), Duration::from_secs(2));
        assert_eq!(
            backoff.next(Duration::from_secs(40)),
            Duration::from_secs(60)
        );
        assert_eq!(backoff.next(Duration::MAX), Duration::from_secs(60));
        let small = Backoff::new(Duration::from_millis(100), Duration::from_secs(60));
        assert_eq!(small.delay(4), Duration::from_millis(800));
    }
}
