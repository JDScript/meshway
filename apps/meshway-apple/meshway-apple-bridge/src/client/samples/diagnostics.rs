//! Ping and netcheck results in the order a Node streams them. Each value is
//! an `ipc` type; the caller frames it with the production codec.
use super::{ClientDiagnosticSample, addresses, netmap, node};
use chrono::{DateTime, Duration, Utc};
use meshway_configuration::execution::UnavailableReason;
use meshway_model::{
    Digest, EventId, Sequence, SocketAddress, TransportProtocol, Version,
    observations::{LinkAcquisition, ObservationSample, ProbeChannel, ProbeChannels, ProbeWindow},
    state::{ProbeKind, ProbeObservation, ProbeOutcome, ResourceRef},
};
use meshway_protocol::{NodeObservationTargetView, ipc};

const SELF: u8 = 2;
const TOKYO: u8 = 3;
const NAS: u8 = 4;
const OFFICE: u8 = 6;

pub(super) fn results(sample: ClientDiagnosticSample) -> Vec<ipc::ResultBody> {
    let now = Utc::now();
    let events = match sample {
        ClientDiagnosticSample::Ping => mesh_ping(now),
        ClientDiagnosticSample::PingExit => exit_ping(),
        ClientDiagnosticSample::PingDenied => vec![
            decision(
                Some("100.64.0.6"),
                Some(target(OFFICE, "office-mac")),
                None,
                Some(ipc::PingAction::Denied {
                    reason: UnavailableReason::Unauthorized,
                }),
                vec![],
            ),
            ipc::PingEvent::Failed {
                code: ipc::PingError::PermissionDenied,
            },
        ],
        ClientDiagnosticSample::PingUnsupported => vec![
            decision(
                Some("203.0.113.9"),
                None,
                Some("work"),
                Some(ipc::PingAction::Direct),
                vec![],
            ),
            ipc::PingEvent::Failed {
                code: ipc::PingError::Unsupported,
            },
        ],
        ClientDiagnosticSample::Netcheck => {
            return vec![ipc::ResultBody::Netcheck {
                report: Box::new(netcheck(now)),
            }];
        }
    };
    events
        .into_iter()
        .enumerate()
        .map(|(index, event)| ipc::ResultBody::Ping {
            sequence: Sequence::new(index as u64).expect("sample sequence"),
            event,
        })
        .collect()
}

fn target(number: u8, hostname: &str) -> NodeObservationTargetView {
    NodeObservationTargetView {
        node_id: node(number),
        hostname: hostname.into(),
        addresses: addresses(
            &format!("100.64.0.{number}"),
            &format!("fd89:98a2:2060::{number}"),
        ),
    }
}

fn decision(
    destination: Option<&str>,
    node: Option<NodeObservationTargetView>,
    profile: Option<&str>,
    action: Option<ipc::PingAction>,
    path: Vec<meshway_model::NodeId>,
) -> ipc::PingEvent {
    ipc::PingEvent::Decision {
        decision: ipc::PingDecision {
            destination: destination.map(Into::into),
            node,
            resolution: None,
            profile: profile.map(Into::into),
            rule: None,
            action,
            path,
        },
    }
}

fn id<T: std::str::FromStr>(number: u32) -> T
where
    T::Err: std::fmt::Debug,
{
    format!("00000000-0000-4000-8000-{number:012}")
        .parse()
        .expect("sample id")
}

fn socket(text: &str) -> SocketAddress {
    text.parse().expect("sample socket")
}

/// studio-nas through meshway-tokyo: four measured windows, the third unanswered.
fn mesh_ping(now: DateTime<Utc>) -> Vec<ipc::PingEvent> {
    let path = vec![node(SELF), node(TOKYO), node(NAS)];
    let observation_id = id(40);
    let channel = ProbeChannel {
        link_id: Digest::from_bytes([0x31; 32]),
        socket_id: Digest::from_bytes([0x32; 32]),
        peer_node_id: node(TOKYO),
        transport: TransportProtocol::NativeUdp,
        local_address: socket("192.168.1.20:41641"),
        remote_address: socket("192.0.2.3:41641"),
        acquisition: LinkAcquisition::PublishedEndpoint,
    };
    let mut events = vec![
        decision(
            Some("100.64.0.4"),
            Some(target(NAS, "studio-nas")),
            None,
            Some(ipc::PingAction::Mesh),
            path.clone(),
        ),
        ipc::PingEvent::Accepted {
            subscription_id: id(41),
            observation_id,
            target: target(NAS, "studio-nas"),
        },
        ipc::PingEvent::Preparing {
            elapsed_ms: Sequence::new(180).expect("sample elapsed"),
        },
    ];
    let durations = [Some(24_310), Some(22_870), None, Some(23_540)];
    for (index, duration) in durations.into_iter().enumerate() {
        let start = now - Duration::seconds(4 - index as i64);
        let end =
            start + duration.map_or(Duration::seconds(1), |us| Duration::microseconds(us as i64));
        let measurement_id: EventId = id(50 + index as u32);
        let outcome = if duration.is_some() {
            ProbeOutcome::Success
        } else {
            ProbeOutcome::Timeout
        };
        events.push(ipc::PingEvent::Sample {
            observation_id,
            sample: ObservationSample {
                measurement_id,
                version: Version::MIN,
                window_start: start,
                window_end: end,
                partial: false,
                probe: Some(ProbeObservation {
                    measurement_id,
                    kind: ProbeKind::Path,
                    link_id: None,
                    transport: None,
                    channels: Some(ProbeChannels {
                        request: channel.clone(),
                        reply: duration.map(|_| channel.clone()),
                    }),
                    path: path.clone(),
                    netmap: netmap(),
                    started_at: start,
                    completed_at: Some(end),
                    outcome,
                    duration_us: duration.map(|us| Sequence::new(us).expect("sample RTT")),
                }),
                traffic: None,
                probe_counts: Some(ProbeWindow {
                    sent: 1,
                    replied_within_deadline: u32::from(duration.is_some()),
                    timeout: u32::from(duration.is_none()),
                    pending: 0,
                }),
            },
        });
    }
    events.push(ipc::PingEvent::Completed {
        sent: 4,
        success: 3,
        timeout: 1,
        failed: 0,
    });
    events
}

/// A name decided by a rule toward the meshway-tokyo exit, then echoed.
fn exit_ping() -> Vec<ipc::PingEvent> {
    let path = vec![node(SELF), node(TOKYO)];
    let mut events = vec![ipc::PingEvent::Decision {
        decision: ipc::PingDecision {
            destination: Some("198.51.100.20".into()),
            node: None,
            resolution: Some(ipc::PingResolution {
                name: "intranet.example".into(),
                resolver: ipc::PingResolver::Exit { node: node(TOKYO) },
                addresses: vec!["198.51.100.20".into()],
                association: Some(ipc::PingAssociation {
                    names: vec!["intranet.example".into()],
                    remaining_seconds: 240,
                }),
            }),
            profile: Some("work".into()),
            rule: Some(ipc::PingRule {
                pointer: "/routing/profiles/work/rules/0".into(),
                remark: "Company intranet".into(),
                rule_set: None,
            }),
            action: Some(ipc::PingAction::Exit { node: node(TOKYO) }),
            path: path.clone(),
        },
    }];
    let echoes = [
        (ipc::PingEchoOutcome::Reply, Some(61_200), true),
        (ipc::PingEchoOutcome::Reply, Some(38_450), false),
        (ipc::PingEchoOutcome::Timeout, None, false),
        (ipc::PingEchoOutcome::Reply, Some(37_980), false),
    ];
    for (index, (outcome, duration, setup)) in echoes.into_iter().enumerate() {
        events.push(ipc::PingEvent::Echo {
            echo: ipc::PingEcho {
                sequence: index as u16 + 1,
                outcome,
                from: duration.map(|_| "198.51.100.20".into()),
                error: None,
                duration_us: duration.map(|us| Sequence::new(us).expect("sample RTT")),
                setup,
                path: if duration.is_some() {
                    path.clone()
                } else {
                    vec![]
                },
            },
        });
    }
    events.push(ipc::PingEvent::Completed {
        sent: 4,
        success: 3,
        timeout: 1,
        failed: 0,
    });
    events
}

/// Endpoint-dependent IPv4 mapping (two observers, two ports) and no IPv6.
fn netcheck(now: DateTime<Utc>) -> ipc::NetcheckReport {
    let first = socket("198.51.100.1:3478");
    let second = socket("203.0.113.1:3478");
    ipc::NetcheckReport {
        observed_at: now - Duration::seconds(8),
        server: ipc::NetcheckServer {
            origin: "https://mesh.example".into(),
            reachable: Some(true),
            rtt_us: Some(Sequence::new(42_800).expect("sample RTT")),
        },
        unavailable: None,
        ipv4: true,
        ipv6: false,
        udp: true,
        sockets: vec![ipc::NetcheckSocket {
            resource: ResourceRef::Listener { key: "udp".into() },
            bound_address: socket("0.0.0.0:41641"),
            mappings: vec![
                ipc::StunMapping {
                    observer: first,
                    mapped: socket("192.0.2.80:52311"),
                    rtt_us: Some(Sequence::new(18_900).expect("sample RTT")),
                    fresh: true,
                },
                ipc::StunMapping {
                    observer: second,
                    mapped: socket("192.0.2.80:52977"),
                    rtt_us: Some(Sequence::new(31_400).expect("sample RTT")),
                    fresh: true,
                },
            ],
            port_mapping: ipc::PortMapping::Changed,
            endpoint_mapping: ipc::EndpointMapping::Dependent,
        }],
        observers: vec![
            ipc::NetcheckObserver {
                observer: first,
                rtt_us: Some(Sequence::new(18_900).expect("sample RTT")),
                size_probe: Some(ipc::NetcheckSizeProbe {
                    size: 1500,
                    answered: true,
                }),
                path_mtu: Some(1500),
            },
            ipc::NetcheckObserver {
                observer: second,
                rtt_us: Some(Sequence::new(31_400).expect("sample RTT")),
                size_probe: None,
                path_mtu: None,
            },
        ],
    }
}
