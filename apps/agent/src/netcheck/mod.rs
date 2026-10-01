//! Independent, unauthenticated STUN observations; never runtime carrier candidates.

use meshway_protocol::stun;

use std::{collections::HashSet, io, net::SocketAddr, time::Duration};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::{net::UdpSocket, time::Instant};

/// The source cannot be confused with a daemon/carrier observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    IndependentUdpProbe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AddressFamily {
    Ipv4,
    Ipv6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Mapped,
    Timeout,
    InvalidResponse,
    IoError,
    StunError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub server: String,
    pub outcome: Outcome,
    #[serde(deserialize_with = "required_nullable")]
    pub mapped_endpoint: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub rtt_ms: Option<u32>,
    pub elapsed_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    #[serde(deserialize_with = "schema_version")]
    pub schema_version: u8,
    pub source: Source,
    pub measured_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    pub bound_socket: String,
    pub address_family: AddressFamily,
    /// At least one valid UDP STUN response in this check, not universal UDP availability.
    pub udp_observed: bool,
    #[serde(deserialize_with = "required_nullable")]
    pub mapping_varies_by_destination: Option<bool>,
    #[serde(deserialize_with = "required_nullable")]
    pub mapping_stable_during_check: Option<bool>,
    pub samples: Vec<Sample>,
}

fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

fn schema_version<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u8, D::Error> {
    let version = u8::deserialize(deserializer)?;
    if version != 1 {
        return Err(serde::de::Error::custom(
            "unsupported netcheck schema version",
        ));
    }
    Ok(version)
}

impl Report {
    /// Observation freshness only; the closed temporary socket is never a usable candidate.
    /// Reject future, expired, unsupported, and improperly extended observation windows.
    pub fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        self.schema_version == 1
            && self.measured_at <= self.completed_at
            && self.completed_at <= now
            && self.completed_at - self.measured_at <= chrono::Duration::seconds(15)
            && self
                .measured_at
                .checked_add_signed(chrono::Duration::seconds(30))
                == Some(self.valid_until)
            && now < self.valid_until
    }
}

/// Validated before opening any socket, for both CLI and library callers.
#[derive(Debug, Clone)]
pub struct Config {
    servers: Vec<SocketAddr>,
    bind: SocketAddr,
    timeout: Duration,
}

impl Config {
    pub fn new(
        servers: Vec<SocketAddr>,
        bind: SocketAddr,
        timeout_ms: u64,
    ) -> Result<Self, String> {
        if !(1..=4).contains(&servers.len()) {
            return Err("--stun-server requires 1..4 explicit numeric endpoints".into());
        }
        if !(250..=3000).contains(&timeout_ms) {
            return Err("--timeout-ms must be 250..3000".into());
        }
        validate_address(bind, true)?;
        let mut seen = HashSet::new();
        for server in &servers {
            validate_address(*server, false)?;
            if server.is_ipv4() != bind.is_ipv4() {
                return Err("bind and all STUN servers must have the same address family; IPv6 requires --bind [::]:0".into());
            }
            if !seen.insert(*server) {
                return Err("duplicate STUN server endpoint".into());
            }
        }
        Ok(Self {
            servers,
            bind,
            timeout: Duration::from_millis(timeout_ms),
        })
    }
}

pub fn parse_endpoint(value: &str) -> Result<SocketAddr, String> {
    if value.contains('%') {
        return Err("IPv6 scope IDs are not supported".into());
    }
    value
        .parse()
        .map_err(|_| "requires a numeric IP:port (IPv6: [IP]:port); DNS is not supported".into())
}

fn validate_address(address: SocketAddr, bind: bool) -> Result<(), String> {
    if let SocketAddr::V6(v6) = address
        && (v6.scope_id() != 0 || v6.flowinfo() != 0 || v6.ip().to_ipv4_mapped().is_some())
    {
        return Err("IPv6 scope IDs, flow labels and IPv4-mapped IPv6 are not supported".into());
    }
    if address.ip().is_multicast()
        || matches!(address.ip(), std::net::IpAddr::V4(ip) if ip.is_broadcast())
        || (!bind && (address.ip().is_unspecified() || address.port() == 0))
    {
        return Err("requires a unicast, non-unspecified STUN IP and nonzero port; bind may use unspecified IP and port 0".into());
    }
    Ok(())
}

/// One unconnected socket and fresh transaction IDs for A, B, C, ..., A.
/// No reuse options, state, daemon, DNS, credentials, or candidate publication.
pub async fn measure(config: Config) -> io::Result<Report> {
    let measured_at = Utc::now();
    let total_deadline = Instant::now() + Duration::from_secs(15);
    let socket = tokio::time::timeout_at(total_deadline, UdpSocket::bind(config.bind)).await??;
    let bound_socket = socket.local_addr()?.to_string();
    let mut order = config.servers.clone();
    if order.len() >= 2 {
        order.push(order[0]);
    }
    let mut samples = Vec::with_capacity(order.len());
    for server in order {
        samples.push(probe(&socket, server, config.timeout, total_deadline).await);
    }
    let completed_at = Utc::now();
    let (stable, varies) = comparisons(&config.servers, &samples);
    Ok(Report {
        schema_version: 1,
        source: Source::IndependentUdpProbe,
        measured_at,
        completed_at,
        valid_until: measured_at + chrono::Duration::seconds(30),
        bound_socket,
        address_family: if config.bind.is_ipv4() {
            AddressFamily::Ipv4
        } else {
            AddressFamily::Ipv6
        },
        udp_observed: samples
            .iter()
            .any(|sample| sample.outcome == Outcome::Mapped),
        mapping_varies_by_destination: varies,
        mapping_stable_during_check: stable,
        samples,
    })
}

fn comparisons(servers: &[SocketAddr], samples: &[Sample]) -> (Option<bool>, Option<bool>) {
    if servers.len() < 2 {
        return (None, None);
    }
    let mapped = |sample: &Sample| sample.outcome == Outcome::Mapped;
    let first = &samples[0];
    let last = &samples[samples.len() - 1];
    let stable =
        (mapped(first) && mapped(last)).then(|| first.mapped_endpoint == last.mapped_endpoint);
    let distinct_ips: HashSet<_> = servers.iter().map(SocketAddr::ip).collect();
    let varies = if stable == Some(true) && distinct_ips.len() >= 2 && samples.iter().all(mapped) {
        // Compare only observations at different destination IPs, not different ports.
        Some(servers.iter().enumerate().any(|(i, a)| {
            servers.iter().enumerate().any(|(j, b)| {
                a.ip() != b.ip() && samples[i].mapped_endpoint != samples[j].mapped_endpoint
            })
        }))
    } else {
        None
    };
    (stable, varies)
}

async fn probe(
    socket: &UdpSocket,
    server: SocketAddr,
    timeout: Duration,
    total: Instant,
) -> Sample {
    let start = Instant::now();
    let deadline = (start + timeout).min(total);
    let mut sample = Sample {
        server: server.to_string(),
        outcome: Outcome::Timeout,
        mapped_endpoint: None,
        rtt_ms: None,
        elapsed_ms: 0,
    };
    // timeout_at may poll an immediately ready IO future before its expired timer.
    // Do not send a later transaction after the absolute overall deadline.
    if start >= deadline {
        return sample;
    }
    let mut transaction = [0; 12];
    if getrandom::fill(&mut transaction).is_err() {
        sample.outcome = Outcome::IoError;
    } else {
        let request = stun::request(transaction);
        match tokio::time::timeout_at(deadline, socket.send_to(&request, server)).await {
            Err(_) => {}
            Ok(Err(_)) => sample.outcome = Outcome::IoError,
            Ok(Ok(_)) => {
                // Extra byte detects oversized/truncated UDP packets even with forged header length.
                let mut buffer = [0; 1025];
                let mut invalid = false;
                let mut packets = 0_u32;
                loop {
                    if Instant::now() >= deadline {
                        sample.outcome = if invalid {
                            Outcome::InvalidResponse
                        } else {
                            Outcome::Timeout
                        };
                        break;
                    }
                    match tokio::time::timeout_at(deadline, socket.recv_from(&mut buffer)).await {
                        Err(_) => {
                            sample.outcome = if invalid {
                                Outcome::InvalidResponse
                            } else {
                                Outcome::Timeout
                            };
                            break;
                        }
                        Ok(Err(_)) => {
                            sample.outcome = Outcome::IoError;
                            break;
                        }
                        Ok(Ok((len, peer))) => {
                            if Instant::now() >= deadline {
                                continue;
                            }
                            if peer == server {
                                match stun::parse(&buffer[..len], transaction, server.is_ipv4()) {
                                    stun::Response::Ignore => {}
                                    stun::Response::Invalid => invalid = true,
                                    stun::Response::Error => {
                                        sample.outcome = Outcome::StunError;
                                        break;
                                    }
                                    stun::Response::Mapped(endpoint) => {
                                        sample.outcome = Outcome::Mapped;
                                        sample.mapped_endpoint = Some(endpoint.to_string());
                                        sample.rtt_ms = Some(milliseconds(start.elapsed()));
                                        break;
                                    }
                                }
                            }
                            packets += 1;
                            // Explicit cooperative budget, including wrong-source traffic.
                            if packets.is_multiple_of(32) {
                                tokio::task::yield_now().await;
                            }
                        }
                    }
                }
            }
        }
    }
    sample.elapsed_ms = milliseconds(start.elapsed());
    sample
}

fn milliseconds(duration: Duration) -> u32 {
    u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_destination_ips_require_complete_stable_observations() {
        // Comparison semantics only. Wire behavior is exercised by real loopback servers.
        let servers = [
            "192.0.2.1:3478".parse().unwrap(),
            "192.0.2.2:3478".parse().unwrap(),
        ];
        let sample = |server: SocketAddr, endpoint: &str| Sample {
            server: server.to_string(),
            outcome: Outcome::Mapped,
            mapped_endpoint: Some(endpoint.into()),
            rtt_ms: Some(1),
            elapsed_ms: 1,
        };
        let a = sample(servers[0], "198.51.100.1:10000");
        let mut b = sample(servers[1], "198.51.100.1:10000");
        let mut last = a.clone();
        assert_eq!(
            comparisons(&servers, &[a.clone(), b.clone(), last.clone()]),
            (Some(true), Some(false))
        );
        b.mapped_endpoint = Some("198.51.100.1:10001".into());
        assert_eq!(
            comparisons(&servers, &[a.clone(), b.clone(), last.clone()]),
            (Some(true), Some(true))
        );
        last.mapped_endpoint = b.mapped_endpoint.clone();
        assert_eq!(
            comparisons(&servers, &[a.clone(), b.clone(), last]),
            (Some(false), None)
        );
        b.outcome = Outcome::Timeout;
        b.mapped_endpoint = None;
        assert_eq!(
            comparisons(&servers, &[a.clone(), b, a.clone()]),
            (Some(true), None)
        );
        let mut last = a.clone();
        last.outcome = Outcome::InvalidResponse;
        last.mapped_endpoint = None;
        assert_eq!(comparisons(&servers, &[a.clone(), a, last]), (None, None));
    }
}
