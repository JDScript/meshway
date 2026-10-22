use super::samples::{ClientSample, client_sample_frame};
use super::*;
const ID: &str = "00000000-0000-4000-8000-000000000001";
fn request(command: ClientCommand) -> Arc<ClientRequest> {
    ClientRequest::new(ID.into(), command).unwrap()
}

#[tokio::test]
async fn commands_use_the_protocol_codec_and_keep_secrets_out_of_debug() {
    let client = request(ClientCommand::Up {
        revision: 7,
        enrollment_key: Some("private-fixture".into()),
        reenroll: false,
    });
    let frame = client.frame().await.unwrap();
    let wire: ipc::Request = ipc::read(&mut frame.as_slice()).await.unwrap();
    assert_eq!(wire.version, ipc::VERSION);
    assert!(!format!("{wire:?}").contains("private-fixture"));
    assert!(
        matches!(wire.command, ipc::Command::Up { expected_revision, .. } if expected_revision.get() == 7)
    );
    assert!(
        ClientRequest::new(
            ID.into(),
            ClientCommand::Ping {
                target: "peer".into(),
                count: 0
            }
        )
        .is_err()
    );
}
#[tokio::test]
async fn typed_status_keeps_unmeasured_rtt_absent_and_actual_idle_route() {
    let frame = client_sample_frame(ClientSample::Running, ID.into())
        .await
        .unwrap();
    let ClientResult::Status { status } =
        request(ClientCommand::Status).decode(frame).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(status.phase, ClientPhase::Running);
    assert!(status.peers[1].rtt.is_none());
    assert_eq!(
        status.peers[2].idle_path.as_ref().unwrap().route,
        ClientIdleRoute::Direct
    );
}
#[tokio::test]
async fn closed_contract_rejects_unknown_missing_required_and_wrong_bindings() {
    let original = client_sample_frame(ClientSample::Unenrolled, ID.into())
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&original[4..]).unwrap();
    for case in 0..4 {
        let mut altered = value.clone();
        let status = altered["result"]["status"].as_object_mut().unwrap();
        match case {
            0 => {
                status.insert("futureField".into(), true.into());
            }
            1 => {
                status.remove("node");
            }
            2 => {
                status.insert("phase".into(), "futurePhase".into());
            }
            _ => {
                altered["version"] = 12.into();
            }
        }
        let mut frame = Vec::new();
        ipc::write(&mut frame, &altered).await.unwrap();
        assert!(request(ClientCommand::Status).decode(frame).await.is_err());
    }
    // Optional observation omitted is accepted; required nullable node is null.
    let mut altered = value;
    altered["result"]["status"]
        .as_object_mut()
        .unwrap()
        .remove("controlObservedAt");
    let mut frame = Vec::new();
    ipc::write(&mut frame, &altered).await.unwrap();
    let ClientResult::Status { status } =
        request(ClientCommand::Status).decode(frame).await.unwrap()
    else {
        panic!()
    };
    assert!(status.control_observed_at.is_none());
    assert!(status.node.is_none());
    let mut extra = original.clone();
    extra.push(0);
    assert!(request(ClientCommand::Status).decode(extra).await.is_err());
    assert!(
        request(ClientCommand::Status)
            .decode(original[..8].to_vec())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn ping_stream_consumes_one_frame_with_sequence_and_terminal_boundary() {
    let client = request(ClientCommand::Ping {
        target: "peer".into(),
        count: 1,
    });
    for (sequence, event) in [
        (
            0,
            ipc::PingEvent::Preparing {
                elapsed_ms: meshway_model::Sequence::MIN,
            },
        ),
        (
            1,
            ipc::PingEvent::Failed {
                code: ipc::PingError::NotRunning,
            },
        ),
    ] {
        let response = ipc::Response {
            version: ipc::VERSION,
            request_id: ID.parse().unwrap(),
            result: ipc::ResultBody::Ping {
                sequence: meshway_model::Sequence::new(sequence).unwrap(),
                event,
            },
        };
        let mut frame = Vec::new();
        ipc::write(&mut frame, &response).await.unwrap();
        assert!(matches!(
            client.decode(frame.clone()).await.unwrap(),
            ClientResult::Ping { .. }
        ));
        if sequence == 1 {
            assert!(matches!(
                client.decode(frame).await,
                Err(ClientError::Completed)
            ));
        }
    }
}

#[tokio::test]
async fn stream_rejects_skips_and_response_request_binding_changes() {
    let client = request(ClientCommand::Ping {
        target: "peer".into(),
        count: 1,
    });
    let response = ipc::Response {
        version: ipc::VERSION,
        request_id: ID.parse().unwrap(),
        result: ipc::ResultBody::Ping {
            sequence: meshway_model::Sequence::new(1).unwrap(),
            event: ipc::PingEvent::Preparing {
                elapsed_ms: meshway_model::Sequence::MIN,
            },
        },
    };
    let mut frame = Vec::new();
    ipc::write(&mut frame, &response).await.unwrap();
    assert!(matches!(
        client.decode(frame).await,
        Err(ClientError::Contract)
    ));
    let frame = client_sample_frame(
        ClientSample::Unenrolled,
        "00000000-0000-4000-8000-000000000002".into(),
    )
    .await
    .unwrap();
    assert!(matches!(
        request(ClientCommand::Status).decode(frame).await,
        Err(ClientError::Contract)
    ));
}

#[tokio::test]
async fn netcheck_keeps_unknown_mapping_and_unmeasured_latency_distinct() {
    let report = ipc::NetcheckReport {
        observed_at: chrono::Utc::now(),
        server: ipc::NetcheckServer {
            origin: "https://server.example/".into(),
            reachable: None,
            rtt_us: None,
        },
        unavailable: None,
        ipv4: false,
        ipv6: false,
        udp: false,
        sockets: vec![ipc::NetcheckSocket {
            resource: meshway_model::state::ResourceRef::Listener { key: "udp".into() },
            bound_address: "0.0.0.0:10000".parse().unwrap(),
            mappings: vec![],
            port_mapping: ipc::PortMapping::Unknown,
            endpoint_mapping: ipc::EndpointMapping::Unknown,
        }],
        observers: vec![],
    };
    let response = ipc::Response {
        version: ipc::VERSION,
        request_id: ID.parse().unwrap(),
        result: ipc::ResultBody::Netcheck {
            report: Box::new(report),
        },
    };
    let mut frame = Vec::new();
    ipc::write(&mut frame, &response).await.unwrap();
    let ClientResult::Netcheck { report } = request(ClientCommand::Netcheck)
        .decode(frame)
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(report.server_reachable.is_none());
    assert!(report.server_rtt_us.is_none());
    assert_eq!(
        report.sockets[0].endpoint_mapping,
        ClientEndpointMapping::Unknown
    );
}

#[tokio::test]
async fn every_preview_sample_decodes_through_the_production_client() {
    use super::samples::{ClientDiagnosticSample, client_sample_diagnostic_frames};
    for sample in [
        ClientSample::Unenrolled,
        ClientSample::AwaitingApproval,
        ClientSample::Running,
        ClientSample::Error,
        ClientSample::ConnectingAuthentication,
        ClientSample::ConnectingSync,
        ClientSample::ConnectingApply,
        ClientSample::ConnectingPlane,
        ClientSample::NoPeers,
        ClientSample::MeshOnly,
        ClientSample::ServerUnavailable,
        ClientSample::LeaseUnavailable,
    ] {
        let frame = client_sample_frame(sample, ID.into()).await.unwrap();
        let ClientResult::Status { status } =
            request(ClientCommand::Status).decode(frame).await.unwrap()
        else {
            panic!()
        };
        match sample {
            ClientSample::Running => {
                let states: Vec<_> = status.peers.iter().map(|peer| &peer.state).collect();
                assert!(states.len() >= 6);
                for state in [
                    ClientPeerState::Direct,
                    ClientPeerState::Relay,
                    ClientPeerState::Idle,
                    ClientPeerState::Offline,
                ] {
                    assert!(states.contains(&&state));
                }
            }
            ClientSample::NoPeers => assert!(status.peers.is_empty() && status.node.is_some()),
            ClientSample::MeshOnly => {
                let routing = status.netmap.unwrap().routing.unwrap();
                assert_eq!(routing.profiles.len(), 1);
                assert_eq!(routing.applied_profile, "mesh");
            }
            ClientSample::ServerUnavailable => {
                assert_eq!(status.sync, ClientSync::Reconnecting);
                assert!(status.netmap.is_some() && status.applied.is_some());
            }
            ClientSample::LeaseUnavailable => {
                assert!(status.netmap.is_none() && status.peers.is_empty());
            }
            _ => {}
        }
    }
    for sample in [
        ClientDiagnosticSample::Ping,
        ClientDiagnosticSample::PingExit,
        ClientDiagnosticSample::PingDenied,
        ClientDiagnosticSample::PingUnsupported,
    ] {
        let client = request(ClientCommand::Ping {
            target: "studio-nas".into(),
            count: 4,
        });
        let frames = client_sample_diagnostic_frames(sample, ID.into())
            .await
            .unwrap();
        let count = frames.len();
        let mut events = Vec::new();
        for frame in frames {
            let ClientResult::Ping { event, .. } = client.decode(frame).await.unwrap() else {
                panic!()
            };
            events.push(event);
        }
        assert!(matches!(events[0], ClientPingEvent::Decision { .. }));
        assert!(matches!(
            events[count - 1],
            ClientPingEvent::Completed { .. } | ClientPingEvent::Failed { .. }
        ));
    }
    let mut frames = client_sample_diagnostic_frames(ClientDiagnosticSample::Netcheck, ID.into())
        .await
        .unwrap();
    assert_eq!(frames.len(), 1);
    let ClientResult::Netcheck { report } = request(ClientCommand::Netcheck)
        .decode(frames.remove(0))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(!report.ipv6 && report.ipv4);
    assert_eq!(
        report.sockets[0].endpoint_mapping,
        ClientEndpointMapping::Dependent
    );
}
