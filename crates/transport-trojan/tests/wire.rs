use std::{io, net::Ipv4Addr, time::Duration};

use meshway_transport_trojan::{
    ClientConfig, Error, MAX_REQUEST_HEADER_LEN, SecretPassword, ServerConfig, Target,
    client_handshake, server_handshake,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

// Independent SHA-224 test vector, not produced by this crate's serializer.
const PASSWORD_HASH: &[u8] = b"d63dc919e201d7bc4c825630d2cf25fdc93d4b2f0d46706d29038d01";

fn password() -> SecretPassword {
    SecretPassword::new(b"password").unwrap()
}

fn target() -> Target {
    Target::ip(Ipv4Addr::LOCALHOST.into(), 443).unwrap()
}

fn request(address: &[u8]) -> Vec<u8> {
    let mut bytes = PASSWORD_HASH.to_vec();
    bytes.extend_from_slice(b"\r\n\x01");
    bytes.extend_from_slice(address);
    bytes.extend_from_slice(b"\r\n");
    bytes
}

fn ipv4_request() -> Vec<u8> {
    request(&[1, 127, 0, 0, 1, 1, 187])
}

async fn parse(bytes: &[u8], expected_target: Target) -> Result<Target, Error> {
    let mut stream = io::Cursor::new(bytes.to_vec());
    server_handshake(
        &mut stream,
        &ServerConfig {
            password: password(),
            expected_target,
        },
    )
    .await
    .map(|request| request.target)
}

#[tokio::test]
async fn independent_vectors_all_address_types() {
    let mut v6 = vec![4];
    v6.extend_from_slice(
        &"2001:db8::1"
            .parse::<std::net::Ipv6Addr>()
            .unwrap()
            .octets(),
    );
    v6.extend_from_slice(&[1, 187]);
    for (target, address) in [
        (target(), vec![1, 127, 0, 0, 1, 1, 187]),
        (Target::ip("2001:db8::1".parse().unwrap(), 443).unwrap(), v6),
        (
            Target::domain("Mesh.Example.", 443).unwrap(),
            b"\x03\x0cmesh.example\x01\xbb".to_vec(),
        ),
    ] {
        let expected = request(&address);
        let mut stream = io::Cursor::new(Vec::new());
        client_handshake(
            &mut stream,
            &ClientConfig {
                password: password(),
                target: target.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(stream.into_inner(), expected);
        assert_eq!(parse(&expected, target.clone()).await.unwrap(), target);
    }
}

#[tokio::test]
async fn fragmented_header_and_coalesced_binary_payload_are_preserved() {
    let payload = (0..=255).collect::<Vec<u8>>();
    for chunk_size in [1, 2, 7, MAX_REQUEST_HEADER_LEN + 256] {
        let (mut sender, mut receiver) = duplex(5);
        let mut bytes = ipv4_request();
        bytes.extend_from_slice(&payload);
        let send = async {
            for chunk in bytes.chunks(chunk_size) {
                sender.write_all(chunk).await.unwrap();
            }
            sender.shutdown().await.unwrap();
        };
        let receive = async {
            server_handshake(
                &mut receiver,
                &ServerConfig {
                    password: password(),
                    expected_target: target(),
                },
            )
            .await
            .unwrap();
            let mut received = Vec::new();
            receiver.read_to_end(&mut received).await.unwrap();
            assert_eq!(received, payload);
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(send, receive);
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn rejects_wrong_secret_malformed_delimiters_and_unsupported_commands() {
    for (offset, byte, expected) in [
        (0, b'0', Error::AuthenticationFailed),
        (0, b'G', Error::AuthenticationFailed),
        (0, b'D', Error::AuthenticationFailed),
        (56, b'\n', Error::InvalidCrlf),
        (57, b'\r', Error::InvalidCrlf),
        (66, 0, Error::InvalidCrlf),
        (67, 0, Error::InvalidCrlf),
        (58, 3, Error::UnsupportedCommand(3)),       // UDP
        (58, 0x7f, Error::UnsupportedCommand(0x7f)), // mux
        (58, 0, Error::UnsupportedCommand(0)),
        (59, 2, Error::UnsupportedAddressType(2)), // VLESS address type != SOCKS
        (59, 5, Error::UnsupportedAddressType(5)),
    ] {
        let mut bytes = ipv4_request();
        bytes[offset] = byte;
        assert_eq!(parse(&bytes, target()).await, Err(expected));
    }
    // Trojan has no version byte. SOCKS5/VLESS-style prefixed versions fail auth.
    for version in [0, 5] {
        let mut bytes = vec![version];
        bytes.extend_from_slice(&ipv4_request());
        assert_eq!(
            parse(&bytes, target()).await,
            Err(Error::AuthenticationFailed)
        );
    }
}

#[tokio::test]
async fn rejects_every_truncated_header_and_never_consumes_payload() {
    let bytes = ipv4_request();
    for length in 0..bytes.len() {
        assert_eq!(
            parse(&bytes[..length], target()).await,
            Err(Error::Io(io::ErrorKind::UnexpectedEof))
        );
    }
    let mut bytes = bytes;
    bytes.extend_from_slice(b"\0\r\n\xff\x80");
    let mut stream = io::Cursor::new(bytes);
    server_handshake(
        &mut stream,
        &ServerConfig {
            password: password(),
            expected_target: target(),
        },
    )
    .await
    .unwrap();
    assert_eq!(stream.position(), 68);
    let mut payload = Vec::new();
    stream.read_to_end(&mut payload).await.unwrap();
    assert_eq!(payload, b"\0\r\n\xff\x80");
}

#[tokio::test]
async fn target_permission_is_exact_and_never_resolves_domains() {
    for wrong in [
        Target::ip(Ipv4Addr::LOCALHOST.into(), 444).unwrap(),
        Target::ip("127.0.0.2".parse().unwrap(), 443).unwrap(),
        Target::ip("::ffff:127.0.0.1".parse().unwrap(), 443).unwrap(),
        Target::domain("localhost", 443).unwrap(),
    ] {
        assert_eq!(
            parse(&ipv4_request(), wrong).await,
            Err(Error::TargetMismatch)
        );
    }
    assert!(
        parse(
            &request(b"\x03\x0dMESH.EXAMPLE.\x01\xbb"),
            Target::domain("mesh.example", 443).unwrap()
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn bounded_domain_validation_and_zero_port() {
    for length in [0, 254, 255] {
        // Fail immediately on the length without waiting for nonexistent bytes.
        let mut bytes = ipv4_request()[..59].to_vec();
        bytes.extend_from_slice(&[3, length]);
        assert_eq!(parse(&bytes, target()).await, Err(Error::InvalidDomain));
    }
    for name in [
        b"a..b".as_slice(),
        b"-abc",
        b"abc-",
        b"\xff",
        b"a\0b",
        b"a/b",
    ] {
        let mut address = vec![3, name.len() as u8];
        address.extend_from_slice(name);
        address.extend_from_slice(&[1, 187]);
        assert_eq!(
            parse(&request(&address), target()).await,
            Err(Error::InvalidDomain)
        );
    }
    let name = [
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61),
    ]
    .join(".");
    let maximum = Target::domain(&name, 443).unwrap();
    let mut address = vec![3, 253];
    address.extend_from_slice(name.as_bytes());
    address.extend_from_slice(&[1, 187]);
    assert_eq!(request(&address).len(), MAX_REQUEST_HEADER_LEN - 2);
    assert_eq!(
        parse(&request(&address), maximum.clone()).await.unwrap(),
        maximum
    );
    let mut bytes = ipv4_request();
    bytes[64..66].fill(0);
    assert_eq!(parse(&bytes, target()).await, Err(Error::InvalidPort));
}

#[tokio::test]
async fn server_writes_first_full_duplex_and_half_close() {
    let (mut client, mut server) = duplex(7);
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(
            async {
                client_handshake(
                    &mut client,
                    &ClientConfig {
                        password: password(),
                        target: target(),
                    },
                )
                .await
                .unwrap();
                let mut greeting = [0; 4];
                client.read_exact(&mut greeting).await.unwrap();
                assert_eq!(&greeting, b"\xff\0\r\n");
                client.write_all(b"\x80\0\x01").await.unwrap();
                client.shutdown().await.unwrap();
                let mut final_payload = Vec::new();
                client.read_to_end(&mut final_payload).await.unwrap();
                assert_eq!(&final_payload, b"\xfe\0");
            },
            async {
                server_handshake(
                    &mut server,
                    &ServerConfig {
                        password: password(),
                        expected_target: target(),
                    },
                )
                .await
                .unwrap();
                server.write_all(b"\xff\0\r\n").await.unwrap();
                let mut payload = Vec::new();
                server.read_to_end(&mut payload).await.unwrap();
                assert_eq!(&payload, b"\x80\0\x01");
                server.write_all(b"\xfe\0").await.unwrap();
                server.shutdown().await.unwrap();
            }
        );
    })
    .await
    .unwrap();
}

#[test]
fn local_validation_and_secret_redaction() {
    assert_eq!(format!("{:?}", password()), "SecretPassword([REDACTED])");
    assert_eq!(
        SecretPassword::new(b"").unwrap_err(),
        Error::InvalidPassword
    );
    assert_eq!(
        SecretPassword::new(&[0; 4097]).unwrap_err(),
        Error::InvalidPassword
    );
    assert_eq!(
        Target::ip(Ipv4Addr::LOCALHOST.into(), 0).unwrap_err(),
        Error::InvalidPort
    );
    for name in ["", ".", "é.example", "a..b", "a.", "a.."] {
        if name == "a." {
            assert!(Target::domain(name, 1).is_ok());
        } else {
            assert_eq!(Target::domain(name, 1).unwrap_err(), Error::InvalidDomain);
        }
    }
    assert_eq!(
        Target::domain(&"a".repeat(64), 1).unwrap_err(),
        Error::InvalidDomain
    );
    let error = Error::from(io::Error::other("sensitive source password"));
    assert!(!format!("{error:?} {error}").contains("sensitive"));
    let config = ClientConfig {
        password: password(),
        target: target(),
    };
    assert!(!format!("{config:?}").contains(std::str::from_utf8(PASSWORD_HASH).unwrap()));
}
