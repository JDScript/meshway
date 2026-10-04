use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{
    http::Uri,
    protocol::{
        Role,
        frame::{
            Frame,
            coding::{Data, OpCode},
        },
    },
};

fn settings() -> Settings {
    Settings::new(
        &ListenerWebSocket {
            path: "/隧道/%2f".parse().unwrap(),
            host: Some("front.example".into()),
            response_headers: WebSocketHeaders::try_from_iter([(
                "X-Response".into(),
                "value".into(),
            )])
            .unwrap(),
        },
        Some(&DialerWebSocket {
            headers: WebSocketHeaders::try_from_iter([(
                "X-Request".into(),
                "credential\tvalue".into(),
            )])
            .unwrap(),
        }),
        "different-sni.example",
    )
    .unwrap()
}

#[tokio::test]
async fn utf8_and_percent_escapes_reach_wire_unchanged_and_host_is_not_sni() {
    let settings = settings();
    let uri: Uri = "wss://front.example/隧道/%2f".parse().unwrap();
    assert_eq!(uri.path(), "/隧道/%2f");
    let (local, remote) = tokio::io::duplex(4096);
    let (client, server) = tokio::join!(
        settings.connect(local),
        tokio_tungstenite::accept_hdr_async(remote, AssertRequest)
    );
    client.unwrap();
    server.unwrap();
}

struct AssertRequest;

impl Callback for AssertRequest {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        assert_eq!(request.uri().path(), "/隧道/%2f");
        assert_eq!(request.headers()["host"], "front.example");
        assert_eq!(request.headers()["x-request"], "credential\tvalue");
        Ok(response)
    }
}

#[tokio::test]
async fn listener_response_headers_and_exact_host_path_checks() {
    let settings = settings();
    for (path, host, expected) in [
        ("/隧道/%2f", "front.example", true),
        ("/隧道/%2F", "front.example", false),
        ("/隧道/%2f?ed=1", "front.example", false),
        ("/隧道/%2f", "different-sni.example", false),
    ] {
        let (local, remote) = tokio::io::duplex(4096);
        let request = format!("wss://{host}{path}").into_client_request().unwrap();
        let (client, server) = tokio::join!(
            tokio_tungstenite::client_async(request, local),
            settings.accept(remote)
        );
        if expected {
            assert_eq!(client.unwrap().1.headers()["x-response"], "value");
            assert!(server.is_ok());
        } else {
            assert!(client.is_err());
            assert!(matches!(server, Err(Error::WebSocket)));
        }
    }
}

#[tokio::test]
async fn rejects_malformed_upgrade_early_data_headers_and_duplicate_host() {
    let settings = settings();
    for extra in [
        "Host: front.example\r\n",
        "Sec-WebSocket-Protocol: ZWFybHk=\r\n",
        "Content-Length: 4\r\n",
        "Transfer-Encoding: chunked\r\n",
    ] {
        let (mut local, remote) = tokio::io::duplex(4096);
        let request = format!(
            "GET /隧道/%2f HTTP/1.1\r\nHost: front.example\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{extra}\r\n"
        );
        local.write_all(request.as_bytes()).await.unwrap();
        assert!(matches!(
            settings.accept(remote).await,
            Err(Error::WebSocket)
        ));
    }
    let (mut local, remote) = tokio::io::duplex(4096);
    local
        .write_all(b"GET / HTTP/1.1\r\nHost: front.example\r\n\r\n")
        .await
        .unwrap();
    assert!(matches!(
        settings.accept(remote).await,
        Err(Error::WebSocket)
    ));
}

#[tokio::test]
async fn oversized_unterminated_upgrade_is_bounded() {
    let settings = settings();
    let (mut local, remote) = tokio::io::duplex(MAX_UPGRADE * 2);
    let mut request = b"GET / HTTP/1.1\r\nX-Large: ".to_vec();
    request.extend(std::iter::repeat_n(b'x', MAX_UPGRADE));
    local.write_all(&request).await.unwrap();
    assert!(matches!(
        settings.accept(remote).await,
        Err(Error::WebSocket)
    ));
}

#[tokio::test]
async fn binary_fragments_combined_messages_ping_pong_and_close_are_transparent() {
    let (local, remote) = tokio::io::duplex(128);
    let mut bytes = ByteStream::new(
        WebSocketStream::from_raw_socket(local, Role::Client, Some(config())).await,
    );
    let mut messages = WebSocketStream::from_raw_socket(remote, Role::Server, Some(config())).await;
    tokio::join!(
        async {
            let mut received = [0; 8];
            for byte in &mut received {
                bytes.read_exact(std::slice::from_mut(byte)).await.unwrap();
            }
            assert_eq!(&received, b"abcdefgh");
            assert_eq!(bytes.read(&mut [0; 1]).await.unwrap(), 0);
        },
        async {
            messages
                .send(Message::Frame(Frame::message(
                    Bytes::from_static(b"ab"),
                    OpCode::Data(Data::Binary),
                    false,
                )))
                .await
                .unwrap();
            messages
                .send(Message::Ping(Bytes::from_static(b"control")))
                .await
                .unwrap();
            messages
                .send(Message::Frame(Frame::message(
                    Bytes::from_static(b"cd"),
                    OpCode::Data(Data::Continue),
                    true,
                )))
                .await
                .unwrap();
            messages
                .send(Message::Binary(Bytes::from_static(b"efgh")))
                .await
                .unwrap();
            assert_eq!(
                messages.next().await.unwrap().unwrap(),
                Message::Pong(Bytes::from_static(b"control"))
            );
            messages.close(None).await.unwrap();
            assert!(matches!(
                messages.next().await.unwrap().unwrap(),
                Message::Close(_)
            ));
        }
    );
}

#[tokio::test]
async fn writes_are_partial_bounded_binary_messages_and_flush_drains_output() {
    let (local, remote) = tokio::io::duplex(64);
    let mut bytes = ByteStream::new(
        WebSocketStream::from_raw_socket(local, Role::Client, Some(config())).await,
    );
    let mut messages = WebSocketStream::from_raw_socket(remote, Role::Server, Some(config())).await;
    let payload = vec![0xa5; CHUNK * 2 + 1];
    let mut result = Vec::new();
    tokio::join!(
        async {
            let accepted = bytes.write(&payload).await.unwrap();
            assert_eq!(accepted, CHUNK);
            let mut pending = Box::pin(bytes.write(&payload[accepted..]));
            assert!(futures_util::poll!(&mut pending).is_pending());
            drop(pending);
            bytes.write_all(&payload[accepted..]).await.unwrap();
            bytes.flush().await.unwrap();
        },
        async {
            while result.len() < payload.len() {
                let Message::Binary(part) = messages.next().await.unwrap().unwrap() else {
                    panic!("expected binary")
                };
                assert!(part.len() <= CHUNK);
                result.extend_from_slice(&part);
            }
        }
    );
    assert_eq!(result, payload);
}

#[tokio::test]
async fn text_and_oversized_messages_are_errors_not_payload() {
    for message in [
        Message::Text("not binary".into()),
        Message::Binary(vec![0; MAX_MESSAGE + 1].into()),
    ] {
        let (local, remote) = tokio::io::duplex(64);
        let mut bytes = ByteStream::new(
            WebSocketStream::from_raw_socket(local, Role::Client, Some(config())).await,
        );
        let mut messages = WebSocketStream::from_raw_socket(remote, Role::Server, None).await;
        tokio::join!(
            async {
                let mut output = [0xaa; 8];
                assert!(bytes.read(&mut output).await.is_err());
                assert_eq!(output, [0xaa; 8]);
                drop(bytes);
            },
            async {
                let _ = messages.send(message).await;
            }
        );
    }
}

#[test]
fn constructor_checks_header_and_path_bounds_and_library_text_constraints() {
    let ws = ListenerWebSocket {
        path: format!("/{}", "x".repeat(MAX_PATH)).parse().unwrap(),
        ..ListenerWebSocket::default()
    };
    assert!(matches!(
        Settings::new(&ws, None, "localhost"),
        Err(Error::Configuration)
    ));
    let mut ws = ListenerWebSocket {
        response_headers: WebSocketHeaders::try_from_iter([(
            "X-Large".into(),
            "x".repeat(MAX_HEADERS),
        )])
        .unwrap(),
        ..ListenerWebSocket::default()
    };
    assert!(matches!(
        Settings::new(&ws, None, "localhost"),
        Err(Error::Configuration)
    ));
    ws.response_headers =
        WebSocketHeaders::try_from_iter([("X-Unicode".into(), "隧道".into())]).unwrap();
    assert!(matches!(
        Settings::new(&ws, None, "localhost"),
        Err(Error::Configuration)
    ));
}

#[tokio::test]
async fn split_reader_flush_cannot_steal_blocked_writer_wakeup() {
    use std::{future::poll_fn, time::Duration};
    use tokio::sync::oneshot;

    let (local, remote) = tokio::io::duplex(8);
    let bytes = ByteStream::new(
        WebSocketStream::from_raw_socket(local, Role::Client, Some(config())).await,
    );
    let (mut reader, mut writer) = tokio::io::split(bytes);
    let mut remote = WebSocketStream::from_raw_socket(remote, Role::Server, Some(config())).await;
    let (writer_pending, writer_waiting) = oneshot::channel();
    let mut writing = tokio::spawn(async move {
        let payload = vec![0xa5; CHUNK * 2];
        {
            let write = writer.write_all(&payload);
            tokio::pin!(write);
            let mut signal = Some(writer_pending);
            poll_fn(|cx| {
                let result = write.as_mut().poll(cx);
                if result.is_pending()
                    && let Some(signal) = signal.take()
                {
                    let _ = signal.send(());
                }
                result
            })
            .await?;
        }
        writer.flush().await
    });
    writer_waiting.await.unwrap();

    // This is a different Tokio task/waker, and is polled after the writer
    // actually blocks. Its read-driven flush used to overwrite the writer slot.
    let (reader_pending, reader_waiting) = oneshot::channel();
    let reading = tokio::spawn(async move {
        let mut output = [0; 1];
        let mut signal = Some(reader_pending);
        poll_fn(|cx| {
            let mut buffer = ReadBuf::new(&mut output);
            let result = Pin::new(&mut reader).poll_read(cx, &mut buffer);
            if result.is_pending()
                && let Some(signal) = signal.take()
            {
                let _ = signal.send(());
            }
            result
        })
        .await
    });
    reader_waiting.await.unwrap();

    let (release, hold) = oneshot::channel();
    let mut draining = tokio::spawn(async move {
        let mut received = 0;
        while received < CHUNK * 2 {
            let Message::Binary(bytes) = remote.next().await.unwrap().unwrap() else {
                panic!("expected only binary payload");
            };
            assert!(bytes.iter().all(|byte| *byte == 0xa5));
            received += bytes.len();
        }
        // Keep the peer open without sending *any* app data/control response.
        let _ = hold.await;
        received
    });
    let result = tokio::time::timeout(Duration::from_secs(3), &mut writing).await;
    let _ = release.send(());
    reading.abort();
    let _ = reading.await;
    if result.is_err() {
        writing.abort();
        let _ = writing.await;
        draining.abort();
        let _ = draining.await;
        panic!("peer drainage did not wake the independent writer");
    }
    result.unwrap().unwrap().unwrap();
    let received = tokio::time::timeout(Duration::from_secs(3), &mut draining).await;
    if received.is_err() {
        draining.abort();
        let _ = draining.await;
    }
    assert_eq!(received.unwrap().unwrap(), CHUNK * 2);
}

#[tokio::test]
async fn server_close_ack_flushes_buffered_carrier_before_eof_without_drop() {
    use std::time::Duration;
    use tokio::{io::BufStream, sync::oneshot};

    // The two-byte server close acknowledgement cannot fit in the carrier.
    // BufStream first accepts it without writing, then flush must make progress
    // over multiple polls after tungstenite has already terminated its state.
    let (local, remote) = tokio::io::duplex(1);
    let buffered = BufStream::with_capacity(32, 32, local);
    let mut server = ByteStream::new(
        WebSocketStream::from_raw_socket(buffered, Role::Server, Some(config())).await,
    );
    let mut client = WebSocketStream::from_raw_socket(remote, Role::Client, Some(config())).await;
    let (eof, got_eof) = oneshot::channel();
    let (release, hold) = oneshot::channel();
    let mut accepting_close = tokio::spawn(async move {
        let read = server.read(&mut [0; 1]).await;
        let _ = eof.send(read);
        let _ = hold.await;
        // Repeated flush on the completed close must not re-poll closed WS.
        server.flush().await
    });
    let mut closing = tokio::spawn(async move {
        client.close(None).await.unwrap();
        let reply = client.next().await;
        (client, reply)
    });
    let reply = tokio::time::timeout(Duration::from_secs(3), &mut closing).await;
    if reply.is_err() {
        closing.abort();
        accepting_close.abort();
        let _ = closing.await;
        let _ = accepting_close.await;
        panic!("close acknowledgement remained in the server carrier buffer");
    }
    let (_client, reply) = reply.unwrap().unwrap();
    assert!(matches!(reply, Some(Ok(Message::Close(_)))));
    assert_eq!(got_eof.await.unwrap().unwrap(), 0);
    assert!(!accepting_close.is_finished()); // still owns the open server stream
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), &mut accepting_close)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
