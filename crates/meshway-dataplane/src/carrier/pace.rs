//! Send pacing of one datagram channel. Not congestion control: the inner
//! transport keeps that. A channel may send at most twice the highest rate
//! it sent at in the last 100 ms (5 ms epochs), with a burst allowance of
//! 2 ms at that rate (at least 64 KiB). A steady flow is never delayed, nor
//! a window-limited one whose bursts repeat within a round trip of up to
//! 100 ms; a burst far above that rate (a TCP window arriving at once after
//! idle, from a fast host) leaves spread over the following epochs instead
//! of at host speed, while the allowed rate still doubles every epoch it is
//! used, so a growing flow is not capped. After 100 ms idle the allowance is
//! the floor.
//!
//! A wait sleeps on the runtime timer (millisecond resolution); the burst
//! allowance covers at least 1 ms at the paced rate, so a wait never idles
//! a channel that may send.
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

const EPOCH: Duration = Duration::from_millis(5);
const EPOCHS: usize = 20;
const GAIN: u64 = 2;
/// Bytes/s a channel may always use: 4 MiB/s (about 33 Mbit/s) before gain.
const FLOOR: u64 = 4 << 20;
const MIN_BURST: u64 = 64 << 10;
const BURST: Duration = Duration::from_millis(2);

pub struct Pacer(Mutex<State>);

struct State {
    epoch: Instant,
    bytes: u64,
    /// Bytes/s of the last completed epochs.
    samples: [u64; EPOCHS],
    next: usize,
    tokens: u64,
    refilled: Instant,
}

fn per(rate: u64, duration: Duration) -> u64 {
    u64::try_from(u128::from(rate) * duration.as_nanos() / 1_000_000_000).unwrap_or(u64::MAX)
}

impl State {
    fn roll(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.epoch);
        if elapsed < EPOCH {
            return;
        }
        let epochs = elapsed.as_nanos() / EPOCH.as_nanos();
        if epochs > EPOCHS as u128 {
            // The epoch with bytes ended before the window: all were idle.
            self.samples = [0; EPOCHS];
        } else {
            let sample = u64::try_from(u128::from(self.bytes) * 1_000_000_000 / EPOCH.as_nanos())
                .unwrap_or(u64::MAX);
            for index in 0..epochs {
                self.samples[self.next] = if index == 0 { sample } else { 0 };
                self.next = (self.next + 1) % EPOCHS;
            }
        }
        self.bytes = 0;
        self.epoch = now - Duration::from_nanos((elapsed.as_nanos() % EPOCH.as_nanos()) as u64);
    }

    fn rate(&self) -> u64 {
        GAIN.saturating_mul(self.samples.iter().copied().max().unwrap_or(0).max(FLOOR))
    }

    fn depth(&self) -> u64 {
        per(self.rate(), BURST).max(MIN_BURST)
    }

    fn refill(&mut self, now: Instant) {
        let rate = self.rate();
        let gained = per(rate, now.saturating_duration_since(self.refilled));
        self.refilled = now;
        self.roll(now);
        self.tokens = self.tokens.saturating_add(gained).min(self.depth());
    }
}

impl Default for Pacer {
    fn default() -> Self {
        let now = Instant::now();
        Self(Mutex::new(State {
            epoch: now,
            bytes: 0,
            samples: [0; EPOCHS],
            next: 0,
            tokens: MIN_BURST,
            refilled: now,
        }))
    }
}

impl Pacer {
    /// How many of the next datagrams (`sizes`, in order) may leave now; or,
    /// when not even the first may, the instant it may. A datagram larger
    /// than the burst allowance leaves when the allowance is full.
    pub fn admit(
        &self,
        sizes: impl IntoIterator<Item = usize>,
        now: Instant,
    ) -> Result<usize, Instant> {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.refill(now);
        let depth = state.depth();
        let mut budget = state.tokens;
        let mut first = None;
        let mut count = 0;
        for size in sizes {
            let size = size as u64;
            first.get_or_insert(size);
            if size > budget && !(count == 0 && state.tokens >= depth) {
                break;
            }
            budget = budget.saturating_sub(size);
            count += 1;
        }
        if count != 0 || first.is_none() {
            return Ok(count);
        }
        let needed = first.unwrap_or(0).min(depth);
        let missing = needed.saturating_sub(state.tokens).max(1);
        let rate = state.rate().max(1);
        Err(now
            + Duration::from_nanos(
                u64::try_from(u128::from(missing) * 1_000_000_000 / u128::from(rate))
                    .unwrap_or(u64::MAX),
            ))
    }

    /// Bytes the transport accepted from this channel.
    pub fn sent(&self, bytes: usize, now: Instant) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.refill(now);
        state.tokens = state.tokens.saturating_sub(bytes as u64);
        state.bytes = state.bytes.saturating_add(bytes as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(
        pacer: &Pacer,
        start: Instant,
        demand: u64,
        step: Duration,
        until: Duration,
    ) -> Vec<u64> {
        // Bytes sent per epoch by a sender that always has `demand` bytes/step.
        let mut per_epoch = vec![0; (until.as_nanos() / EPOCH.as_nanos()) as usize];
        let mut queued = 0u64;
        let mut at = start;
        while at < start + until {
            queued += demand;
            while queued >= 1400 {
                match pacer.admit([1400], at) {
                    Ok(_) => {
                        pacer.sent(1400, at);
                        queued -= 1400;
                        per_epoch
                            [(at.duration_since(start).as_nanos() / EPOCH.as_nanos()) as usize] +=
                            1400;
                    }
                    Err(_) => break,
                }
            }
            at += step;
        }
        per_epoch
    }

    #[test]
    fn steady_flow_is_never_delayed() {
        let pacer = Pacer::default();
        // 1400 B every 10 µs: 140 MB/s, smooth.
        let sent = drain(
            &pacer,
            Instant::now(),
            1400,
            Duration::from_micros(10),
            Duration::from_millis(200),
        );
        // After the ramp, every epoch carries the whole demand (700 KB).
        assert!(sent[10..].iter().all(|bytes| *bytes >= 690_000), "{sent:?}");
    }

    #[test]
    fn burst_after_idle_is_spread_and_rate_doubles_per_epoch() {
        let pacer = Pacer::default();
        let start = Instant::now();
        // 1 MB arrives at once after idle.
        let mut first = 0;
        while pacer.admit([1400], start).is_ok() {
            pacer.sent(1400, start);
            first += 1400;
        }
        assert!(first <= MIN_BURST + 1400);
        let wait = pacer.admit([1400], start).unwrap_err();
        assert!(wait > start && wait - start <= Duration::from_millis(10));
        // Unbounded demand: each epoch may send about twice the previous one.
        let pacer = Pacer::default();
        let sent = drain(
            &pacer,
            start,
            1 << 20,
            Duration::from_micros(250),
            Duration::from_millis(40),
        );
        for pair in sent.windows(2).take(5) {
            assert!(
                pair[1] >= pair[0] && pair[1] <= pair[0] * 2 + 2 * MIN_BURST,
                "{sent:?}"
            );
        }
        assert!(sent[6] > 8 * sent[0], "{sent:?}");
    }

    #[test]
    fn window_limited_bursts_repeating_within_a_round_trip_are_not_delayed_again() {
        let pacer = Pacer::default();
        let start = Instant::now();
        let mut drained = Vec::new();
        for round in 0..6u32 {
            // 1 MB arrives at once every 60 ms (a window per round trip).
            let at = start + Duration::from_millis(60) * round;
            let mut queued = 1_000_000u64;
            let mut now = at;
            while queued >= 1400 {
                match pacer.admit([1400], now) {
                    Ok(_) => {
                        pacer.sent(1400, now);
                        queued -= 1400;
                    }
                    Err(_) => now += Duration::from_micros(100),
                }
            }
            drained.push(now - at);
        }
        assert!(drained[0] > Duration::from_millis(10), "{drained:?}");
        assert!(
            drained[2..]
                .iter()
                .all(|time| *time <= Duration::from_millis(2)),
            "{drained:?}"
        );
    }

    #[test]
    fn idle_returns_to_the_floor_and_oversized_waits_for_a_full_allowance() {
        let pacer = Pacer::default();
        let start = Instant::now();
        drain(
            &pacer,
            start,
            1 << 20,
            Duration::from_micros(250),
            Duration::from_millis(40),
        );
        let later = start + Duration::from_millis(200);
        assert!(pacer.admit([1400], later).is_ok());
        {
            let mut state = pacer.0.lock().unwrap();
            state.refill(later);
            assert_eq!(state.rate(), GAIN * FLOOR);
        }
        let pacer = Pacer::default();
        assert_eq!(pacer.admit([200_000], start), Ok(1));
        pacer.sent(200_000, start);
        assert!(pacer.admit([200_000], start).is_err());
    }
}
