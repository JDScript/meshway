//! Debug-only visual fixtures built as real IPC values, serialized by its codec.
use super::ClientError;
#[cfg(any(debug_assertions, test))]
use chrono::{Duration, Utc};
#[cfg(any(debug_assertions, test))]
use meshway_configuration::{NodeAddresses, selection::*};
#[cfg(any(debug_assertions, test))]
use meshway_model::{NetmapRef, NodeId, Sequence, Version};
#[cfg(any(debug_assertions, test))]
use meshway_protocol::ipc;

#[derive(Clone, Copy, uniffi::Enum)]
pub enum ClientSample {
    Unenrolled,
    AwaitingApproval,
    Running,
    Error,
}

#[cfg(any(debug_assertions, test))]
pub(super) fn status(sample: ClientSample) -> ipc::Status {
    let now = Utc::now();
    let mut status = ipc::Status {
        process_id: 1234,
        revision: Version::MIN,
        desired_up: false,
        phase: ipc::Phase::Down,
        server: "https://mesh.example/".into(),
        observed_at: now - Duration::minutes(12),
        control_observed_at: Some(now),
        enrollment_request_id: None,
        enrollment_url: None,
        identity: None,
        authenticated: false,
        sync: ipc::SyncState::Disconnected,
        last_sync_at: None,
        sync_error: None,
        instance: None,
        netmap: None,
        saved: None,
        applied: None,
        data_plane: ipc::DataPlane::Stopped,
        network_mode: ipc::NetworkMode::Tun,
        exit: None,
        node: None,
        peers: Vec::new(),
        sockets: Vec::new(),
        last_error: None,
        last_error_detail: None,
        retry_at: None,
        network_error: None,
        last_network_change: None,
        change_pending: None,
        control_relay: None,
    };
    match sample {
        ClientSample::Unenrolled => {}
        ClientSample::AwaitingApproval => {
            status.desired_up = true;
            status.phase = ipc::Phase::AwaitingApproval;
            status.enrollment_request_id = Some(
                "00000000-0000-4000-8000-000000000010"
                    .parse()
                    .expect("sample enrollment"),
            );
            status.enrollment_url = Some("https://console.example/approve/sample".into());
        }
        ClientSample::Running | ClientSample::Error => {
            status.desired_up = true;
            status.phase = ipc::Phase::Running;
            status.data_plane = ipc::DataPlane::Ready;
            status.authenticated = true;
            status.instance = Some(meshway_protocol::InstanceLease {
                instance_id: "00000000-0000-4000-8000-000000000030"
                    .parse()
                    .expect("sample instance"),
                generation: Version::MIN,
                expires_at_ms: (now + Duration::minutes(10))
                    .timestamp_millis()
                    .try_into()
                    .expect("sample timestamp"),
            });
            let key = |number, algorithm| meshway_model::BoundNodeKey {
                id: format!("00000000-0000-4000-8000-{number:012}")
                    .parse()
                    .expect("sample key id"),
                generation: Version::MIN,
                public_key: meshway_model::NodePublicKey {
                    algorithm,
                    bytes: meshway_model::Digest::from_bytes([number; 32]),
                },
            };
            status.identity = Some(meshway_protocol::EnrollmentResponse {
                controller_id: "00000000-0000-4000-8000-000000000011"
                    .parse()
                    .expect("sample controller"),
                network_id: "00000000-0000-4000-8000-000000000012"
                    .parse()
                    .expect("sample network"),
                node_id: "00000000-0000-4000-8000-000000000002"
                    .parse()
                    .expect("sample node"),
                keys: meshway_protocol::NodeKeyMetadata {
                    control: key(20, meshway_model::KeyAlgorithm::Ed25519),
                    dataplane: key(21, meshway_model::KeyAlgorithm::X25519),
                },
                server_signing_key: meshway_protocol::netmap::ServerSigningKey {
                    key_id: key(22, meshway_model::KeyAlgorithm::Ed25519).id,
                    key_generation: Version::MIN,
                    public_key: key(22, meshway_model::KeyAlgorithm::Ed25519).public_key,
                },
            });
            status.sync = ipc::SyncState::Connected;
            status.last_sync_at = Some(now - Duration::seconds(24));
            status.node = Some(ipc::NodeSelf {
                hostname: "MacBook Pro".into(),
                addresses: addresses("100.64.0.2", "fd89:98a2:2060::2"),
            });
            let relay: NodeId = "00000000-0000-4000-8000-000000000003"
                .parse()
                .expect("sample id");
            status.peers = vec![
                ipc::PeerStatus {
                    node_id: relay,
                    hostname: "meshway-tokyo".into(),
                    addresses: addresses("100.64.0.3", "fd89:98a2:2060::3"),
                    state: ipc::PeerState::Direct,
                    transport: Some(meshway_model::TransportProtocol::NativeUdp),
                    endpoint: Some("192.0.2.3:41641".parse().expect("sample socket")),
                    mtu: Some(ipc::PeerMtu {
                        path: 1500,
                        inner: 1386,
                    }),
                    via: vec![],
                    rtt: Some(ipc::PeerRtt {
                        duration_us: Sequence::new(18420).expect("sample duration"),
                        observed_at: now - Duration::minutes(3),
                    }),
                    idle_path: None,
                },
                ipc::PeerStatus {
                    node_id: "00000000-0000-4000-8000-000000000004"
                        .parse()
                        .expect("sample id"),
                    hostname: "studio-nas".into(),
                    addresses: addresses("100.64.0.4", "fd89:98a2:2060::4"),
                    state: ipc::PeerState::Relay,
                    transport: Some(meshway_model::TransportProtocol::NativeUdp),
                    endpoint: None,
                    mtu: None,
                    via: vec![relay],
                    rtt: None,
                    idle_path: None,
                },
                ipc::PeerStatus {
                    node_id: "00000000-0000-4000-8000-000000000005"
                        .parse()
                        .expect("sample id"),
                    hostname: "home-linux".into(),
                    addresses: addresses("100.64.0.5", "fd89:98a2:2060::5"),
                    state: ipc::PeerState::Idle,
                    transport: None,
                    endpoint: None,
                    mtu: None,
                    via: vec![],
                    rtt: None,
                    idle_path: Some(ipc::IdlePath {
                        route: ipc::IdleRoute::Direct,
                        via: vec![],
                    }),
                },
            ];
            let reference = NetmapRef {
                revision: meshway_model::Revision::MIN,
                serial: Version::MIN,
                digest: meshway_model::Digest::from_bytes([0xaa; 32]),
            };
            status.saved = Some(reference);
            status.applied = Some(reference);
            status.netmap = Some(ipc::NetmapStatus {
                netmap: reference,
                issued_at: now - Duration::minutes(12),
                expires_at: now + Duration::hours(23),
                missing_artifacts: vec![],
                artifact_problem: None,
                capture: None,
                routing: Some(RoutingSelectionState {
                    requested_profile: Some("work".into()),
                    version: Some(Version::MIN),
                    applied_profile: "work".into(),
                    requested_unavailable: None,
                    default_profile: "mesh".into(),
                    locked: false,
                    profiles: vec![
                        ProfileSummary {
                            name: "mesh".into(),
                            kind: ProfileKind::Mesh,
                            rules: 0,
                            exits: vec![],
                            fallback: Some(meshway_configuration::RouteAction::Unmanaged),
                        },
                        ProfileSummary {
                            name: "work".into(),
                            kind: ProfileKind::Custom,
                            rules: 2,
                            exits: vec![relay],
                            fallback: Some(meshway_configuration::RouteAction::Exit {
                                nodes: vec![format!("node:{relay}")],
                                via: vec![],
                            }),
                        },
                    ],
                    auto: None,
                }),
            });
            if matches!(sample, ClientSample::Error) {
                status.authenticated = false;
                status.instance = None;
                status.phase = ipc::Phase::Blocked;
                status.data_plane = ipc::DataPlane::Failed;
                status.applied = None;
                status.node = None;
                status.peers.clear();
                status.last_error = Some(ipc::Problem::Credentials);
                status.network_error = Some(ipc::NetworkError {
                    problem: ipc::Problem::Dns,
                    detail: Some(
                        "DNS settings could not be installed. Packet output is suspended.".into(),
                    ),
                });
                status.sync = ipc::SyncState::Reconnecting;
                status.sync_error = Some(ipc::SyncError::Credentials);
            }
        }
    }
    status
}
#[cfg(any(debug_assertions, test))]
fn addresses(ipv4: &str, ipv6: &str) -> NodeAddresses {
    NodeAddresses {
        ipv4: Some(ipv4.parse().expect("sample IPv4")),
        ipv6: Some(ipv6.parse().expect("sample IPv6")),
    }
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn client_sample_frame(
    sample: ClientSample,
    request_id: String,
) -> Result<Vec<u8>, ClientError> {
    // Keep binding metadata stable when a Debug App links an optimized core.
    // Release libraries contain no fixture data and cannot produce samples.
    #[cfg(not(any(debug_assertions, test)))]
    {
        let _ = (sample, request_id);
        Err(ClientError::Input)
    }
    #[cfg(any(debug_assertions, test))]
    {
        let response = ipc::Response {
            version: ipc::VERSION,
            request_id: request_id.parse().map_err(|_| ClientError::Input)?,
            result: ipc::ResultBody::Status {
                status: Box::new(status(sample)),
            },
        };
        let mut frame = Vec::new();
        ipc::write(&mut frame, &response)
            .await
            .map_err(|_| ClientError::Frame)?;
        Ok(frame)
    }
}
