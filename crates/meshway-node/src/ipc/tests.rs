use super::*;
use meshway_model::RequestId;
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

const ID: &str = "00000000-0000-4000-8000-000000000001";

fn request() -> Request {
    Request {
        version: VERSION,
        request_id: ID.parse::<RequestId>().unwrap(),
        command: Command::Ping {
            target: meshway_protocol::ipc::PingTarget::Name {
                name: "peer.mesh".into(),
            },
            count: 10,
        },
    }
}

fn accepted() -> PingEvent {
    serde_json::from_value(json!({
        "type":"accepted","subscriptionId":ID,"observationId":ID,
        "target":{"nodeId":ID,"hostname":"peer.mesh","addresses":{"ipv4":"10.0.0.2","ipv6":null}}
    }))
    .unwrap()
}

async fn bridge() -> (
    UnixStream,
    PingRequest,
    watch::Sender<bool>,
    tokio::task::JoinHandle<()>,
) {
    let (client, server) = UnixStream::pair().unwrap();
    let (pings, mut requests) = mpsc::channel(1);
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(handle_ping(server, request(), pings, stopped));
    let ping = requests.recv().await.unwrap();
    assert_eq!(ping.id, request().request_id);
    assert_eq!(ping.count, 10);
    assert_eq!(ping.events.max_capacity(), 16);
    (client, ping, stop, task)
}

#[tokio::test]
async fn bridge_frames_use_one_request_and_contiguous_sequences_then_cancel() {
    let (mut client, ping, _stop, task) = bridge().await;
    for (index, event) in [
        accepted(),
        PingEvent::Preparing {
            elapsed_ms: Sequence::new(42).unwrap(),
        },
        PingEvent::Completed {
            sent: 10,
            success: 9,
            timeout: 1,
            failed: 0,
        },
    ]
    .into_iter()
    .enumerate()
    {
        ping.events.send(event.clone()).await.unwrap();
        let response: Response = meshway_protocol::ipc::read(&mut client).await.unwrap();
        assert_eq!(response.request_id, ping.id);
        assert_eq!(response.version, VERSION);
        let ResultBody::Ping {
            sequence,
            event: actual,
        } = response.result
        else {
            panic!("expected ping");
        };
        assert_eq!(sequence.get(), index as u64);
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(event).unwrap()
        );
    }
    task.await.unwrap();
    assert!(ping.cancellation.is_cancelled());
    assert!(ping.events.is_closed());
    assert_eq!(client.read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test]
async fn eof_extra_input_stop_and_aborted_handler_cancel_subscriber() {
    for mode in 0..4 {
        let (mut client, ping, stop, task) = bridge().await;
        match mode {
            0 => drop(client),
            1 => {
                client.write_all(b"unexpected request").await.unwrap();
            }
            2 => {
                stop.send_replace(true);
            }
            _ => task.abort(),
        }
        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap();
        if mode == 3 {
            assert!(result.unwrap_err().is_cancelled());
        } else {
            result.unwrap();
        }
        assert!(ping.cancellation.is_cancelled());
        assert!(ping.events.is_closed());
    }
}

#[tokio::test]
async fn full_queue_rejects_without_waiting_and_closed_owner_is_terminal() {
    for busy in [true, false] {
        let (mut client, server) = UnixStream::pair().unwrap();
        let (pings, requests) = mpsc::channel(1);
        let (events, _received) = mpsc::channel(1);
        if busy {
            assert!(
                pings
                    .try_send(PingRequest {
                        id: request().request_id,
                        target: meshway_protocol::ipc::PingTarget::Name {
                            name: "occupied".into()
                        },
                        count: 1,
                        events,
                        cancellation: CancellationToken::new()
                    })
                    .is_ok()
            );
        } else {
            drop(requests);
        }
        let (_stop, stopped) = watch::channel(false);
        let task = tokio::spawn(handle_ping(server, request(), pings, stopped));
        let response: Response = meshway_protocol::ipc::read(&mut client).await.unwrap();
        let ResultBody::Ping {
            sequence,
            event: PingEvent::Failed { code },
        } = response.result
        else {
            panic!("expected failure");
        };
        assert_eq!(sequence, Sequence::MIN);
        assert_eq!(
            code,
            if busy {
                PingError::Busy
            } else {
                PingError::Stopped
            }
        );
        task.await.unwrap();
    }
}

#[tokio::test]
async fn slow_consumer_has_bounded_queue_write_deadline_and_cancellation() {
    // C: real WRITE_TIMEOUT on the blocked Unix stream is the behavior under
    // test; producer completion alone would not prove the write deadline.
    let (_client, ping, _stop, task) = bridge().await;
    let mut event = accepted();
    if let PingEvent::Accepted { target, .. } = &mut event {
        target.hostname = "x".repeat(32 * 1024);
    }
    let events = ping.events.clone();
    let producer = tokio::spawn(async move {
        for _ in 0..1024 {
            if events.send(event.clone()).await.is_err() {
                return;
            }
        }
        panic!("unread connection must not absorb unbounded events");
    });
    tokio::time::timeout(WRITE_TIMEOUT + Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert!(ping.cancellation.is_cancelled());
    assert!(ping.events.is_closed());
    producer.await.unwrap();
}

#[tokio::test]
async fn lost_owner_sends_terminal_stopped() {
    let (mut client, ping, _stop, task) = bridge().await;
    let cancellation = ping.cancellation.clone();
    drop(ping);
    let response: Response = meshway_protocol::ipc::read(&mut client).await.unwrap();
    assert!(matches!(
        response.result,
        ResultBody::Ping {
            event: PingEvent::Failed {
                code: PingError::Stopped
            },
            ..
        }
    ));
    task.await.unwrap();
    assert!(cancellation.is_cancelled());
}
