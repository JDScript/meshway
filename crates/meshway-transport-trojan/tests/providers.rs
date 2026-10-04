use std::{collections::VecDeque, future::poll_fn, task::Poll};

use meshway_transport::{Frame, Limits, SendContext, SendOutcome, Session};
use meshway_transport_spec::{
    ClientTls, DialParameters, Endpoint, Listener, ListenerTls, Port, PreparedListener,
    SecretString, SocketOptions, TcpOptions, TlsMode, TlsVersion, TrojanDialer, TrojanListener,
    TrojanStream,
};
use meshway_transport_trojan::{Acceptor, Connector, Error};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn fixture() -> (DialParameters, PreparedListener) {
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
    let options = TrojanDialer::default();
    let parameters = DialParameters::Trojan {
        tls: ClientTls {
            server_name: "localhost".into(),
            certificate_chain: tls.certificate.clone().unwrap(),
            options: options.tls.clone(),
        },
        options,
        password: SecretString::new("fixture-password".into()),
    };
    let prepared = PreparedListener::prepare(Listener::Trojan(TrojanListener {
        enabled: true,
        remark: String::new(),
        listen: "127.0.0.1".parse().unwrap(),
        listen_port: Port::new(443).unwrap(),
        endpoints: vec![],
        bind_interface: String::new(),
        socket: SocketOptions::default(),
        tcp: TcpOptions::default(),
        stream: TrojanStream::Tcp,
        password: Some(SecretString::new("fixture-password".into())),
        tls,
    }))
    .unwrap();
    (parameters, prepared)
}

fn target(host: &str) -> Endpoint {
    Endpoint {
        host: host.into(),
        port: Port::new(12345).unwrap(),
    }
}

#[tokio::test]
async fn actual_tls_exchange_carries_raw_bytes_and_destination_metadata() {
    let (parameters, listener) = fixture();
    let connector = Connector::new(&parameters).unwrap();
    let acceptor = Acceptor::new(&listener).unwrap();
    for host in ["192.0.2.1", "2001:db8::1", "service.example"] {
        let destination = target(host);
        let (local, remote) = tokio::io::duplex(128);
        let (client, server) = tokio::join!(
            connector.connect(local, &destination),
            acceptor.accept(remote)
        );
        let mut client = client.unwrap();
        let mut server = server.unwrap();
        assert_eq!(server.destination, destination);
        let data = vec![0xa5; 32_000];
        let mut received = vec![0; data.len()];
        let (written, read) = tokio::join!(
            async {
                client.write_all(&data).await.unwrap();
                client.flush().await
            },
            server.stream.read_exact(&mut received)
        );
        written.unwrap();
        read.unwrap();
        assert_eq!(received, data);
        let (written, read) = tokio::join!(
            async {
                server.stream.write_all(b"response").await.unwrap();
                server.stream.flush().await
            },
            client.read_exact(&mut received[..8])
        );
        written.unwrap();
        read.unwrap();
        assert_eq!(&received[..8], b"response");
    }
}

#[tokio::test]
async fn loopback_tls_session_reuses_core_framing() {
    let (parameters, prepared) = fixture();
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

#[tokio::test]
async fn wrong_password_fails_and_debug_does_not_expose_credentials() {
    let (mut parameters, listener) = fixture();
    if let DialParameters::Trojan { password, .. } = &mut parameters {
        *password = SecretString::new("wrong-password".into());
    }
    let connector = Connector::new(&parameters).unwrap();
    let acceptor = Acceptor::new(&listener).unwrap();
    assert!(!format!("{connector:?} {acceptor:?}").contains("password"));
    let destination = target("192.0.2.1");
    let (local, remote) = tokio::io::duplex(4096);
    let (_, accepted) = tokio::join!(
        connector.connect(local, &destination),
        acceptor.accept(remote)
    );
    assert!(matches!(accepted, Err(Error::Credential)));
}

#[tokio::test]
async fn mismatched_tls_leaf_is_rejected() {
    let (parameters, _) = fixture();
    let (_, listener) = fixture();
    let connector = Connector::new(&parameters).unwrap();
    let acceptor = Acceptor::new(&listener).unwrap();
    let destination = target("service.example");
    let (local, remote) = tokio::io::duplex(4096);
    let (connected, accepted) = tokio::join!(
        connector.connect(local, &destination),
        acceptor.accept(remote)
    );
    assert!(matches!(connected, Err(Error::Tls(_))));
    assert!(matches!(accepted, Err(Error::Tls(_))));
}

#[tokio::test]
async fn timeout_and_cancel_drop_the_already_open_stream() {
    let (mut parameters, _) = fixture();
    if let DialParameters::Trojan { options, .. } = &mut parameters {
        options.connect_timeout = "10ms".parse().unwrap();
    }
    let connector = Connector::new(&parameters).unwrap();
    let destination = target("service.example");
    let (local, mut remote) = tokio::io::duplex(4096);
    assert!(matches!(
        connector.connect(local, &destination).await,
        Err(Error::Timeout)
    ));
    remote.read_to_end(&mut Vec::new()).await.unwrap();
    let (local, mut remote) = tokio::io::duplex(4096);
    let mut pending = Box::pin(connector.connect(local, &destination));
    poll_fn(|cx| {
        assert!(pending.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(pending);
    remote.read_to_end(&mut Vec::new()).await.unwrap();
}

#[tokio::test]
async fn listener_timeout_includes_connect_header_after_tls() {
    let (parameters, prepared) = fixture();
    let mut listener = prepared.declaration().clone();
    if let Listener::Trojan(options) = &mut listener {
        options.tls.handshake_timeout = "100ms".parse().unwrap();
    }
    let acceptor = Acceptor::new(&PreparedListener::prepare(listener).unwrap()).unwrap();
    let DialParameters::Trojan { tls, .. } = parameters else {
        unreachable!()
    };
    let client = meshway_tls::Client::new(&tls).unwrap();
    let (local, remote) = tokio::io::duplex(4096);
    let (connected, accepted) = tokio::join!(client.connect(local), acceptor.accept(remote));
    let mut connected = connected.unwrap();
    assert!(matches!(accepted, Err(Error::Timeout)));
    assert!(connected.read_u8().await.is_err());
}
