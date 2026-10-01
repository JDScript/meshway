mod support;

use std::{io::ErrorKind, time::Duration};

use meshway_transport_trojan::{
    Error, SecretPassword,
    tls::{
        ClientTls, ServerTls,
        rustls::{RootCertStore, pki_types::PrivatePkcs8KeyDer},
    },
};
use support::{Certificates, SERVER_NAME, configs};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, duplex},
    time::timeout,
};

const DEADLINE: Duration = Duration::from_secs(5);

#[tokio::test]
async fn verified_tls_reverse_business_full_duplex_and_half_close() {
    let certs = Certificates::new(false);
    let client_tls = certs.client();
    let server_tls = certs.server();
    let (client_config, server_config) = configs();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let payload: Vec<u8> = (0..32768).map(|i| (i % 256) as u8).collect();
    timeout(DEADLINE, async {
        tokio::join!(
            async {
                let mut stream = client_tls
                    .dial(address, &client_config, DEADLINE)
                    .await
                    .unwrap();
                let mut greeting = [0; 4];
                stream.read_exact(&mut greeting).await.unwrap();
                assert_eq!(&greeting, b"\0\xff\x80\x01");
                let (mut reader, mut writer) = tokio::io::split(stream);
                tokio::join!(
                    async {
                        writer.write_all(&payload).await.unwrap();
                        writer.shutdown().await.unwrap();
                    },
                    async {
                        let mut response = Vec::new();
                        reader.read_to_end(&mut response).await.unwrap();
                        assert_eq!(response, payload);
                    }
                );
            },
            async {
                let (stream, _) = listener.accept().await.unwrap();
                let (mut stream, request) = server_tls
                    .accept(stream, &server_config, DEADLINE)
                    .await
                    .unwrap();
                assert_eq!(request.target, server_config.expected_target);
                stream.write_all(b"\0\xff\x80\x01").await.unwrap();
                stream.flush().await.unwrap();
                let mut incoming = Vec::new();
                stream.read_to_end(&mut incoming).await.unwrap();
                assert_eq!(incoming, payload);
                // Our peer's close_notify closes only its writing direction.
                stream.write_all(&incoming).await.unwrap();
                stream.shutdown().await.unwrap();
            }
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn simultaneous_tls_payloads_progress_with_small_transport_buffers() {
    let certs = Certificates::new(false);
    let client_tls = certs.client();
    let server_tls = certs.server();
    let (client_config, server_config) = configs();
    let (client, server) = duplex(64);
    let forward: Vec<u8> = (0..32768).map(|i| (i % 256) as u8).collect();
    let reverse: Vec<u8> = forward.iter().rev().copied().collect();
    timeout(DEADLINE, async {
        tokio::join!(
            async {
                let stream = client_tls
                    .connect(client, &client_config, DEADLINE)
                    .await
                    .unwrap();
                let (mut read, mut write) = tokio::io::split(stream);
                tokio::join!(
                    async {
                        write.write_all(&forward).await.unwrap();
                        write.shutdown().await.unwrap();
                    },
                    async {
                        let mut received = Vec::new();
                        read.read_to_end(&mut received).await.unwrap();
                        assert_eq!(received, reverse);
                    }
                );
            },
            async {
                let (stream, _) = server_tls
                    .accept(server, &server_config, DEADLINE)
                    .await
                    .unwrap();
                let (mut read, mut write) = tokio::io::split(stream);
                tokio::join!(
                    async {
                        write.write_all(&reverse).await.unwrap();
                        write.shutdown().await.unwrap();
                    },
                    async {
                        let mut received = Vec::new();
                        read.read_to_end(&mut received).await.unwrap();
                        assert_eq!(received, forward);
                    }
                );
            }
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tls_rejects_wrong_name_untrusted_ca_and_expired_certificate() {
    for case in 0..3 {
        let certs = Certificates::new(case == 2);
        let roots = if case == 1 {
            Certificates::new(false).roots()
        } else {
            certs.roots()
        };
        let client_tls = ClientTls::new(
            if case == 0 {
                "wrong.example"
            } else {
                SERVER_NAME
            },
            roots,
        )
        .unwrap();
        let server_tls = certs.server();
        let (client_config, server_config) = configs();
        let (client, server) = duplex(16384);
        let (client, server) = tokio::join!(
            client_tls.connect(client, &client_config, DEADLINE),
            server_tls.accept(server, &server_config, DEADLINE)
        );
        assert!(
            matches!(client, Err(Error::TlsHandshake(ErrorKind::InvalidData))),
            "case {case}"
        );
        assert!(server.is_err());
    }
}

#[tokio::test]
async fn tls_does_not_turn_a_wrong_trojan_password_into_successful_authorization() {
    let certs = Certificates::new(false);
    let client_tls = certs.client();
    let server_tls = certs.server();
    let (mut client_config, server_config) = configs();
    client_config.password = SecretPassword::new(b"wrong credential").unwrap();
    let (client, server) = duplex(16384);
    let (client, server) = tokio::join!(
        client_tls.connect(client, &client_config, DEADLINE),
        server_tls.accept(server, &server_config, DEADLINE)
    );
    // There is deliberately no Trojan server ACK; request flush is not acceptance.
    assert!(client.is_ok());
    assert!(matches!(server, Err(Error::AuthenticationFailed)));
}

#[tokio::test(start_paused = true)]
async fn deadlines_cover_tls_and_trojan_and_drop_owned_streams() {
    let certs = Certificates::new(false);
    let client_tls = certs.client();
    let server_tls = certs.server();
    let (client_config, server_config) = configs();
    let (stream, mut peer) = duplex(16384);
    assert!(matches!(
        client_tls.connect(stream, &client_config, DEADLINE).await,
        Err(Error::Timeout)
    ));
    let mut leftover = Vec::new();
    peer.read_to_end(&mut leftover).await.unwrap(); // EOF proves owned stream closed
    let (stream, mut peer) = duplex(16384);
    assert!(matches!(
        server_tls.accept(stream, &server_config, DEADLINE).await,
        Err(Error::Timeout)
    ));
    assert_eq!(
        peer.read_u8().await.unwrap_err().kind(),
        ErrorKind::UnexpectedEof
    );

    // Complete TLS then stall before the Trojan request. Same accept deadline.
    let (client, server) = duplex(16384);
    let config = meshway_transport_trojan::tls::rustls::ClientConfig::builder_with_provider(
        std::sync::Arc::new(
            meshway_transport_trojan::tls::rustls::crypto::aws_lc_rs::default_provider(),
        ),
    )
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(certs.roots())
    .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));
    let (client_result, server_result) = tokio::join!(
        async {
            let stream = connector
                .connect(SERVER_NAME.try_into().unwrap(), client)
                .await
                .unwrap();
            tokio::time::sleep(DEADLINE * 2).await;
            drop(stream);
        },
        server_tls.accept(server, &server_config, DEADLINE)
    );
    let () = client_result;
    assert!(matches!(server_result, Err(Error::Timeout)));
}

#[tokio::test]
async fn cancelling_tls_accept_closes_connection() {
    let certs = Certificates::new(false);
    let server_tls = certs.server();
    let (_, server_config) = configs();
    let (stream, mut peer) = duplex(16384);
    let task =
        tokio::spawn(async move { server_tls.accept(stream, &server_config, DEADLINE).await });
    tokio::task::yield_now().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        peer.read_u8().await.unwrap_err().kind(),
        ErrorKind::UnexpectedEof
    );
}

#[test]
fn tls_configuration_is_strict_and_redacted() {
    let certs = Certificates::new(false);
    assert_eq!(
        ClientTls::new(SERVER_NAME, RootCertStore::empty()).unwrap_err(),
        Error::EmptyTrustStore
    );
    assert_eq!(
        ClientTls::new("invalid name", certs.roots()).unwrap_err(),
        Error::InvalidServerName
    );
    assert_eq!(format!("{:?}", certs.server()), "ServerTls([REDACTED])");
    assert_eq!(
        ServerTls::new(
            vec![certs.leaf.der().clone()],
            PrivatePkcs8KeyDer::from(vec![0, 1, 2]).into()
        )
        .unwrap_err(),
        Error::TlsConfig
    );
    let wrong_key = rcgen::KeyPair::generate().unwrap();
    assert_eq!(
        ServerTls::new(
            vec![certs.leaf.der().clone()],
            PrivatePkcs8KeyDer::from(wrong_key.serialize_der()).into()
        )
        .unwrap_err(),
        Error::TlsConfig
    );
}

#[tokio::test]
async fn verified_tls_supports_noise_ik_client_first_then_reverse_binary_business() {
    let certs = Certificates::new(false);
    let client_tls = certs.client();
    let server_tls = certs.server();
    let (client_config, server_config) = configs();
    let params: snow::params::NoiseParams = "Noise_IK_25519_ChaChaPoly_BLAKE2s".parse().unwrap();
    let client_key = snow::Builder::new(params.clone())
        .generate_keypair()
        .unwrap();
    let server_key = snow::Builder::new(params.clone())
        .generate_keypair()
        .unwrap();
    let mut initiator = snow::Builder::new(params.clone())
        .local_private_key(&client_key.private)
        .remote_public_key(&server_key.public)
        .build_initiator()
        .unwrap();
    let mut responder = snow::Builder::new(params)
        .local_private_key(&server_key.private)
        .build_responder()
        .unwrap();
    let (client, server) = duplex(1024);
    timeout(DEADLINE, async {
        tokio::join!(
            async {
                let mut stream = client_tls
                    .connect(client, &client_config, DEADLINE)
                    .await
                    .unwrap();
                let mut bytes = [0; 512];
                let length = initiator.write_message(&[], &mut bytes).unwrap();
                stream.write_u16(length as u16).await.unwrap();
                stream.write_all(&bytes[..length]).await.unwrap();
                stream.flush().await.unwrap();
                let length = stream.read_u16().await.unwrap() as usize;
                stream.read_exact(&mut bytes[..length]).await.unwrap();
                initiator
                    .read_message(&bytes[..length], &mut [0; 512])
                    .unwrap();
                let mut channel = initiator.into_transport_mode().unwrap();
                let length = stream.read_u16().await.unwrap() as usize;
                stream.read_exact(&mut bytes[..length]).await.unwrap();
                let mut plain = [0; 512];
                let length = channel.read_message(&bytes[..length], &mut plain).unwrap();
                assert_eq!(&plain[..length], b"\0\xff\x80reverse business");
            },
            async {
                let (mut stream, _) = server_tls
                    .accept(server, &server_config, DEADLINE)
                    .await
                    .unwrap();
                let mut bytes = [0; 512];
                let length = stream.read_u16().await.unwrap() as usize;
                stream.read_exact(&mut bytes[..length]).await.unwrap();
                responder
                    .read_message(&bytes[..length], &mut [0; 512])
                    .unwrap();
                assert_eq!(responder.get_remote_static().unwrap(), client_key.public);
                let length = responder.write_message(&[], &mut bytes).unwrap();
                stream.write_u16(length as u16).await.unwrap();
                stream.write_all(&bytes[..length]).await.unwrap();
                let mut channel = responder.into_transport_mode().unwrap();
                let length = channel
                    .write_message(b"\0\xff\x80reverse business", &mut bytes)
                    .unwrap();
                stream.write_u16(length as u16).await.unwrap();
                stream.write_all(&bytes[..length]).await.unwrap();
                stream.flush().await.unwrap();
            }
        );
    })
    .await
    .unwrap();
}
