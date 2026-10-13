use super::*;
use crate::ipc::PingError;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use tokio::{io::AsyncReadExt, net::UnixListener};

const ID: &str = "00000000-0000-4000-8000-000000000001";

fn accepted() -> PingEvent {
    serde_json::from_value(json!({
        "type":"accepted", "subscriptionId":ID, "observationId":ID,
        "target":{"nodeId":ID,"hostname":"peer.mesh","addresses":{"ipv4":"10.0.0.2","ipv6":null}}
    }))
    .unwrap()
}

fn target() -> PingTarget {
    PingTarget::Name {
        name: "peer.mesh".into(),
    }
}

fn sample() -> PingEvent {
    serde_json::from_value(json!({
        "type":"sample","observationId":ID,
        "sample":{"measurementId":ID,"version":1,"windowStart":"2026-10-05T00:00:00Z",
        "windowEnd":"2026-10-05T00:00:01Z","partial":false,"probe":null,"traffic":null,"probeCounts":null}
    })).unwrap()
}

fn listener() -> (tempfile::TempDir, UnixListener, Client) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control.sock");
    let listener = UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    (directory, listener, Client::new(path))
}

async fn send(stream: &mut UnixStream, request: &Request, sequence: u64, event: PingEvent) {
    crate::ipc::write(
        stream,
        &Response {
            version: VERSION,
            request_id: request.request_id,
            result: ResultBody::Ping {
                sequence: Sequence::new(sequence).unwrap(),
                event,
            },
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn preparing_can_cover_server_admission_before_the_single_acceptance() {
    let (_directory, listener, client) = listener();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = crate::ipc::read(&mut stream).await.unwrap();
        send(
            &mut stream,
            &request,
            0,
            PingEvent::Preparing {
                elapsed_ms: Sequence::MIN,
            },
        )
        .await;
        send(&mut stream, &request, 1, accepted()).await;
        send(
            &mut stream,
            &request,
            2,
            PingEvent::Completed {
                sent: 0,
                success: 0,
                timeout: 0,
                failed: 1,
            },
        )
        .await;
    });
    let mut ping = client.ping(target(), 1).await.unwrap();
    assert!(matches!(
        ping.next().await.unwrap(),
        Some(PingEvent::Preparing { .. })
    ));
    assert!(matches!(
        ping.next().await.unwrap(),
        Some(PingEvent::Accepted { .. })
    ));
    assert!(matches!(
        ping.next().await.unwrap(),
        Some(PingEvent::Completed { .. })
    ));
    assert!(ping.next().await.unwrap().is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn multiple_frames_stop_at_terminal_and_dropping_cancels() {
    let (_directory, listener, client) = listener();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request: Request = crate::ipc::read(&mut stream).await.unwrap();
        assert!(matches!(request.command, Command::Ping { count: 10, .. }));
        send(&mut stream, &request, 0, accepted()).await;
        send(
            &mut stream,
            &request,
            1,
            PingEvent::Preparing {
                elapsed_ms: Sequence::new(1250).unwrap(),
            },
        )
        .await;
        send(&mut stream, &request, 2, sample()).await;
        send(
            &mut stream,
            &request,
            3,
            PingEvent::Completed {
                sent: 10,
                success: 8,
                timeout: 1,
                failed: 1,
            },
        )
        .await;
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).await.unwrap(), 0);
    });
    let mut ping = client.ping(target(), 10).await.unwrap();
    assert!(matches!(
        ping.next().await.unwrap(),
        Some(PingEvent::Accepted { .. })
    ));
    assert!(matches!(
        ping.next().await.unwrap(),
        Some(PingEvent::Preparing { .. })
    ));
    assert!(matches!(
        ping.next().await.unwrap(),
        Some(PingEvent::Sample { .. })
    ));
    assert!(matches!(
        ping.next().await.unwrap(),
        Some(PingEvent::Completed { .. })
    ));
    assert!(ping.next().await.unwrap().is_none());
    server.await.unwrap();

    let (_directory, listener, client) = self::listener();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = crate::ipc::read(&mut stream).await.unwrap();
        send(&mut stream, &request, 0, accepted()).await;
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).await.unwrap(), 0);
    });
    drop(client.ping(target(), 1).await.unwrap());
    server.await.unwrap();
}

#[tokio::test]
async fn rejects_version_request_sequence_and_illegal_event_shapes() {
    for corruption in 0..8 {
        let (_directory, listener, client) = listener();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request: Request = crate::ipc::read(&mut stream).await.unwrap();
            let mut value = serde_json::to_value(Response {
                version: VERSION,
                request_id: request.request_id,
                result: ResultBody::Ping {
                    sequence: Sequence::MIN,
                    event: accepted(),
                },
            })
            .unwrap();
            match corruption {
                0 => value["version"] = json!(VERSION + 1),
                1 => value["requestId"] = json!(ID),
                2 => value["result"]["sequence"] = json!(1),
                3 => value["result"]["event"]["unexpected"] = json!(true),
                4 => value["result"]["event"]["target"]["addresses"] = json!({"ipv4":null}),
                5 => value["result"]["event"] = serde_json::to_value(sample()).unwrap(),
                6 => {
                    value["result"]["event"] =
                        json!({"type":"completed","sent":0,"success":0,"timeout":0,"failed":0})
                }
                _ => value["result"]["event"] = json!({"type":"failed","code":{"timeout":null}}),
            }
            crate::ipc::write(&mut stream, &value).await.unwrap();
        });
        assert!(
            client.ping(target(), 1).await.is_err(),
            "corruption {corruption}"
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn sequence_must_be_contiguous_and_accepted_cannot_repeat() {
    let mut wrong_observation = sample();
    if let PingEvent::Sample { observation_id, .. } = &mut wrong_observation {
        *observation_id = "00000000-0000-4000-8000-000000000002".parse().unwrap();
    }
    for (sequence, event) in [
        (
            0,
            PingEvent::Preparing {
                elapsed_ms: Sequence::MIN,
            },
        ),
        (
            2,
            PingEvent::Failed {
                code: PingError::Timeout,
            },
        ),
        (1, accepted()),
        (1, wrong_observation),
    ] {
        let (_directory, listener, client) = listener();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = crate::ipc::read(&mut stream).await.unwrap();
            send(&mut stream, &request, 0, accepted()).await;
            send(&mut stream, &request, sequence, event).await;
        });
        let mut ping = client.ping(target(), 1).await.unwrap();
        ping.next().await.unwrap();
        assert!(matches!(ping.next().await, Err(ClientError::Response)));
        assert!(ping.next().await.unwrap().is_none());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn eof_is_not_completion_and_startup_errors_are_preserved() {
    for startup_error in [false, true] {
        let (_directory, listener, client) = listener();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request: Request = crate::ipc::read(&mut stream).await.unwrap();
            if startup_error {
                crate::ipc::write(
                    &mut stream,
                    &Response {
                        version: VERSION,
                        request_id: request.request_id,
                        result: ResultBody::Error {
                            code: ErrorCode::Version,
                        },
                    },
                )
                .await
                .unwrap();
            } else {
                send(&mut stream, &request, 0, accepted()).await;
            }
        });
        if startup_error {
            assert!(matches!(
                client.ping(target(), 1).await,
                Err(ClientError::Remote(ErrorCode::Version))
            ));
        } else {
            let mut ping = client.ping(target(), 1).await.unwrap();
            ping.next().await.unwrap();
            assert!(matches!(ping.next().await, Err(ClientError::Read(_))));
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn one_shot_rejects_ping_and_socket_must_be_private() {
    let (directory, listener, client) = listener();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = crate::ipc::read(&mut stream).await.unwrap();
        send(
            &mut stream,
            &request,
            0,
            PingEvent::Failed {
                code: PingError::NotRunning,
            },
        )
        .await;
    });
    assert!(matches!(
        client.call(Command::Status).await,
        Err(ClientError::Response)
    ));
    server.await.unwrap();
    let path = directory.path().join("public.sock");
    let _listener = UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(matches!(
        Client::new(path).ping(target(), 1).await,
        Err(ClientError::Identity)
    ));
}

#[tokio::test]
async fn stream_has_per_frame_idle_limit_instead_of_overall_request_deadline() {
    // C: cross REQUEST_TIMEOUT (12 s), remaining inside the streaming idle
    // budget. A shorter wait cannot distinguish one-shot and streaming reads.
    let (_directory, listener, client) = listener();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = crate::ipc::read(&mut stream).await.unwrap();
        send(&mut stream, &request, 0, accepted()).await;
        // This is an intentional quiet interval, with no setup inside it.
        // Progress heartbeats would hide a regression using REQUEST_TIMEOUT
        // for each frame. Cross that bound while staying inside PING_IDLE_TIMEOUT.
        let silence = REQUEST_TIMEOUT + (PING_IDLE_TIMEOUT - REQUEST_TIMEOUT) / 3;
        tokio::time::sleep(silence).await;
        send(
            &mut stream,
            &request,
            1,
            PingEvent::Completed {
                sent: 1,
                success: 0,
                timeout: 1,
                failed: 0,
            },
        )
        .await;
    });
    let mut ping = client.ping(target(), 1).await.unwrap();
    ping.next().await.unwrap();
    assert!(matches!(
        ping.next().await.unwrap(),
        Some(PingEvent::Completed { .. })
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn cancelling_next_closes_incomplete_frame_without_retry() {
    use tokio::io::AsyncWriteExt;
    let (_directory, listener, client) = listener();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = crate::ipc::read(&mut stream).await.unwrap();
        send(&mut stream, &request, 0, accepted()).await;
        stream.write_u32(100).await.unwrap();
        stream.write_all(b"{").await.unwrap();
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).await.unwrap(), 0);
    });
    let mut ping = client.ping(target(), 1).await.unwrap();
    ping.next().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), ping.next())
            .await
            .is_err()
    );
    assert!(ping.next().await.unwrap().is_none());
    server.await.unwrap();
}

fn report() -> NetcheckReport {
    serde_json::from_value(json!({
        "observedAt":"2026-10-11T00:00:00Z",
        "server":{"origin":"https://server.example/","reachable":true,"rttUs":1200},
        "unavailable":null,"ipv4":true,"ipv6":false,"udp":true,
        "sockets":[{"resource":{"kind":"listener","key":"udp"},"boundAddress":"0.0.0.0:35000",
            "mappings":[{"observer":"192.0.2.10:3478","mapped":"198.51.100.4:35000","rttUs":800,"fresh":true}],
            "portMapping":"preserved","endpointMapping":"unknown"}],
        "observers":[{"observer":"192.0.2.10:3478","rttUs":800,
            "sizeProbe":{"size":1500,"answered":false},"pathMtu":1500}]
    }))
    .unwrap()
}

#[tokio::test]
async fn netcheck_is_one_bounded_request_with_one_typed_report() {
    let (_directory, listener, client) = listener();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request: Request = crate::ipc::read(&mut stream).await.unwrap();
        assert!(matches!(request.command, Command::Netcheck));
        crate::ipc::write(
            &mut stream,
            &Response {
                version: VERSION,
                request_id: request.request_id,
                result: ResultBody::Netcheck {
                    report: Box::new(report()),
                },
            },
        )
        .await
        .unwrap();
        let (mut stream, _) = listener.accept().await.unwrap();
        let request: Request = crate::ipc::read(&mut stream).await.unwrap();
        crate::ipc::write(
            &mut stream,
            &Response {
                version: VERSION,
                request_id: request.request_id,
                result: ResultBody::Ping {
                    sequence: Sequence::MIN,
                    event: accepted(),
                },
            },
        )
        .await
        .unwrap();
    });
    assert_eq!(client.netcheck().await.unwrap(), report());
    assert!(matches!(
        client.netcheck().await,
        Err(ClientError::Response)
    ));
    server.await.unwrap();
}

fn decision(action: serde_json::Value) -> PingEvent {
    serde_json::from_value(json!({
        "type":"decision","decision":{"destination":"11.77.0.3","node":null,"resolution":null,
        "profile":"exit","rule":null,"action":action,"path":[ID]}
    }))
    .unwrap()
}

fn echo() -> PingEvent {
    serde_json::from_value(json!({
        "type":"echo","echo":{"sequence":0,"outcome":"reply","from":"11.77.0.3","error":null,
        "durationUs":1200,"setup":false,"path":[ID]}
    }))
    .unwrap()
}

#[tokio::test]
async fn an_external_decision_is_followed_by_echoes_never_by_a_mesh_sample() {
    let (_directory, listener, client) = listener();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = crate::ipc::read(&mut stream).await.unwrap();
        send(
            &mut stream,
            &request,
            0,
            decision(json!({"type":"exit","node":ID})),
        )
        .await;
        send(&mut stream, &request, 1, echo()).await;
        send(
            &mut stream,
            &request,
            2,
            PingEvent::Completed {
                sent: 1,
                success: 1,
                timeout: 0,
                failed: 0,
            },
        )
        .await;
    });
    let mut ping = client.ping(target(), 1).await.unwrap();
    assert!(matches!(
        ping.next().await.unwrap(),
        Some(PingEvent::Decision { .. })
    ));
    assert!(matches!(
        ping.next().await.unwrap(),
        Some(PingEvent::Echo { .. })
    ));
    assert!(matches!(
        ping.next().await.unwrap(),
        Some(PingEvent::Completed { .. })
    ));
    server.await.unwrap();

    // An external decision admits no subscription; a denied one no echo.
    for (action, next) in [
        (json!({"type":"direct"}), accepted()),
        (json!({"type":"denied","reason":"unauthorized"}), echo()),
    ] {
        let (_directory, listener, client) = self::listener();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = crate::ipc::read(&mut stream).await.unwrap();
            send(&mut stream, &request, 0, decision(action)).await;
            send(&mut stream, &request, 1, next).await;
        });
        let mut ping = client.ping(target(), 1).await.unwrap();
        assert!(matches!(
            ping.next().await.unwrap(),
            Some(PingEvent::Decision { .. })
        ));
        assert!(ping.next().await.is_err());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn preparing_may_precede_the_decision_but_a_second_decision_is_rejected() {
    let (_directory, listener, client) = listener();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = crate::ipc::read(&mut stream).await.unwrap();
        send(
            &mut stream,
            &request,
            0,
            PingEvent::Preparing {
                elapsed_ms: Sequence::MIN,
            },
        )
        .await;
        send(
            &mut stream,
            &request,
            1,
            decision(json!({"type":"exit","node":ID})),
        )
        .await;
        send(
            &mut stream,
            &request,
            2,
            decision(json!({"type":"exit","node":ID})),
        )
        .await;
    });
    let mut ping = client.ping(target(), 1).await.unwrap();
    assert!(matches!(
        ping.next().await.unwrap(),
        Some(PingEvent::Preparing { .. })
    ));
    assert!(matches!(
        ping.next().await.unwrap(),
        Some(PingEvent::Decision { .. })
    ));
    assert!(ping.next().await.is_err());
    server.await.unwrap();
}
