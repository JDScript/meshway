use std::{
    collections::{HashMap, hash_map::Entry},
    io,
    net::IpAddr,
    time::Duration,
};

use tokio::{net::UdpSocket, time::Instant};
use tokio_util::sync::CancellationToken;

use crate::{MAX_DATAGRAM_SIZE, encode_response, parse_request};

const WINDOW: Duration = Duration::from_secs(1);
const SOURCE_TTL: Duration = Duration::from_secs(60);
const SEND_DEADLINE: Duration = Duration::from_millis(100);

/// Fixed one-second windows; boundary bursts can span two adjacent windows.
/// All received datagrams, including malformed ones, consume request budgets.
#[derive(Clone, Copy, Debug)]
pub struct ServiceConfig {
    pub max_source_ips: usize,
    pub global_datagrams_per_second: u32,
    pub per_ip_datagrams_per_second: u32,
    pub global_output_bytes_per_second: u32,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            max_source_ips: 1024,
            global_datagrams_per_second: 1000,
            per_ip_datagrams_per_second: 10,
            global_output_bytes_per_second: 64 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid STUN service limits")]
pub struct ServiceConfigError;

/// Real cumulative counters, saturating at u64::MAX. Output counters count only
/// successful sends; output rate budgets conservatively reserve before sending.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServiceStats {
    pub received_datagrams: u64,
    pub empty_datagrams: u64,
    pub oversized_datagrams: u64,
    pub invalid_datagrams: u64,
    pub rate_limited_datagrams: u64,
    pub source_table_full_datagrams: u64,
    pub output_limited_datagrams: u64,
    pub sent_datagrams: u64,
    pub sent_bytes: u64,
    pub send_failures: u64,
    pub peak_source_ips: u64,
}

/// Owns only an already-bound socket and anonymous-service abuse budgets.
/// `run` is the sole future: no binds, detached tasks, retries, or background GC.
/// Caller may await it directly or spawn and explicitly cancel/join it. Dropping
/// the future also closes the socket; cancellation interrupts receive and send.
pub struct Service {
    socket: UdpSocket,
    limits: Limits,
}

impl Service {
    pub fn new(socket: UdpSocket, config: ServiceConfig) -> Result<Self, ServiceConfigError> {
        if config.max_source_ips == 0
            || config.max_source_ips > 4096
            || config.global_datagrams_per_second == 0
            || config.global_datagrams_per_second > 100_000
            || config.per_ip_datagrams_per_second == 0
            || config.per_ip_datagrams_per_second > config.global_datagrams_per_second
            || config.global_output_bytes_per_second == 0
            || config.global_output_bytes_per_second > 16 * 1024 * 1024
        {
            return Err(ServiceConfigError);
        }
        Ok(Self {
            socket,
            limits: Limits::new(config),
        })
    }

    pub async fn run(mut self, cancellation: CancellationToken) -> io::Result<ServiceStats> {
        let mut stats = ServiceStats::default();
        // A MAX+1 buffer distinguishes oversized/truncated datagrams from a
        // valid datagram exactly at MAX, including on Tokio's truncating recv.
        let mut buffer = [0; MAX_DATAGRAM_SIZE + 1];
        loop {
            let (length, source) = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Ok(stats),
                received = self.socket.recv_from(&mut buffer) => received?,
            };
            increment(&mut stats.received_datagrams);
            if length == 0 {
                increment(&mut stats.empty_datagrams);
            }
            if length > MAX_DATAGRAM_SIZE {
                increment(&mut stats.oversized_datagrams);
            }
            match self.limits.admit(source.ip(), Instant::now()) {
                Admission::Rate => {
                    increment(&mut stats.rate_limited_datagrams);
                    continue;
                }
                Admission::TableFull => {
                    increment(&mut stats.source_table_full_datagrams);
                    continue;
                }
                Admission::Allowed => {}
            }
            stats.peak_source_ips = stats.peak_source_ips.max(self.limits.sources.len() as u64);
            if length == 0 || length > MAX_DATAGRAM_SIZE {
                continue;
            }
            let request = match parse_request(&buffer[..length]) {
                Ok(request) => request,
                Err(_) => {
                    increment(&mut stats.invalid_datagrams);
                    continue;
                }
            };
            let response = encode_response(&request, source);
            // UDP payload ratio <= 2.2, sufficient for a bare IPv6 Binding
            // exchange (20 -> 44). This is not zero amplification/source auth.
            if response.len() * 5 > length * 11 || !self.limits.reserve_output(response.len()) {
                increment(&mut stats.output_limited_datagrams);
                continue;
            }
            let sent = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Ok(stats),
                result = tokio::time::timeout(SEND_DEADLINE, self.socket.send_to(&response, source)) => result,
            };
            match sent {
                Ok(Ok(length)) if length == response.len() => {
                    increment(&mut stats.sent_datagrams);
                    stats.sent_bytes = stats.sent_bytes.saturating_add(length as u64);
                }
                _ => increment(&mut stats.send_failures),
            }
        }
    }
}

fn increment(value: &mut u64) {
    *value = value.saturating_add(1);
}

fn normalized_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ip)),
        ip => ip,
    }
}

struct Source {
    window: Instant,
    last_seen: Instant,
    datagrams: u32,
}

struct Limits {
    config: ServiceConfig,
    window: Instant,
    datagrams: u32,
    output_bytes: u32,
    sources: HashMap<IpAddr, Source>,
}

#[derive(Debug, Eq, PartialEq)]
enum Admission {
    Allowed,
    Rate,
    TableFull,
}

impl Limits {
    fn new(config: ServiceConfig) -> Self {
        Self {
            config,
            window: Instant::now(),
            datagrams: 0,
            output_bytes: 0,
            sources: HashMap::with_capacity(config.max_source_ips),
        }
    }

    fn admit(&mut self, ip: IpAddr, now: Instant) -> Admission {
        if now.duration_since(self.window) >= WINDOW {
            self.window = now;
            self.datagrams = 0;
            self.output_bytes = 0;
            // At most one bounded table scan per second, independent of the
            // number of hostile source IPs. Never evict a live entry to admit
            // a new IP when full. No per-source futures or timers.
            self.sources
                .retain(|_, source| now.duration_since(source.last_seen) < SOURCE_TTL);
        }
        if self.datagrams >= self.config.global_datagrams_per_second {
            return Admission::Rate;
        }
        self.datagrams += 1;
        let ip = normalized_ip(ip);
        let full = self.sources.len() >= self.config.max_source_ips;
        let source = match self.sources.entry(ip) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                if full {
                    return Admission::TableFull;
                }
                entry.insert(Source {
                    window: now,
                    last_seen: now,
                    datagrams: 0,
                })
            }
        };
        source.last_seen = now;
        if now.duration_since(source.window) >= WINDOW {
            source.window = now;
            source.datagrams = 0;
        }
        if source.datagrams >= self.config.per_ip_datagrams_per_second {
            return Admission::Rate;
        }
        source.datagrams += 1;
        Admission::Allowed
    }

    fn reserve_output(&mut self, length: usize) -> bool {
        let Ok(length) = u32::try_from(length) else {
            return false;
        };
        if length > self.config.global_output_bytes_per_second - self.output_bytes {
            return false;
        }
        self.output_bytes += length;
        true
    }
}

#[cfg(test)]
mod tests;
