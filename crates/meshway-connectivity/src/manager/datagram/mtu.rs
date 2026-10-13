//! Packetization-layer path MTU search of one Native UDP channel (RFC 8899
//! style). A middlebox that drops oversized DF datagrams without an ICMP
//! "too big" leaves the kernel's route MTU at the interface MTU; only an
//! authenticated answer to a probe of a given size proves the path carries it.
//! A probe is signed for its exact length (`Transcript::probe`) and its Pong
//! answers this owner's fresh nonce, so a forged, replayed or truncated probe
//! confirms nothing. A confirmed size is a held fact, not a permanent one:
//! every search starts by probing it again, and a path that no longer carries
//! it is searched again below it.
use tokio::time::{Duration, Instant};

/// IPv6 minimum link MTU: the smallest size searched.
const FLOOR: u16 = 1280;
/// A probe unanswered within this long counts as lost.
const PROBE_WAIT: Duration = Duration::from_secs(2);
/// Lost probes of one size before the path counts as smaller.
const TRIES: u8 = 2;
/// A finished search runs again after this long: a path may grow or shrink.
const RAISE: Duration = Duration::from_secs(600);

pub(super) struct Search {
    /// Largest size an authenticated answer confirmed, if any.
    confirmed: Option<u16>,
    /// Smallest size known not to pass (or the route MTU) plus one.
    above: u16,
    /// The confirmed size this search must prove again before anything else.
    recheck: Option<u16>,
    probe: Option<Probe>,
    done_at: Option<Instant>,
}

struct Probe {
    nonce: [u8; 32],
    size: u16,
    tries: u8,
    sent: Instant,
}

impl Search {
    pub fn new() -> Self {
        Self {
            confirmed: None,
            above: u16::MAX,
            recheck: None,
            probe: None,
            done_at: None,
        }
    }

    /// The channel's path MTU once known: the largest confirmed size, never
    /// above the route's own MTU. `None` while searching or if not even the
    /// floor is confirmed.
    pub fn path(&self, route: Option<u16>) -> Option<u16> {
        self.done_at?;
        let confirmed = self.confirmed?;
        Some(route.map_or(confirmed, |route| confirmed.min(route)))
    }

    /// What a finished search measured: `Some(None)` when it confirmed
    /// nothing, so a held value must be withdrawn; `None` while searching (the
    /// held value stands until disproved).
    pub fn measured(&self, route: Option<u16>) -> Option<Option<u16>> {
        self.done_at?;
        Some(self.path(route))
    }

    /// The next probe size due now, if any. `route` caps the search: a size
    /// above it cannot even leave this host (DF).
    pub fn due(&mut self, route: Option<u16>, now: Instant) -> Option<u16> {
        let route = route?.max(FLOOR);
        if let Some(done) = self.done_at {
            if now < done + RAISE && self.confirmed.is_none_or(|confirmed| confirmed <= route) {
                return None;
            }
            // Raise timer or a route above what was searched: search again,
            // first proving the confirmed size still passes.
            self.done_at = None;
            self.above = u16::MAX;
            self.recheck = self.confirmed.filter(|confirmed| *confirmed <= route);
        }
        self.above = self.above.min(route.saturating_add(1));
        if let Some(probe) = &mut self.probe {
            if now < probe.sent + PROBE_WAIT {
                return None;
            }
            if probe.tries < TRIES && probe.size < self.above {
                probe.tries += 1;
                probe.sent = now;
                return Some(probe.size);
            }
            // Lost every try: this size does not pass.
            let size = probe.size;
            self.probe = None;
            self.above = self.above.min(size);
            if self.recheck.take() == Some(size) {
                // The path shrank below the held size: search below it.
                self.confirmed = None;
            }
        }
        if let Some(size) = self.recheck {
            return Some(self.start(size, now));
        }
        let low = self.confirmed.unwrap_or(FLOOR - 1);
        if low.saturating_add(1) >= self.above {
            self.done_at = Some(now);
            return None;
        }
        // Try the route MTU first (the common case is a full path), then bisect.
        let size = if self.confirmed.is_none() && self.above == route.saturating_add(1) {
            route
        } else {
            low + (self.above - low) / 2
        };
        Some(self.start(size.max(FLOOR), now))
    }

    fn start(&mut self, size: u16, now: Instant) -> u16 {
        self.probe = Some(Probe {
            nonce: [0; 32],
            size,
            tries: 1,
            sent: now,
        });
        size
    }

    /// The fresh nonce the due probe carries.
    pub fn sent(&mut self, nonce: [u8; 32]) {
        if let Some(probe) = &mut self.probe {
            probe.nonce = nonce;
        }
    }

    /// The due probe could not be sent at all: end this measurement with what
    /// is confirmed; the next one starts after the raise timer.
    pub fn abandon(&mut self, now: Instant) {
        self.probe = None;
        self.recheck = None;
        self.done_at = Some(now);
    }

    /// An authenticated Pong for `nonce`: true if it answered the probe.
    pub fn answered(&mut self, nonce: &[u8; 32], now: Instant) -> bool {
        let Some(probe) = self.probe.take_if(|probe| probe.nonce == *nonce) else {
            return false;
        };
        if self.recheck == Some(probe.size) {
            self.recheck = None;
        }
        self.confirmed = Some(self.confirmed.map_or(probe.size, |old| old.max(probe.size)));
        if self
            .confirmed
            .is_some_and(|confirmed| confirmed.saturating_add(1) >= self.above)
        {
            self.done_at = Some(now);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive a search over a path that silently drops datagrams above `path`.
    fn converge(route: u16, path: u16) -> (Option<u16>, usize) {
        let mut search = Search::new();
        let mut now = Instant::now();
        let mut probes = 0;
        for round in 0..64u8 {
            if let Some(size) = search.due(Some(route), now) {
                probes += 1;
                let nonce = [round; 32];
                search.sent(nonce);
                if size <= path {
                    assert!(search.answered(&nonce, now));
                }
            }
            if search.path(Some(route)).is_some() {
                break;
            }
            now += PROBE_WAIT;
        }
        (search.path(Some(route)), probes)
    }

    #[test]
    fn a_blackholed_path_converges_to_its_real_size() {
        // The WARP path: the route says 1500, the path carries 1357.
        let (path, probes) = converge(1500, 1357);
        assert_eq!(path, Some(1357));
        assert!(probes <= 2 + 2 * 8, "{probes} probes");
        // A full path costs one probe.
        assert_eq!(converge(1500, 1500), (Some(1500), 1));
        // The route never lets a larger size count.
        assert_eq!(converge(1400, 1500), (Some(1400), 1));
        // Not even the floor passes (an old peer ignores filled Pings).
        assert_eq!(converge(1500, 1000).0, None);
    }

    #[test]
    fn a_reply_to_another_nonce_confirms_nothing() {
        let mut search = Search::new();
        let now = Instant::now();
        assert_eq!(search.due(Some(1500), now), Some(1500));
        search.sent([1; 32]);
        assert!(!search.answered(&[2; 32], now));
        assert_eq!(search.path(Some(1500)), None);
        assert!(search.answered(&[1; 32], now));
        assert_eq!(search.path(Some(1500)), Some(1500));
        // A route that later learned a smaller PMTU caps the fact at once.
        assert_eq!(search.path(Some(1357)), Some(1357));
        // After the raise timer only the held size is probed again, never
        // anything above the route; it stays held while being rechecked.
        assert_eq!(search.due(Some(1500), now + RAISE), Some(1500));
        search.sent([3; 32]);
        assert!(search.answered(&[3; 32], now + RAISE));
        assert_eq!(search.path(Some(1500)), Some(1500));
        assert_eq!(search.due(Some(1500), now + RAISE), None);
    }

    /// A path that silently shrinks after the search finished is found by the
    /// next search: the held size is lost twice and the search goes below it.
    #[test]
    fn a_path_that_shrinks_is_searched_again_below_the_held_size() {
        let (mut search, mut now, mut probes) = (Search::new(), Instant::now(), 0u8);
        let mut drive = |search: &mut Search, now: &mut Instant, path: u16| {
            for _ in 0..64 {
                if let Some(size) = search.due(Some(1500), *now) {
                    probes = probes.wrapping_add(1);
                    let nonce = [probes; 32];
                    search.sent(nonce);
                    if size <= path {
                        assert!(search.answered(&nonce, *now));
                    }
                }
                if let Some(measured) = search.measured(Some(1500)) {
                    return measured;
                }
                *now += PROBE_WAIT;
            }
            panic!("search did not finish");
        };
        assert_eq!(drive(&mut search, &mut now, 1500), Some(1500));
        now += RAISE;
        assert_eq!(drive(&mut search, &mut now, 1357), Some(1357));
        // Shrinking below the floor withdraws the value.
        now += RAISE;
        assert_eq!(drive(&mut search, &mut now, 1000), None);
    }

    /// A probe that cannot be built ends the measurement, keeping what was
    /// confirmed, and nothing is due until the raise timer.
    #[test]
    fn an_abandoned_probe_ends_only_that_measurement() {
        let mut search = Search::new();
        let now = Instant::now();
        assert_eq!(search.due(Some(9000), now), Some(9000));
        search.abandon(now);
        assert_eq!(search.measured(Some(9000)), Some(None));
        assert_eq!(search.due(Some(9000), now + PROBE_WAIT), None);
        assert!(search.due(Some(9000), now + RAISE).is_some());
    }
}
