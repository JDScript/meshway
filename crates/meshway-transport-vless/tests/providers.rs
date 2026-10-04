use std::{collections::VecDeque, time::Duration};

use meshway_transport::{Frame, Limits, SendContext, SendOutcome, Session};
use meshway_transport_spec::{
    ClientTls, DialParameters, DialerWebSocket, Endpoint, Listener, ListenerTls, ListenerWebSocket,
    Port, PreparedListener, SecretString, SocketOptions, TcpOptions, TlsMode, TlsVersion,
    VlessDialer, VlessListener, VlessStream, WebSocketHeaders,
};
use meshway_transport_vless::{Acceptor, Connector, Error};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const UUID: &str = "00112233-4455-6677-8899-aabbccddeeff";

fn fixture(stream: VlessStream) -> (DialParameters, PreparedListener, ListenerTls) {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let tls = ListenerTls {
        mode: TlsMode::Provided,
        server_name: "localhost".into(),
        certificate: Some(SecretString::new(certified.cert.pem())),
        private_key: Some(SecretString::new(certified.signing_key.serialize_pem())),
        min_version: TlsVersion::Tls12,
        max_version: TlsVersion::Tls13,
        alpn: vec![],
        handshake_timeout: "2s".parse().unwrap(),
    };
    let websocket = (stream == VlessStream::WebSocket).then(|| ListenerWebSocket {
        path: "/隧道/%2f".parse().unwrap(),
        host: Some("front.example".into()),
        response_headers: WebSocketHeaders::try_from_iter([("X-Response".into(), "value".into())])
            .unwrap(),
    });
    let listener = PreparedListener::prepare(Listener::Vless(VlessListener {
        enabled: true,
        remark: String::new(),
        listen: "127.0.0.1".parse().unwrap(),
        listen_port: Port::new(443).unwrap(),
        endpoints: vec![],
        bind_interface: String::new(),
        socket: SocketOptions::default(),
        tcp: TcpOptions::default(),
        stream,
        uuid: Some(SecretString::new(UUID.into())),
        flow: String::new(),
        tls: tls.clone(),
        websocket: websocket.clone(),
    }))
    .unwrap();
    let mut options = VlessDialer::default();
    if stream == VlessStream::WebSocket {
        options.websocket = Some(DialerWebSocket {
            headers: WebSocketHeaders::try_from_iter([("X-Request".into(), "secret".into())])
                .unwrap(),
        });
    }
    let parameters = DialParameters::Vless {
        tls: ClientTls {
            server_name: "localhost".into(),
            certificate_chain: tls.certificate.clone().unwrap(),
            options: options.tls.clone(),
        },
        options,
        stream,
        flow: String::new(),
        uuid: SecretString::new(UUID.into()),
        websocket,
    };
    (parameters, listener, tls)
}

fn target(host: &str) -> Endpoint {
    Endpoint {
        host: host.into(),
        port: Port::new(12345).unwrap(),
    }
}

#[tokio::test]
async fn real_tls_tcp_and_websocket_carry_bidirectional_raw_bytes() {
    for stream in [VlessStream::Tcp, VlessStream::WebSocket] {
        let (parameters, listener, _) = fixture(stream);
        let connector = Connector::new(&parameters).unwrap();
        let acceptor = Acceptor::new(&listener).unwrap();
        for host in ["192.0.2.1", "2001:db8::1", "mesh.example"] {
            let destination = target(host);
            let (client, server) = tokio::io::duplex(256);
            let (client, server) = tokio::join!(
                connector.connect(client, &destination),
                acceptor.accept(server)
            );
            let mut client = client.unwrap();
            let mut server = server.unwrap();
            assert_eq!(server.destination, destination);
            let payload = vec![0xa5; 40_000];
            let mut received = vec![0; payload.len()];
            let (sent, read) = tokio::join!(
                async {
                    client.write_all(&payload).await.unwrap();
                    client.flush().await
                },
                server.stream.read_exact(&mut received)
            );
            sent.unwrap();
            read.unwrap();
            assert_eq!(received, payload);
            let (sent, read) = tokio::join!(
                async {
                    server.stream.write_all(b"reply").await.unwrap();
                    server.stream.flush().await
                },
                client.read_exact(&mut received[..5])
            );
            sent.unwrap();
            read.unwrap();
            assert_eq!(&received[..5], b"reply");
        }
    }
}

#[tokio::test]
async fn loopback_tls_session_uses_the_same_provider_and_core_framing() {
    for stream in [VlessStream::Tcp, VlessStream::WebSocket] {
        let (parameters, prepared, _) = fixture(stream);
        let connector = Connector::new(&parameters).unwrap();
        let acceptor = Acceptor::new(&prepared).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let destination = target("service.example");
        let (client, server) = tokio::join!(
            async {
                connector
                    .connect(
                        tokio::net::TcpStream::connect(address).await.unwrap(),
                        &destination,
                    )
                    .await
                    .unwrap()
            },
            async {
                acceptor
                    .accept(listener.accept().await.unwrap().0)
                    .await
                    .unwrap()
            }
        );
        assert_eq!(server.destination, destination);
        let limits = Limits::new(4096, 8, 32768, 4, 8).unwrap();
        let mut client = Session::stream(client, address, limits);
        let mut server = Session::stream(server.stream, address, limits);
        let mut frames = VecDeque::from([
            Frame::new(b"one".to_vec(), None),
            Frame::new(vec![7; 4096], None),
        ]);
        let admission = client
            .sender()
            .try_send(&mut frames, SendContext::default());
        assert_eq!(admission.accepted, 2);
        assert_eq!(server.receiver().recv().await.unwrap().payload, b"one");
        assert_eq!(
            server.receiver().recv().await.unwrap().payload,
            vec![7; 4096]
        );
        assert!(
            admission
                .completion
                .unwrap()
                .wait()
                .await
                .unwrap()
                .frames
                .iter()
                .all(|frame| matches!(frame.outcome, SendOutcome::LocalComplete))
        );
        client.shutdown().await.unwrap();
        server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn client_does_not_wait_for_delayed_upstream_response_header() {
    let (parameters, _, tls) = fixture(VlessStream::Tcp);
    let connector = Connector::new(&parameters).unwrap();
    let upstream = meshway_tls::Server::new(&tls).unwrap();
    let (client, server) = tokio::io::duplex(128);
    let destination = target("192.0.2.1");
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(
            async {
                let mut stream = connector.connect(client, &destination).await.unwrap();
                stream.write_all(b"first").await.unwrap();
                stream.flush().await.unwrap();
                let mut reply = [0; 5];
                stream.read_exact(&mut reply).await.unwrap();
                assert_eq!(&reply, b"reply");
            },
            async {
                let mut stream = upstream.accept(server).await.unwrap();
                let mut header = [0; 26];
                stream.read_exact(&mut header).await.unwrap();
                assert_eq!(&header[17..], &[0, 1, 0x30, 0x39, 1, 192, 0, 2, 1]);
                let mut first = [0; 5];
                stream.read_exact(&mut first).await.unwrap();
                assert_eq!(&first, b"first");
                stream.write_all(&[0]).await.unwrap();
                stream.flush().await.unwrap();
                stream.write_all(b"\x00reply").await.unwrap();
                stream.flush().await.unwrap();
            }
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn wrong_uuid_is_rejected_after_tls_and_errors_are_redacted() {
    let (mut parameters, listener, _) = fixture(VlessStream::Tcp);
    if let DialParameters::Vless { uuid, .. } = &mut parameters {
        *uuid = SecretString::new("11112233-4455-6677-8899-aabbccddeeff".into());
    }
    let connector = Connector::new(&parameters).unwrap();
    let acceptor = Acceptor::new(&listener).unwrap();
    assert!(!format!("{connector:?} {acceptor:?}").contains(UUID));
    let (client, server) = tokio::io::duplex(4096);
    let destination = target("192.0.2.1");
    let (_, accepted) = tokio::join!(
        connector.connect(client, &destination),
        acceptor.accept(server)
    );
    assert!(matches!(accepted, Err(Error::Credential)));
}

#[tokio::test]
async fn different_tls_leaf_fails_without_plaintext_fallback() {
    let (parameters, _, _) = fixture(VlessStream::Tcp);
    let (_, listener, _) = fixture(VlessStream::Tcp);
    let connector = Connector::new(&parameters).unwrap();
    let acceptor = Acceptor::new(&listener).unwrap();
    let (local, remote) = tokio::io::duplex(4096);
    let destination = target("service.example");
    let (connected, accepted) = tokio::join!(
        connector.connect(local, &destination),
        acceptor.accept(remote)
    );
    assert!(matches!(connected, Err(Error::Tls(_))));
    assert!(matches!(accepted, Err(Error::Tls(_))));
}

#[tokio::test]
async fn connector_timeout_and_cancellation_drop_owned_stream() {
    let (mut parameters, _, _) = fixture(VlessStream::Tcp);
    if let DialParameters::Vless { options, .. } = &mut parameters {
        options.connect_timeout = "10ms".parse().unwrap();
    }
    let connector = Connector::new(&parameters).unwrap();
    let destination = target("service.example");
    let (client, mut remote) = tokio::io::duplex(4096);
    assert!(matches!(
        connector.connect(client, &destination).await,
        Err(Error::Timeout)
    ));
    let mut discarded = Vec::new();
    remote.read_to_end(&mut discarded).await.unwrap();

    let (client, mut remote) = tokio::io::duplex(4096);
    let mut pending = Box::pin(connector.connect(client, &destination));
    assert!(futures_util::poll!(&mut pending).is_pending());
    drop(pending);
    remote.read_to_end(&mut Vec::new()).await.unwrap();
}

#[tokio::test]
async fn connector_budget_covers_websocket_upgrade_after_tls() {
    let (mut parameters, _, tls) = fixture(VlessStream::WebSocket);
    if let DialParameters::Vless { options, .. } = &mut parameters {
        options.connect_timeout = "100ms".parse().unwrap();
    }
    let connector = Connector::new(&parameters).unwrap();
    let server = meshway_tls::Server::new(&tls).unwrap();
    let destination = target("service.example");
    let (local, remote) = tokio::io::duplex(4096);
    let (connected, accepted_tls) = tokio::join!(
        connector.connect(local, &destination),
        server.accept(remote)
    );
    let mut server = accepted_tls.unwrap();
    assert!(matches!(connected, Err(Error::Timeout)));
    let mut request = Vec::new();
    let _ = server.read_to_end(&mut request).await;
    assert!(request.starts_with(b"GET "));
}

#[tokio::test]
async fn listener_cap_covers_vless_after_successful_tls() {
    let (parameters, listener, _) = fixture(VlessStream::Tcp);
    let mut declaration = listener.declaration().clone();
    if let Listener::Vless(options) = &mut declaration {
        options.tls.handshake_timeout = "100ms".parse().unwrap();
    }
    let acceptor = Acceptor::new(&PreparedListener::prepare(declaration).unwrap()).unwrap();
    let DialParameters::Vless { tls, .. } = parameters else {
        unreachable!()
    };
    let client = meshway_tls::Client::new(&tls).unwrap();
    let (local, remote) = tokio::io::duplex(4096);
    let (connected, accepted) = tokio::join!(client.connect(local), acceptor.accept(remote));
    let mut connected = connected.unwrap();
    assert!(matches!(accepted, Err(Error::Timeout)));
    // The local cap owns and closes the carrier; TLS reports either EOF or an
    // unclean peer shutdown, rather than retaining an abandoned connection.
    let result = connected.read_u8().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn cancelled_accept_owns_and_drops_partial_tls_or_request() {
    let (parameters, listener, _) = fixture(VlessStream::Tcp);
    let acceptor = Acceptor::new(&listener).unwrap();
    let DialParameters::Vless { tls, .. } = parameters else {
        unreachable!()
    };
    let client = meshway_tls::Client::new(&tls).unwrap();
    let (local, remote) = tokio::io::duplex(4096);
    let mut accepting = Box::pin(acceptor.accept(remote));
    let connecting = client.connect(local);
    tokio::pin!(connecting);
    let mut stream = tokio::select! {
        connected = &mut connecting => connected.unwrap(),
        _ = &mut accepting => panic!("accept must wait for request"),
    };
    stream.write_all(&[0, 0, 0]).await.unwrap();
    stream.flush().await.unwrap();
    assert!(futures_util::poll!(&mut accepting).is_pending());
    drop(accepting);
    assert!(stream.read_u8().await.is_err());
}

#[test]
fn constructors_reject_unsupported_flow_and_inconsistent_stream_settings() {
    let (mut parameters, _, _) = fixture(VlessStream::Tcp);
    if let DialParameters::Vless { flow, .. } = &mut parameters {
        *flow = "xtls-rprx-vision".into();
    }
    assert!(matches!(
        Connector::new(&parameters),
        Err(Error::Configuration)
    ));
    if let DialParameters::Vless {
        flow, websocket, ..
    } = &mut parameters
    {
        flow.clear();
        *websocket = Some(ListenerWebSocket::default());
    }
    assert!(matches!(
        Connector::new(&parameters),
        Err(Error::Configuration)
    ));
}

#[tokio::test]
async fn withheld_tls_response_times_out_large_app_write_without_reader_and_drops_carrier() {
    let (mut parameters, _, tls) = fixture(VlessStream::Tcp);
    if let DialParameters::Vless { options, .. } = &mut parameters {
        options.connect_timeout = "1s".parse().unwrap();
    }
    let connector = Connector::new(&parameters).unwrap();
    let upstream = meshway_tls::Server::new(&tls).unwrap();
    let destination = target("192.0.2.1");
    let (local, remote) = tokio::io::duplex(64);
    let (client, mut server) = tokio::join!(connector.connect(local, &destination), async {
        let mut stream = upstream.accept(remote).await.unwrap();
        let mut header = [0; 26];
        stream.read_exact(&mut header).await.unwrap();
        assert_eq!(&header[17..], &[0, 1, 0x30, 0x39, 1, 192, 0, 2, 1]);
        stream
    });
    let mut client = client.unwrap();
    // The upstream now withholds both its response header and all further reads.
    // This exceeds rustls and carrier buffering; no concurrent client read can
    // accidentally supply the clock wakeup needed by this writer-only task.
    let mut writing = tokio::spawn(async move {
        let payload = vec![0xa5; 2 * 1024 * 1024];
        let result = async {
            client.write_all(&payload).await?;
            client.flush().await
        }
        .await;
        (client, result)
    });
    let result = tokio::time::timeout(Duration::from_secs(3), &mut writing).await;
    if result.is_err() {
        writing.abort();
        let _ = writing.await;
        panic!("writer bypassed the original first-response deadline");
    }
    let (_still_owned_wrapper, result) = result.unwrap().unwrap();
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    // The wrapper remains alive, so EOF/unclean TLS EOF proves its inner carrier
    // was dropped by the deadline rather than by the caller dropping the wrapper.
    let mut drained = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(2), server.read_to_end(&mut drained))
        .await
        .unwrap();
    assert!(drained.len() < 2 * 1024 * 1024);
}

#[tokio::test]
async fn unsolicited_upgrade_extensions_or_subprotocol_never_receive_vless_credentials() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::{
        handshake::server::{Callback, ErrorResponse, Request, Response},
        http::HeaderValue,
    };

    struct Unsolicited(&'static str, &'static str);
    impl Callback for Unsolicited {
        fn on_request(
            self,
            _: &Request,
            mut response: Response,
        ) -> Result<Response, ErrorResponse> {
            response
                .headers_mut()
                .insert(self.0, HeaderValue::from_static(self.1));
            Ok(response)
        }
    }

    for (header, value) in [
        ("sec-websocket-extensions", "permessage-deflate"),
        ("sec-websocket-protocol", "unsolicited"),
    ] {
        let (parameters, _, tls) = fixture(VlessStream::WebSocket);
        let connector = Connector::new(&parameters).unwrap();
        let upstream = meshway_tls::Server::new(&tls).unwrap();
        let destination = target("192.0.2.1");
        let (local, remote) = tokio::io::duplex(128);
        let (client, first_message) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(connector.connect(local, &destination), async {
                let stream = upstream.accept(remote).await.unwrap();
                let mut websocket =
                    tokio_tungstenite::accept_hdr_async(stream, Unsolicited(header, value))
                        .await
                        .unwrap();
                // The 101 is otherwise valid. A binary message here would be
                // the real provider's credential-bearing VLESS request.
                websocket.next().await
            })
        })
        .await
        .unwrap();
        assert!(matches!(client, Err(Error::WebSocket)));
        assert!(matches!(first_message, None | Some(Err(_))));
    }
}
