use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use meshway_transport_socks5::{
    Auth, BoundAddress, ClientConfig, Credentials, Error, ReplyCode, ServerConfig, Target,
    TargetHost, client_handshake, client_stream, server_handshake, server_stream,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

const IPV4_REQUEST: &[u8] = &[5, 1, 0, 1, 127, 0, 0, 1, 0x23, 0x45];
const SUCCESS: &[u8] = &[5, 0, 0, 1, 0, 0, 0, 0, 0, 0];

fn target() -> Target {
    Target::ip(Ipv4Addr::LOCALHOST.into(), 0x2345).unwrap()
}
fn credentials() -> Credentials {
    Credentials::new(b"user", b"pass").unwrap()
}
fn server(auth: Auth, target: Target) -> ServerConfig {
    ServerConfig {
        auth,
        expected_target: target,
        bound_address: "0.0.0.0:0".parse().unwrap(),
    }
}

/// Force both reads and writes to fragment each field, independent of scheduling.
struct Fragmented<S>(S);
impl<S: AsyncRead + Unpin> AsyncRead for Fragmented<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if out.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let mut byte = [0];
        let mut buffer = ReadBuf::new(&mut byte);
        match Pin::new(&mut self.0).poll_read(cx, &mut buffer) {
            Poll::Ready(Ok(())) => {
                out.put_slice(buffer.filled());
                Poll::Ready(Ok(()))
            }
            result => result,
        }
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for Fragmented<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, &bytes[..bytes.len().min(1)])
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

async fn read_equal<S: AsyncRead + Unpin>(stream: &mut S, bytes: &[u8]) {
    let mut actual = vec![0; bytes.len()];
    stream.read_exact(&mut actual).await.unwrap();
    assert_eq!(actual, bytes);
}

fn vectors() -> Vec<(Target, Vec<u8>)> {
    let mut ipv6 = vec![5, 1, 0, 4];
    ipv6.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
    ipv6.extend_from_slice(&[0x23, 0x45]);
    let mut domain = vec![5, 1, 0, 3, 12];
    domain.extend_from_slice(b"mesh.example");
    domain.extend_from_slice(&[0x23, 0x45]);
    vec![
        (target(), IPV4_REQUEST.to_vec()),
        (
            Target::ip(Ipv6Addr::LOCALHOST.into(), 0x2345).unwrap(),
            ipv6,
        ),
        (Target::domain("Mesh.Example.", 0x2345).unwrap(), domain),
    ]
}

#[test]
fn validates_target_and_credential_byte_bounds_and_redacts_secrets() {
    for name in [
        "",
        ".",
        "a..b",
        "-abc",
        "abc-",
        "white space",
        "abc\0def",
        "é.example",
        "a/",
        "a..",
    ] {
        assert!(
            matches!(Target::domain(name, 1), Err(Error::InvalidDomain)),
            "{name:?}"
        );
    }
    assert!(Target::domain(&"a".repeat(64), 1).is_err());
    let maximum = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    assert_eq!(maximum.len(), 253);
    assert!(Target::domain(&maximum, 1).is_ok());
    assert!(Target::domain(&(maximum + "a"), 1).is_err());
    assert!(matches!(
        Target::ip(Ipv4Addr::LOCALHOST.into(), 0),
        Err(Error::InvalidPort)
    ));
    assert!(matches!(
        Target::domain("mesh.example", 0),
        Err(Error::InvalidPort)
    ));
    assert_eq!(
        Target::domain("Mesh.Example.", 1).unwrap(),
        Target::domain("mesh.example", 1).unwrap()
    );
    for (username, password) in [
        (vec![], vec![1]),
        (vec![1], vec![]),
        (vec![1; 256], vec![1]),
        (vec![1], vec![1; 256]),
    ] {
        assert!(matches!(
            Credentials::new(&username, &password),
            Err(Error::InvalidCredentialLength)
        ));
    }
    assert!(Credentials::new(&[0; 255], &[255; 255]).is_ok());
    assert!(Credentials::new("é".repeat(127).as_bytes(), b"p").is_ok());
    assert!(Credentials::new("é".repeat(128).as_bytes(), b"p").is_err());
    let config = ClientConfig {
        auth: Auth::UsernamePassword(Credentials::new(b"secret-user", b"secret-password").unwrap()),
        target: target(),
    };
    let debug = format!("{config:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("secret-user") && !debug.contains("secret-password"));
    assert!(!Error::AuthenticationFailed.to_string().contains("secret"));
}

#[tokio::test]
async fn client_standard_vectors_fragmented_and_no_payload_read_ahead() {
    for (target, request) in vectors() {
        for auth in [Auth::NoAuth, Auth::UsernamePassword(credentials())] {
            let (client, mut peer) = tokio::io::duplex(4096);
            let config = ClientConfig {
                auth: auth.clone(),
                target: target.clone(),
            };
            let ((), ()) = tokio::join!(
                async {
                    let mut client = Fragmented(client);
                    let bound = client_handshake(&mut client, &config).await.unwrap();
                    assert_eq!(
                        bound,
                        BoundAddress::from("0.0.0.0:0".parse::<std::net::SocketAddr>().unwrap())
                    );
                    read_equal(&mut client, b"\0\xff\x05payload").await;
                    client.write_all(b"reverse\0\xff").await.unwrap();
                },
                async {
                    match auth {
                        Auth::NoAuth => {
                            read_equal(&mut peer, &[5, 1, 0]).await;
                            peer.write_all(&[5, 0]).await.unwrap();
                        }
                        Auth::UsernamePassword(_) => {
                            read_equal(&mut peer, &[5, 1, 2]).await;
                            peer.write_all(&[5, 2]).await.unwrap();
                            read_equal(&mut peer, b"\x01\x04user\x04pass").await;
                            peer.write_all(&[1, 0]).await.unwrap();
                        }
                    }
                    read_equal(&mut peer, &request).await;
                    peer.write_all(&[SUCCESS, b"\0\xff\x05payload"].concat())
                        .await
                        .unwrap();
                    read_equal(&mut peer, b"reverse\0\xff").await;
                }
            );
        }
    }
}

#[tokio::test]
async fn acceptor_standard_vectors_pipelined_without_prefix_loss() {
    for (target, request) in vectors() {
        for auth in [Auth::NoAuth, Auth::UsernamePassword(credentials())] {
            let (mut peer, local) = tokio::io::duplex(4096);
            let config = server(auth.clone(), target.clone());
            let ((), ()) = tokio::join!(
                async {
                    let mut local = Fragmented(local);
                    let validated = server_handshake(&mut local, &config).await.unwrap();
                    assert_eq!(validated.target, target);
                    // Acceptor writes first; dialing direction doesn't restrict payload direction.
                    local.write_all(b"acceptor-first\0\xff").await.unwrap();
                    read_equal(&mut local, b"\x05\0\xffmesh").await;
                },
                async {
                    let (greeting, auth_reply): (&[u8], &[u8]) = match auth {
                        Auth::NoAuth => (&[5, 1, 0], &[5, 0]),
                        Auth::UsernamePassword(_) => {
                            (b"\x05\x02\0\x02\x01\x04user\x04pass", &[5, 2, 1, 0])
                        }
                    };
                    peer.write_all(&[greeting, &request, b"\x05\0\xffmesh"].concat())
                        .await
                        .unwrap();
                    read_equal(&mut peer, auth_reply).await;
                    read_equal(&mut peer, SUCCESS).await;
                    read_equal(&mut peer, b"acceptor-first\0\xff").await;
                }
            );
        }
    }
}

/// Feed a scripted peer response to a real client and collect its exact writes.
async fn scripted_client(
    config: ClientConfig,
    reply: &[u8],
) -> (Result<BoundAddress, Error>, Vec<u8>) {
    let (mut local, mut peer) = tokio::io::duplex(4096);
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(
            async {
                let result = client_handshake(&mut local, &config).await;
                drop(local);
                result
            },
            async {
                peer.write_all(reply).await.unwrap();
                peer.shutdown().await.unwrap();
                let mut writes = vec![];
                peer.read_to_end(&mut writes).await.unwrap();
                writes
            }
        )
    })
    .await
    .expect("client must not wait beyond truncated/rejected response")
}

async fn scripted_server(
    config: ServerConfig,
    request: &[u8],
) -> (
    Result<meshway_transport_socks5::ValidatedRequest, Error>,
    Vec<u8>,
) {
    let (mut local, mut peer) = tokio::io::duplex(4096);
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(
            async {
                let result = server_handshake(&mut local, &config).await;
                drop(local);
                result
            },
            async {
                peer.write_all(request).await.unwrap();
                peer.shutdown().await.unwrap();
                let mut writes = vec![];
                peer.read_to_end(&mut writes).await.unwrap();
                writes
            }
        )
    })
    .await
    .expect("server must not wait beyond truncated/rejected request")
}

#[tokio::test]
async fn client_rejects_unoffered_methods_including_auth_downgrade() {
    for (auth, selected) in [
        (Auth::UsernamePassword(credentials()), 0),
        (Auth::NoAuth, 2),
        (Auth::NoAuth, 1),
        (Auth::NoAuth, 0xfe),
    ] {
        let expected = if matches!(auth, Auth::NoAuth) { 0 } else { 2 };
        let (result, writes) = scripted_client(
            ClientConfig {
                auth,
                target: target(),
            },
            &[5, selected],
        )
        .await;
        assert!(matches!(result, Err(Error::UnofferedMethod(method)) if method == selected));
        assert_eq!(writes, [5, 1, expected]); // No credentials or CONNECT leaked.
    }
    let (result, _) = scripted_client(
        ClientConfig {
            auth: Auth::NoAuth,
            target: target(),
        },
        &[5, 255],
    )
    .await;
    assert!(matches!(result, Err(Error::NoAcceptableMethod)));
}

#[tokio::test]
async fn server_requires_configured_method_even_when_others_are_offered() {
    for (auth, greeting) in [
        (Auth::UsernamePassword(credentials()), vec![5, 1, 0]),
        (
            Auth::UsernamePassword(credentials()),
            vec![5, 3, 0, 1, 0xfe],
        ),
        (Auth::NoAuth, vec![5, 1, 2]),
    ] {
        let (result, writes) = scripted_server(server(auth, target()), &greeting).await;
        assert!(matches!(result, Err(Error::NoAcceptableMethod)));
        assert_eq!(writes, [5, 255]);
    }
    let (result, writes) = scripted_server(server(Auth::NoAuth, target()), &[5, 0]).await;
    assert!(matches!(result, Err(Error::EmptyMethods)));
    assert_eq!(writes, [5, 255]);
    let mut request = vec![5, 255];
    request.extend_from_slice(&[1; 254]);
    request.push(0);
    request.extend_from_slice(IPV4_REQUEST);
    let (result, writes) = scripted_server(server(Auth::NoAuth, target()), &request).await;
    assert!(result.is_ok());
    assert_eq!(writes, [&[5, 0], SUCCESS].concat());
}

#[tokio::test]
async fn invalid_authentication_versions_lengths_and_secrets_fail_closed() {
    for (auth_frame, expected) in [
        (b"\x02".as_slice(), "UnsupportedAuthVersion(2)"),
        (b"\x01\0", "InvalidCredentialLength"),
        (b"\x01\x04user\0", "InvalidCredentialLength"),
        (b"\x01\x04USER\x04pass", "AuthenticationFailed"),
        (b"\x01\x04user\x04PASS", "AuthenticationFailed"),
        (b"\x01\x05user\0\x04pass", "AuthenticationFailed"),
        (b"\x01\x04user\x05pass\0", "AuthenticationFailed"),
    ] {
        let request = [&[5, 1, 2], auth_frame, IPV4_REQUEST].concat();
        let (result, writes) = scripted_server(
            server(Auth::UsernamePassword(credentials()), target()),
            &request,
        )
        .await;
        assert_eq!(format!("{:?}", result.unwrap_err()), expected);
        assert_eq!(writes, [5, 2, 1, 1]);
    }
    for (reply, expected) in [
        (&[5, 2, 5, 0][..], "UnsupportedAuthVersion(5)"),
        (&[5, 2, 1, 1], "AuthenticationFailed"),
        (&[5, 2, 1, 255], "AuthenticationFailed"),
    ] {
        let (result, writes) = scripted_client(
            ClientConfig {
                auth: Auth::UsernamePassword(credentials()),
                target: target(),
            },
            reply,
        )
        .await;
        assert_eq!(format!("{:?}", result.unwrap_err()), expected);
        assert_eq!(writes, b"\x05\x01\x02\x01\x04user\x04pass");
    }
}

#[tokio::test]
async fn maximum_binary_credentials_and_dns_name_round_trip() {
    let auth = Auth::UsernamePassword(Credentials::new(&[0; 255], &[255; 255]).unwrap());
    let name = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    let target = Target::domain(&name, 65535).unwrap();
    let server = server(auth.clone(), target.clone());
    let client = ClientConfig { auth, target };
    let (a, b) = tokio::io::duplex(1);
    let (a, b) = tokio::join!(
        client_stream(Fragmented(a), &client),
        server_stream(Fragmented(b), &server)
    );
    assert!(a.is_ok());
    assert!(b.is_ok());
}

#[tokio::test]
async fn each_standard_reply_error_and_unknown_reply_is_rejected() {
    for (code, expected) in [
        (1, ReplyCode::GeneralFailure),
        (2, ReplyCode::ConnectionNotAllowed),
        (3, ReplyCode::NetworkUnreachable),
        (4, ReplyCode::HostUnreachable),
        (5, ReplyCode::ConnectionRefused),
        (6, ReplyCode::TtlExpired),
        (7, ReplyCode::CommandNotSupported),
        (8, ReplyCode::AddressTypeNotSupported),
    ] {
        let (result, _) = scripted_client(
            ClientConfig {
                auth: Auth::NoAuth,
                target: target(),
            },
            &[5, 0, 5, code, 0],
        )
        .await;
        assert!(matches!(result, Err(Error::Reply(actual)) if actual == expected));
    }
    for code in [9, 127, 255] {
        let (result, _) = scripted_client(
            ClientConfig {
                auth: Auth::NoAuth,
                target: target(),
            },
            &[5, 0, 5, code, 0],
        )
        .await;
        assert!(matches!(result, Err(Error::UnknownReplyCode(actual)) if actual == code));
    }
}

#[tokio::test]
async fn client_checks_versions_reserved_and_bound_address() {
    for (reply, expected) in [
        (vec![4, 0], "UnsupportedVersion(4)"),
        (vec![5, 0, 4], "UnsupportedVersion(4)"),
        (vec![5, 0, 5, 0, 1], "InvalidReserved(1)"),
        (vec![5, 0, 5, 0, 0, 2], "UnsupportedAddressType(2)"),
        (vec![5, 0, 5, 0, 0, 3, 0], "InvalidDomain"),
        (vec![5, 0, 5, 0, 0, 3, 1, 255], "InvalidDomain"),
    ] {
        let (result, _) = scripted_client(
            ClientConfig {
                auth: Auth::NoAuth,
                target: target(),
            },
            &reply,
        )
        .await;
        assert_eq!(format!("{:?}", result.unwrap_err()), expected);
    }
    for (target, mut reply) in vectors() {
        reply[1] = 0;
        let (result, _) = scripted_client(
            ClientConfig {
                auth: Auth::NoAuth,
                target: target.clone(),
            },
            &[&[5, 0], &reply[..]].concat(),
        )
        .await;
        let bound = result.unwrap();
        assert_eq!(bound.host(), target.host());
        assert_eq!(bound.port(), target.port());
    }
}

#[tokio::test]
async fn server_rejects_commands_invalid_request_fields_and_wrong_targets_with_standard_replies() {
    let mut cases = vec![];
    for (index, value, error, code) in [
        (0, 4, "UnsupportedVersion(4)", 1),
        (1, 2, "UnsupportedCommand(2)", 7),
        (1, 3, "UnsupportedCommand(3)", 7),
        (1, 255, "UnsupportedCommand(255)", 7),
        (2, 1, "InvalidReserved(1)", 1),
        (3, 2, "UnsupportedAddressType(2)", 8),
        (7, 2, "TargetMismatch", 2),
        (9, 0, "TargetMismatch", 2),
    ] {
        let mut request = IPV4_REQUEST.to_vec();
        request[index] = value;
        cases.push((request, error, code));
    }
    cases.extend([
        (vec![5, 1, 0, 3, 0], "InvalidDomain", 8),
        (vec![5, 1, 0, 3, 1, 255], "InvalidDomain", 8),
        (vec![5, 1, 0, 1, 127, 0, 0, 1, 0, 0], "InvalidPort", 1),
    ]);
    for (request, expected, code) in cases {
        let (result, writes) = scripted_server(
            server(Auth::NoAuth, target()),
            &[&[5, 1, 0], request.as_slice()].concat(),
        )
        .await;
        assert_eq!(format!("{:?}", result.unwrap_err()), expected);
        let mut reply = SUCCESS.to_vec();
        reply[1] = code;
        assert_eq!(writes, [&[5, 0], reply.as_slice()].concat());
    }
    let (result, writes) = scripted_server(server(Auth::NoAuth, target()), &[4]).await;
    assert!(matches!(result, Err(Error::UnsupportedVersion(4))));
    assert!(writes.is_empty());
    // No DNS/IP equivalence or mapped-address equivalence bypasses the fixed target.
    for configured in [
        Target::domain("localhost", 0x2345).unwrap(),
        Target::ip(Ipv4Addr::LOCALHOST.to_ipv6_mapped().into(), 0x2345).unwrap(),
    ] {
        let (result, _) = scripted_server(
            server(Auth::NoAuth, configured),
            &[&[5, 1, 0], IPV4_REQUEST].concat(),
        )
        .await;
        assert!(matches!(result, Err(Error::TargetMismatch)));
    }
}

#[tokio::test]
async fn truncation_at_every_byte_of_both_handshakes_is_an_error() {
    for (target, connect) in vectors() {
        for auth in [Auth::NoAuth, Auth::UsernamePassword(credentials())] {
            let (greeting, response): (&[u8], &[u8]) = match auth {
                Auth::NoAuth => (&[5, 1, 0], &[5, 0]),
                Auth::UsernamePassword(_) => (b"\x05\x01\x02\x01\x04user\x04pass", &[5, 2, 1, 0]),
            };
            let request = [greeting, &connect].concat();
            for cut in 0..request.len() {
                let (result, _) =
                    scripted_server(server(auth.clone(), target.clone()), &request[..cut]).await;
                assert!(
                    matches!(result, Err(Error::Io(ref error)) if error.kind() == io::ErrorKind::UnexpectedEof),
                    "server cut {cut}: {result:?}"
                );
            }
            let mut bound = connect.clone();
            bound[1] = 0;
            let response = [response, &bound].concat();
            for cut in 0..response.len() {
                let (result, _) = scripted_client(
                    ClientConfig {
                        auth: auth.clone(),
                        target: target.clone(),
                    },
                    &response[..cut],
                )
                .await;
                assert!(
                    matches!(result, Err(Error::Io(ref error)) if error.kind() == io::ErrorKind::UnexpectedEof),
                    "client cut {cut}: {result:?}"
                );
            }
        }
    }
}

#[tokio::test]
async fn owned_stream_preserves_full_duplex_half_close_and_read_cancellation() {
    let client_config = ClientConfig {
        auth: Auth::NoAuth,
        target: target(),
    };
    let mut server_config = server(Auth::NoAuth, target());
    server_config.bound_address = "[::1]:1080".parse().unwrap();
    let (a, b) = tokio::io::duplex(4096);
    let (a, b) = tokio::join!(
        client_stream(a, &client_config),
        server_stream(b, &server_config)
    );
    let (mut a, bound) = a.unwrap();
    let (mut b, request) = b.unwrap();
    assert_eq!(
        bound.host(),
        &TargetHost::Ip(IpAddr::V6(Ipv6Addr::LOCALHOST))
    );
    assert_eq!(bound.port(), 1080);
    assert_eq!(request.target, target());
    let mut cancelled = [0; 1];
    assert!(
        tokio::time::timeout(Duration::from_millis(10), a.read(&mut cancelled))
            .await
            .is_err()
    );
    b.write_all(b"acceptor-first").await.unwrap();
    read_equal(&mut a, b"acceptor-first").await;
    // Simultaneous unframed binary writes in both directions.
    let data: Vec<u8> = (0..=255).collect();
    let (a_write, b_write) = tokio::join!(a.write_all(&data), b.write_all(&data));
    a_write.unwrap();
    b_write.unwrap();
    read_equal(&mut a, &data).await;
    read_equal(&mut b, &data).await;
    a.shutdown().await.unwrap();
    assert_eq!(b.read(&mut cancelled).await.unwrap(), 0);
    b.write_all(b"after-half-close").await.unwrap();
    b.shutdown().await.unwrap();
    let mut rest = vec![];
    a.read_to_end(&mut rest).await.unwrap();
    assert_eq!(rest, b"after-half-close");
    let _ = a.into_inner();
}

#[tokio::test]
async fn owned_handshake_deadline_and_cancellation_close_connection() {
    let config = ClientConfig {
        auth: Auth::NoAuth,
        target: target(),
    };
    let (a, mut peer) = tokio::io::duplex(64);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), client_stream(a, &config))
            .await
            .is_err()
    );
    let mut received = vec![];
    peer.read_to_end(&mut received).await.unwrap();
    assert_eq!(received, [5, 1, 0]);

    let (b, mut peer) = tokio::io::duplex(64);
    // Cancellation after partial authentication consumes no request/payload and
    // drops the owned socket. There is no detached handshake task left running.
    peer.write_all(b"\x05\x01\x02\x01\x04us").await.unwrap();
    let config = server(Auth::UsernamePassword(credentials()), target());
    assert!(
        tokio::time::timeout(Duration::from_millis(10), server_stream(b, &config))
            .await
            .is_err()
    );
    let mut received = vec![];
    peer.read_to_end(&mut received).await.unwrap();
    assert_eq!(received, [5, 2]);
}
