//! Formatting and machine-readable CLI results for the private daemon API.
use std::{path::Path, time::Duration};

use meshway_agent::diagnostics::{self, ProbeError, ProbeObservation, SCHEMA_VERSION};
use serde::Serialize;

#[derive(Serialize)]
#[serde(
    tag = "status",
    content = "result",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum Outcome<T> {
    Ok(T),
    Error(ProbeError),
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct CommandResult<T> {
    schema_version: u32,
    outcome: Outcome<T>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct PingResult {
    schema_version: u32,
    requested_count: u8,
    samples: Vec<PingSample>,
    result: Outcome<()>,
    /// The existing CLI stops at the first failed sample.
    stopped_on_failure: bool,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct PingSample {
    sequence: u8,
    outcome: Outcome<ProbeObservation>,
}

/// Keep JSON valid while escaping HTML and terminal control/bidi characters in
/// inventory-controlled names. This also applies to machine-readable output.
pub(crate) fn escaped_json(value: &impl Serialize) -> Result<String, serde_json::Error> {
    Ok(serde_json::to_string(value)?
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(character, '<' | '>' | '&' | '\u{2028}' | '\u{2029}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                format!("\\u{:04x}", character as u32)
            } else {
                character.to_string()
            }
        })
        .collect())
}

fn finish<T: Serialize>(
    result: Result<T, ProbeError>,
    json: bool,
    human: impl FnOnce(&T),
) -> Result<(), Box<dyn std::error::Error>> {
    match result {
        Ok(value) => {
            if json {
                println!(
                    "{}",
                    escaped_json(&CommandResult {
                        schema_version: SCHEMA_VERSION,
                        outcome: Outcome::Ok(value)
                    })?
                );
            } else {
                human(&value);
            }
            Ok(())
        }
        Err(error) => {
            if json {
                println!(
                    "{}",
                    escaped_json(&CommandResult::<()> {
                        schema_version: SCHEMA_VERSION,
                        outcome: Outcome::Error(error)
                    })?
                );
            } else {
                println!("unknown: {error}");
            }
            Err(error.into())
        }
    }
}

pub async fn status(
    state: &Path,
    json: bool,
    peers: bool,
    offset: u32,
    limit: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = diagnostics::live_status(state, peers, offset, limit).await;
    finish(result, json, |status| {
        println!(
            "installed revision={} source_revision={} generation={} lease_remaining_ms={}",
            status.installed_revision,
            status.source_revision,
            status.generation,
            status.lease_remaining_ms
        );
        println!(
            "local={} identity={}",
            status.local_node_id,
            escaped_json(&status.local_identity).expect("public identity is serializable")
        );
        println!(
            "authenticated_channels={} evidence=current_authenticated_channel_registry (not business reachability)",
            status.authenticated_channel_count
        );
        if let Some(peers) = &status.peers {
            for peer in peers {
                println!(
                    "{}",
                    escaped_json(peer).expect("public channel is serializable")
                );
            }
        }
        if let Some(next) = status.next_peer_offset {
            println!(
                "more peers: --peers --peer-offset {next} --peer-limit {limit} (pages are independent point-in-time observations)"
            );
        }
        println!("DNS: not observed by this interface");
    })
}

pub async fn whois(
    state: &Path,
    query: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    finish(
        diagnostics::whois(state, query).await,
        json,
        |observation| {
            println!(
                "{}",
                escaped_json(&observation.identity).expect("public identity is serializable")
            );
            println!(
                "installed_revision={} source_revision={} generation={} evidence=current_trusted_inventory",
                observation.installed_revision, observation.source_revision, observation.generation
            );
        },
    )
}

pub async fn ping(
    state: &Path,
    peer: &str,
    count: u8,
    timeout_seconds: u64,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut samples = Vec::with_capacity(count as usize);
    let mut failure = None;
    if !json {
        println!("authenticated neighbor probe / 认证邻接探测 (非ICMP，非mesh trace)");
    }
    for sequence in 1..=count {
        match diagnostics::probe(state, peer, Duration::from_secs(timeout_seconds)).await {
            Ok(observation) => {
                if !json {
                    let direction = if observation.initiator_node_id == observation.local_node_id {
                        "outgoing (locally initiated)"
                    } else {
                        "accepted (peer initiated)"
                    };
                    let transport = match observation.transport {
                        meshway_protocol::ConnectionTransport::NativeUdp => "native_udp",
                        meshway_protocol::ConnectionTransport::NativeTcp => "native_tcp",
                        meshway_protocol::ConnectionTransport::Vless => "vless (TLS)",
                    };
                    println!(
                        "seq={sequence} peer={} name={} direction={direction} transport={transport} initiator={} RTT={}.{:03} µs",
                        observation.peer_node_id,
                        escaped_json(&observation.peer_name)?,
                        observation.initiator_node_id,
                        observation.rtt_ns / 1000,
                        observation.rtt_ns % 1000
                    );
                }
                samples.push(PingSample {
                    sequence,
                    outcome: Outcome::Ok(observation),
                });
            }
            Err(error) => {
                samples.push(PingSample {
                    sequence,
                    outcome: Outcome::Error(error),
                });
                failure = Some(error);
                break;
            }
        }
        if sequence < count {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    if json {
        println!(
            "{}",
            escaped_json(&PingResult {
                schema_version: SCHEMA_VERSION,
                requested_count: count,
                samples,
                result: match failure {
                    Some(error) => Outcome::Error(error),
                    None => Outcome::Ok(()),
                },
                stopped_on_failure: failure.is_some()
            })?
        );
    }
    if let Some(error) = failure {
        return Err(error.into());
    }
    Ok(())
}
