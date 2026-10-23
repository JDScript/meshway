//! Bounded backoff of failed logins per account and source. The key is the
//! submitted email (normalized when it is one; unknown accounts are keyed
//! the same way, so the limit is no account oracle) with the direct peer
//! address (an IPv6 source by its /64). After [`FREE_FAILURES`] failures in a
//! row each further failure closes the key for twice as long as the last,
//! from [`FIRST_DELAY`] up to [`MAX_DELAY`]; an attempt while it is closed,
//! or while another attempt of the key is still running, is `Busy` without
//! computing the password hash. A successful login, or [`FORGET_AFTER`]
//! without failures, clears the key: nothing is locked for good. At most
//! [`MAX_KEYS`] keys are kept, the least recently failed dropped first.
//!
//! The source is the TCP peer, never a forwarded header: behind a reverse
//! proxy every client has the proxy's address and the limit is per account
//! (see docs/v2/install.md for proxy-side limits).

use crate::error::Error;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) const FREE_FAILURES: u32 = 5;
pub(crate) const FIRST_DELAY: Duration = Duration::from_secs(1);
pub(crate) const MAX_DELAY: Duration = Duration::from_secs(300);
pub(crate) const FORGET_AFTER: Duration = Duration::from_secs(15 * 60);
pub(crate) const MAX_KEYS: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Source {
    V4([u8; 4]),
    V6([u8; 8]),
    Unknown,
}

impl Source {
    fn of(address: Option<IpAddr>) -> Self {
        match address.map(|address| address.to_canonical()) {
            Some(IpAddr::V4(address)) => Self::V4(address.octets()),
            Some(IpAddr::V6(address)) => {
                let mut prefix = [0; 8];
                prefix.copy_from_slice(&address.octets()[..8]);
                Self::V6(prefix)
            }
            None => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    account: [u8; 32],
    source: Source,
}

impl Key {
    pub(crate) fn new(email: &str, source: Option<IpAddr>) -> Self {
        let account =
            super::email::normalize(email).unwrap_or_else(|_| email.trim().to_lowercase());
        Self {
            account: Sha256::digest(account.as_bytes()).into(),
            source: Source::of(source),
        }
    }
}

struct Entry {
    failures: u32,
    closed_until: Option<Instant>,
    last_failure: Instant,
    running: bool,
}

#[derive(Clone, Default)]
pub(crate) struct Throttle(Arc<Mutex<HashMap<Key, Entry>>>);

/// One admitted attempt of a key; [`Attempt::finish`] records its result,
/// dropping it unfinished (cancelled, or failed otherwise) records nothing.
pub(crate) struct Attempt {
    throttle: Throttle,
    key: Key,
}

impl Throttle {
    pub(crate) fn admit(&self, key: Key) -> Result<Attempt, Error> {
        self.admit_at(key, Instant::now())
    }

    fn admit_at(&self, key: Key, now: Instant) -> Result<Attempt, Error> {
        let mut entries = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = entries.get_mut(&key) {
            if now.duration_since(entry.last_failure) >= FORGET_AFTER && !entry.running {
                entries.remove(&key);
            } else if entry.running || entry.closed_until.is_some_and(|until| now < until) {
                return Err(Error::Busy);
            } else {
                entry.running = true;
                return Ok(Attempt {
                    throttle: self.clone(),
                    key,
                });
            }
        }
        if entries.len() >= MAX_KEYS {
            entries.retain(|_, entry| {
                entry.running || now.duration_since(entry.last_failure) < FORGET_AFTER
            });
        }
        if entries.len() >= MAX_KEYS
            && let Some(oldest) = entries
                .iter()
                .filter(|(_, entry)| !entry.running)
                .min_by_key(|(_, entry)| entry.last_failure)
                .map(|(key, _)| *key)
        {
            entries.remove(&oldest);
        }
        entries.insert(
            key,
            Entry {
                failures: 0,
                closed_until: None,
                last_failure: now,
                running: true,
            },
        );
        Ok(Attempt {
            throttle: self.clone(),
            key,
        })
    }

    #[cfg(test)]
    fn keys(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

impl Attempt {
    /// `failed`: the credentials were wrong.
    pub(crate) fn finish(self, failed: bool) {
        self.finish_at(failed, Instant::now());
    }

    fn finish_at(self, failed: bool, now: Instant) {
        let mut entries = self
            .throttle
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !failed {
            entries.remove(&self.key);
            return;
        }
        if let Some(entry) = entries.get_mut(&self.key) {
            entry.failures = entry.failures.saturating_add(1);
            entry.last_failure = now;
            if entry.failures >= FREE_FAILURES {
                let doublings = (entry.failures - FREE_FAILURES).min(16);
                let delay = FIRST_DELAY.saturating_mul(1 << doublings).min(MAX_DELAY);
                entry.closed_until = Some(now + delay);
            }
        }
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        let mut entries = self
            .throttle
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = entries.get_mut(&self.key) {
            entry.running = false;
            if entry.failures == 0 {
                entries.remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(value: &str) -> Option<IpAddr> {
        Some(value.parse().unwrap())
    }

    #[test]
    fn failures_back_off_boundedly_and_success_clears() {
        let throttle = Throttle::default();
        let key = Key::new("User@Example.test", ip("192.0.2.1"));
        let mut now = Instant::now();
        for _ in 0..FREE_FAILURES - 1 {
            throttle.admit_at(key, now).unwrap().finish_at(true, now);
        }
        // The fifth failure closes the key for 1 s, then 2 s, 4 s, ... 300 s.
        throttle.admit_at(key, now).unwrap().finish_at(true, now);
        assert!(matches!(throttle.admit_at(key, now), Err(Error::Busy)));
        let mut delay = FIRST_DELAY;
        for _ in 0..12 {
            assert!(matches!(
                throttle.admit_at(key, now + delay - Duration::from_millis(1)),
                Err(Error::Busy)
            ));
            now += delay;
            throttle.admit_at(key, now).unwrap().finish_at(true, now);
            delay = (delay * 2).min(MAX_DELAY);
        }
        assert_eq!(delay, MAX_DELAY);
        // Never locked for good: open again after the longest delay.
        now += MAX_DELAY;
        throttle.admit_at(key, now).unwrap().finish_at(false, now);
        assert_eq!(throttle.keys(), 0);
        throttle.admit_at(key, now).unwrap().finish_at(true, now);
        throttle.admit_at(key, now).unwrap();
    }

    #[test]
    fn keys_are_per_account_and_source_without_an_account_oracle() {
        let throttle = Throttle::default();
        let now = Instant::now();
        let key = Key::new("victim@example.test", ip("192.0.2.1"));
        for _ in 0..FREE_FAILURES {
            throttle.admit_at(key, now).unwrap().finish_at(true, now);
        }
        assert!(matches!(throttle.admit_at(key, now), Err(Error::Busy)));
        // The same account normalized, and an IPv4-mapped peer, are the key.
        assert_eq!(
            key,
            Key::new(" VICTIM@example.test ", ip("::ffff:192.0.2.1"))
        );
        // Another source or another account is not limited by it.
        throttle
            .admit_at(Key::new("victim@example.test", ip("192.0.2.2")), now)
            .unwrap();
        throttle
            .admit_at(Key::new("other@example.test", ip("192.0.2.1")), now)
            .unwrap();
        // An IPv6 source counts by its /64.
        assert_eq!(
            Key::new("a@example.test", ip("2001:db8:1:2::1")),
            Key::new("a@example.test", ip("2001:db8:1:2:ffff::9"))
        );
        assert_ne!(
            Key::new("a@example.test", ip("2001:db8:1:2::1")),
            Key::new("a@example.test", ip("2001:db8:1:3::1"))
        );
    }

    #[test]
    fn one_attempt_of_a_key_runs_at_a_time_and_cancellation_counts_nothing() {
        let throttle = Throttle::default();
        let now = Instant::now();
        let key = Key::new("a@example.test", None);
        let running = throttle.admit_at(key, now).unwrap();
        assert!(matches!(throttle.admit_at(key, now), Err(Error::Busy)));
        drop(running);
        assert_eq!(throttle.keys(), 0);
        throttle.admit_at(key, now).unwrap().finish_at(true, now);
        let running = throttle.admit_at(key, now).unwrap();
        drop(running);
        // The earlier failure is kept; the cancelled attempt added none.
        assert_eq!(throttle.keys(), 1);
    }

    #[test]
    fn failures_are_forgotten_and_the_table_is_bounded() {
        let throttle = Throttle::default();
        let now = Instant::now();
        let key = Key::new("a@example.test", ip("192.0.2.1"));
        for _ in 0..FREE_FAILURES {
            throttle.admit_at(key, now).unwrap().finish_at(true, now);
        }
        throttle
            .admit_at(key, now + FORGET_AFTER)
            .unwrap()
            .finish_at(true, now + FORGET_AFTER);
        // Forgotten: the failure after it is the first again.
        throttle.admit_at(key, now + FORGET_AFTER).unwrap();
        let throttle = Throttle::default();
        for n in 0..MAX_KEYS + 10 {
            let key = Key::new(&format!("u{n}@example.test"), ip("192.0.2.1"));
            throttle.admit_at(key, now).unwrap().finish_at(true, now);
        }
        assert_eq!(throttle.keys(), MAX_KEYS);
    }
}
