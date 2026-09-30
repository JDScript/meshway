use std::{io, net::IpAddr, time::Duration};

use meshway_transport_vless::{
    ClientConfig, Error, MAX_REQUEST_HEADER_LEN, SecretUuid, ServerConfig, TRANSPORT_LABEL, Target,
    TransportLabel, ValidatedRequest, client_handshake, client_stream, read_server_response,
    server_handshake, write_client_request,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex, split};
use tokio::time::timeout;

// Public protocol-test vector, not a deployable credential.
const UUID: &str = "00112233-4455-6677-8899-aabbccddeeff";
const UUID_BYTES: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];

fn ip_target() -> Target {
    Target::ip("127.0.0.1".parse().unwrap(), 19001).unwrap()
}

fn configs(target: Target) -> (ClientConfig, ServerConfig) {
    (
        ClientConfig {
            uuid: UUID.parse().unwrap(),
            target: target.clone(),
        },
        ServerConfig {
            uuid: UUID.parse().unwrap(),
            expected_target: target,
        },
    )
}

// Independent wire fixture: version, UUID, empty addons, TCP, big-endian port,
// VLESS address tag (not SOCKS5's domain/IPv6 tag ordering), then address bytes.
fn request(address_type: u8, address: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0];
    bytes.extend_from_slice(&UUID_BYTES);
    bytes.extend_from_slice(&[0, 1, 0x4a, 0x39, address_type]);
    bytes.extend_from_slice(address);
    bytes
}

fn domain_request(name: &[u8]) -> Vec<u8> {
    let mut address = vec![u8::try_from(name.len()).unwrap()];
    address.extend_from_slice(name);
    request(2, &address)
}

async fn accept_wire(bytes: &[u8], target: Target) -> (Result<ValidatedRequest, Error>, Vec<u8>) {
    let (_, config) = configs(target);
    let (mut peer, mut server) = duplex(1024);
    peer.write_all(bytes).await.unwrap();
    peer.shutdown().await.unwrap();
    let result = timeout(
        Duration::from_secs(2),
        server_handshake(&mut server, &config),
    )
    .await
    .expect("bounded input must finish");
    drop(server);
    let mut response = Vec::new();
    peer.read_to_end(&mut response).await.unwrap();
    (result, response)
}

#[test]
fn uuid_is_strict_and_never_in_debug_or_parse_errors() {
    for valid in [UUID, "00112233-4455-6677-8899-AABBCCDDEEFF"] {
        let secret: SecretUuid = valid.parse().unwrap();
        assert_eq!(format!("{secret:?}"), "SecretUuid([REDACTED])");
    }
    for invalid in [
        "",
        "00112233445566778899aabbccddeeff",
        "00112233_4455-6677-8899-aabbccddeeff",
        "00112233-4455-6677-8899-aabbccddeefg",
        " 00112233-4455-6677-8899-aabbccddeeff",
        "{00112233-4455-6677-8899-aabbccddeeff}",
        "urn:uuid:00112233-4455-6677-8899-aabbccddeeff",
        "00112233-4455-6677-8899-aabbccddeeé",
    ] {
        let error = invalid.parse::<SecretUuid>().unwrap_err();
        assert!(matches!(error, Error::InvalidUuid));
        assert!(!format!("{error:?}: {error}").contains(UUID));
    }
    let (client, server) = configs(ip_target());
    assert!(!format!("{client:?} {server:?}").contains(UUID));
    assert!(!format!("{client:?} {server:?}").contains("aabbccdd"));
}

#[test]
fn targets_validate_and_canonicalize_without_dns_or_wildcards() {
    assert_eq!(
        Target::domain("MESH.Example.", 19001).unwrap(),
        Target::domain("mesh.example", 19001).unwrap()
    );
    assert!(Target::domain("xn--bcher-kva.example", 19001).is_ok());
    for invalid in [
        "",
        ".",
        "a..b",
        "a..",
        "-a.example",
        "a-.example",
        "*.example",
        "a_b",
        "a/b",
        "a\0b",
        "bücher.example",
        " a.example",
    ] {
        assert!(matches!(
            Target::domain(invalid, 19001),
            Err(Error::InvalidDomain)
        ));
    }
    assert!(Target::domain(&"a".repeat(64), 19001).is_err());
    assert!(
        Target::domain(
            &format!(
                "{}.{}.{}.{}",
                "a".repeat(63),
                "b".repeat(63),
                "c".repeat(63),
                "d".repeat(63)
            ),
            19001
        )
        .is_err()
    );
    assert!(matches!(
        Target::domain("mesh.example", 0),
        Err(Error::InvalidPort)
    ));
    assert!(matches!(
        Target::ip(IpAddr::from([127, 0, 0, 1]), 0),
        Err(Error::InvalidPort)
    ));
    assert_ne!(Target::domain("127.0.0.1", 19001).unwrap(), ip_target());
    assert_ne!(
        Target::ip("::ffff:127.0.0.1".parse().unwrap(), 19001).unwrap(),
        ip_target()
    );
    assert_eq!(TRANSPORT_LABEL, TransportLabel::VlessTcp);
}

#[tokio::test]
async fn client_emits_standard_ipv4_ipv6_and_domain_wire_vectors() {
    let cases = [
        (ip_target(), request(1, &[127, 0, 0, 1])),
        (
            Target::ip("2001:db8::7".parse().unwrap(), 19001).unwrap(),
            request(
                3,
                &[0x20, 1, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7],
            ),
        ),
        (
            Target::domain("MESH.Example.", 19001).unwrap(),
            domain_request(b"mesh.example"),
        ),
    ];
    for (target, expected) in cases {
        let (config, _) = configs(target.clone());
        let mut actual = Vec::new();
        write_client_request(&mut actual, &config).await.unwrap();
        assert_eq!(actual, expected);
        assert!(actual.len() <= MAX_REQUEST_HEADER_LEN);
        let (accepted, response) = accept_wire(&expected, target.clone()).await;
        assert_eq!(accepted.unwrap().target, target);
        assert_eq!(response, [0, 0]);
    }
}

#[tokio::test]
async fn server_canonicalizes_domain_before_exact_allowlist_comparison() {
    let target = Target::domain("mesh.example", 19001).unwrap();
    let (accepted, response) = accept_wire(&domain_request(b"MESH.Example."), target.clone()).await;
    assert_eq!(accepted.unwrap().target, target);
    assert_eq!(response, [0, 0]);
}

#[tokio::test]
async fn fragmented_handshake_and_simultaneous_binary_full_duplex() {
    timeout(Duration::from_secs(5), async {
        // Capacity one forces fragmented header reads and partial writes.
        let (mut sha, mut lax) = duplex(1);
        let (client, server) = configs(ip_target());
        let (dialed, accepted) = tokio::join!(
            client_handshake(&mut sha, &client),
            server_handshake(&mut lax, &server),
        );
        dialed.unwrap();
        assert_eq!(accepted.unwrap().target, ip_target());

        let sha_bytes: Vec<u8> = (0..4096).map(|n| (n % 256) as u8).collect();
        let lax_bytes: Vec<u8> = sha_bytes.iter().rev().copied().collect();
        let (mut sha_read, mut sha_write) = split(sha);
        let (mut lax_read, mut lax_write) = split(lax);
        let (sha_sent, lax_sent, at_sha, at_lax) = tokio::join!(
            async {
                sha_write.write_all(&sha_bytes).await.unwrap();
                sha_write.shutdown().await
            },
            async {
                lax_write.write_all(&lax_bytes).await.unwrap();
                lax_write.shutdown().await
            },
            async {
                let mut received = Vec::new();
                sha_read.read_to_end(&mut received).await.unwrap();
                received
            },
            async {
                let mut received = Vec::new();
                lax_read.read_to_end(&mut received).await.unwrap();
                received
            }
        );
        sha_sent.unwrap();
        lax_sent.unwrap();
        assert_eq!(at_sha, lax_bytes);
        assert_eq!(at_lax, sha_bytes);
    })
    .await
    .expect("fragmented full-duplex transfer must complete");
}

#[tokio::test]
async fn acceptor_can_send_first_after_reverse_dial_and_half_close() {
    timeout(Duration::from_secs(2), async {
        let (mut sha, mut lax) = duplex(64);
        let (client, server) = configs(ip_target());
        let (a, b) = tokio::join!(
            client_handshake(&mut sha, &client),
            server_handshake(&mut lax, &server)
        );
        a.unwrap();
        b.unwrap();
        lax.write_all(b"\0\xffLAX-to-SHA\x80").await.unwrap();
        lax.shutdown().await.unwrap();
        let mut received = Vec::new();
        sha.read_to_end(&mut received).await.unwrap();
        assert_eq!(received, b"\0\xffLAX-to-SHA\x80");
        // Closing LAX's write half must not close SHA's write half.
        sha.write_all(b"\xffSHA-reply\0").await.unwrap();
        sha.shutdown().await.unwrap();
        received.clear();
        lax.read_to_end(&mut received).await.unwrap();
        assert_eq!(received, b"\xffSHA-reply\0");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn coalesced_payload_is_never_consumed_by_handshake() {
    let (mut peer, mut server) = duplex(1024);
    let (_, config) = configs(ip_target());
    let mut bytes = request(1, &[127, 0, 0, 1]);
    bytes.extend_from_slice(b"\x00\xff\x01\x02payload");
    peer.write_all(&bytes).await.unwrap();
    peer.shutdown().await.unwrap();
    server_handshake(&mut server, &config).await.unwrap();
    let mut payload = Vec::new();
    server.read_to_end(&mut payload).await.unwrap();
    assert_eq!(payload, b"\x00\xff\x01\x02payload");

    server.write_all(b"\xff\0response").await.unwrap();
    server.shutdown().await.unwrap();
    // The actual response header and payload are now coalesced in the buffer.
    read_server_response(&mut peer).await.unwrap();
    payload.clear();
    peer.read_to_end(&mut payload).await.unwrap();
    assert_eq!(payload, b"\xff\0response");
}

#[tokio::test]
async fn rejects_every_wrong_uuid_byte_without_reply_or_secret_disclosure() {
    for index in 1..=16 {
        let mut bytes = request(1, &[127, 0, 0, 1]);
        bytes[index] ^= 0xff;
        let (result, response) = accept_wire(&bytes, ip_target()).await;
        let error = result.unwrap_err();
        assert!(matches!(error, Error::AuthenticationFailed));
        assert!(!format!("{error:?}: {error}").contains(UUID));
        assert!(response.is_empty());
    }
}

#[tokio::test]
async fn rejects_wrong_version_non_tcp_unknown_address_and_zero_port() {
    let original = request(1, &[127, 0, 0, 1]);
    for version in [1, 255] {
        let mut bytes = original.clone();
        bytes[0] = version;
        let (result, response) = accept_wire(&bytes, ip_target()).await;
        assert!(matches!(result, Err(Error::UnsupportedVersion(v)) if v == version));
        assert!(response.is_empty());
    }
    for command in [0, 2, 3, 255] {
        let mut bytes = original.clone();
        bytes[18] = command;
        let (result, response) = accept_wire(&bytes, ip_target()).await;
        assert!(matches!(result, Err(Error::UnsupportedCommand(c)) if c == command));
        assert!(response.is_empty());
    }
    let mut bytes = original.clone();
    bytes[21] = 4;
    let (result, response) = accept_wire(&bytes, ip_target()).await;
    assert!(matches!(result, Err(Error::UnsupportedAddressType(4))));
    assert!(response.is_empty());

    let mut bytes = original;
    bytes[19..21].copy_from_slice(&[0, 0]);
    let (result, response) = accept_wire(&bytes, ip_target()).await;
    assert!(matches!(result, Err(Error::InvalidPort)));
    assert!(response.is_empty());
}

#[tokio::test]
async fn rejects_all_addons_including_vision_and_malformed_protobuf_without_draining() {
    for addons in [
        vec![0xff],          // invalid protobuf
        vec![10, 127, b'x'], // declared protobuf field overruns addon body
        [vec![10, 16], b"xtls-rprx-vision".to_vec()].concat(),
        vec![0; 255],
    ] {
        let mut bytes = vec![0];
        bytes.extend_from_slice(&UUID_BYTES);
        bytes.push(u8::try_from(addons.len()).unwrap());
        bytes.extend_from_slice(&addons);
        let (result, response) = accept_wire(&bytes, ip_target()).await;
        assert!(matches!(result, Err(Error::UnsupportedAddons(_))));
        assert!(response.is_empty());
    }
    // Nonempty addon lengths are rejected immediately, even if a peer withholds
    // the extension body. This is an explicit unsupported feature, not EOF.
    let mut header = vec![0];
    header.extend_from_slice(&UUID_BYTES);
    header.push(255);
    let (mut peer, mut server) = duplex(64);
    peer.write_all(&header).await.unwrap();
    let (_, config) = configs(ip_target());
    assert!(matches!(
        timeout(
            Duration::from_secs(1),
            server_handshake(&mut server, &config)
        )
        .await
        .unwrap(),
        Err(Error::UnsupportedAddons(255))
    ));
}

#[tokio::test]
async fn fixed_target_cannot_be_used_as_an_open_proxy() {
    let cases = [
        request(1, &[127, 0, 0, 2]),
        request(1, &[8, 8, 8, 8]),
        request(3, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 127, 0, 0, 1]),
        domain_request(b"127.0.0.1"),
        domain_request(b"localhost"),
    ];
    for bytes in cases {
        let (result, response) = accept_wire(&bytes, ip_target()).await;
        assert!(matches!(result, Err(Error::TargetMismatch)));
        assert!(response.is_empty());
    }
    let mut wrong_port = request(1, &[127, 0, 0, 1]);
    wrong_port[20] ^= 1;
    let (result, response) = accept_wire(&wrong_port, ip_target()).await;
    assert!(matches!(result, Err(Error::TargetMismatch)));
    assert!(response.is_empty());

    for name in [
        b"evilmesh.example".as_slice(),
        b"child.mesh.example",
        b"mesh.example.evil",
    ] {
        let (result, response) = accept_wire(
            &domain_request(name),
            Target::domain("mesh.example", 19001).unwrap(),
        )
        .await;
        assert!(matches!(result, Err(Error::TargetMismatch)));
        assert!(response.is_empty());
    }
}

#[tokio::test]
async fn rejects_invalid_wire_domains_and_bounds_maximum_header() {
    for name in [vec![], vec![0xff], b"bad\0name".to_vec(), vec![b'a'; 255]] {
        let bytes = domain_request(&name);
        assert!(bytes.len() <= MAX_REQUEST_HEADER_LEN);
        let (result, response) = accept_wire(&bytes, ip_target()).await;
        assert!(matches!(result, Err(Error::InvalidDomain)));
        assert!(response.is_empty());
    }
    let name = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    assert_eq!(name.len(), 253);
    let target = Target::domain(&name, 19001).unwrap();
    let (result, response) = accept_wire(&domain_request(name.as_bytes()), target.clone()).await;
    assert_eq!(result.unwrap().target, target);
    assert_eq!(response, [0, 0]);
}

#[tokio::test]
async fn every_truncated_supported_request_fails_with_eof() {
    let cases = [
        (request(1, &[127, 0, 0, 1]), ip_target()),
        (
            request(3, &[0; 16]),
            Target::ip("::".parse().unwrap(), 19001).unwrap(),
        ),
        (
            domain_request(b"mesh.example"),
            Target::domain("mesh.example", 19001).unwrap(),
        ),
    ];
    for (bytes, target) in cases {
        for length in 0..bytes.len() {
            let (result, response) = accept_wire(&bytes[..length], target.clone()).await;
            assert!(
                matches!(result, Err(Error::Io(ref error)) if error.kind() == io::ErrorKind::UnexpectedEof),
                "prefix length {length}: {result:?}"
            );
            assert!(response.is_empty());
        }
    }
}

#[tokio::test]
async fn response_validation_is_bounded_and_rejects_versions_addons_and_eof() {
    for bytes in [b"".as_slice(), &[0]] {
        let mut input = bytes;
        let error = read_server_response(&mut input).await.unwrap_err();
        assert!(matches!(error, Error::Io(error) if error.kind() == io::ErrorKind::UnexpectedEof));
    }
    let mut input = [1, 0].as_slice();
    assert!(matches!(
        read_server_response(&mut input).await,
        Err(Error::UnsupportedVersion(1))
    ));
    for length in [1, 255] {
        let bytes = [0, length, 0xff, 0x80];
        let mut input = bytes.as_slice();
        assert!(
            matches!(read_server_response(&mut input).await, Err(Error::UnsupportedAddons(l)) if l == length)
        );
        assert_eq!(input, [0xff, 0x80]); // no drain or payload read-ahead
    }
}

#[tokio::test]
async fn external_proxy_lazy_response_allows_client_first_noise_bytes() {
    timeout(Duration::from_secs(3), async {
        let (mut sha, mut proxy) = duplex(1);
        let (config, _) = configs(ip_target());
        let client = async {
            let mut carrier = client_stream(&mut sha, &config).await.unwrap();
            assert!(!carrier.response_validated());
            carrier = carrier.try_into_inner().unwrap_err();
            // Stand-in bytes only: the core supplies the actual authenticated
            // Noise handshake. This tests ordering, not cryptographic identity.
            carrier.write_all(b"\0\xffNoise-first").await.unwrap();
            carrier.flush().await.unwrap();
            let mut response = [0; 13];
            carrier.read_exact(&mut response).await.unwrap();
            assert_eq!(&response, b"\x80Noise-answer");
            assert!(carrier.response_validated());
            let inner = carrier.try_into_inner().unwrap();
            inner.write_all(b"\xffack").await.unwrap();
        };
        let server = async {
            let mut header = [0; 26];
            proxy.read_exact(&mut header).await.unwrap();
            assert_eq!(header.as_slice(), request(1, &[127, 0, 0, 1]));
            // Like sing-box, withhold the VLESS response until upstream's first
            // reply. Waiting for response before writing would deadlock here.
            let mut initial_noise = [0; 13];
            proxy.read_exact(&mut initial_noise).await.unwrap();
            assert_eq!(&initial_noise, b"\0\xffNoise-first");
            proxy.write_all(b"\0\0\x80Noise-answer").await.unwrap();
            let mut ack = [0; 4];
            proxy.read_exact(&mut ack).await.unwrap();
            assert_eq!(&ack, b"\xffack");
        };
        tokio::join!(client, server);
    })
    .await
    .expect("lazy response must permit client-first payload");
}

#[tokio::test]
async fn lazy_response_failure_is_terminal_and_preserves_typed_protocol_error() {
    for response in [vec![1], vec![0, 255], vec![0], vec![]] {
        let (mut client, mut peer) = duplex(1024);
        let (config, _) = configs(ip_target());
        let mut carrier = client_stream(&mut client, &config).await.unwrap();
        peer.write_all(&response).await.unwrap();
        peer.shutdown().await.unwrap();
        let mut buffer = [0; 32];
        let error = carrier.read(&mut buffer).await.unwrap_err();
        if response == [1] {
            assert!(matches!(
                error.get_ref().and_then(|e| e.downcast_ref::<Error>()),
                Some(Error::UnsupportedVersion(1))
            ));
        } else if response == [0, 255] {
            assert!(matches!(
                error.get_ref().and_then(|e| e.downcast_ref::<Error>()),
                Some(Error::UnsupportedAddons(255))
            ));
        } else {
            assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        }
        assert!(!carrier.response_validated());
        assert!(carrier.read(&mut buffer).await.is_err());
        assert!(carrier.write_all(b"must-not-be-sent").await.is_err());
        assert!(carrier.flush().await.is_err());
        carrier.shutdown().await.unwrap();
        let mut received = Vec::new();
        peer.read_to_end(&mut received).await.unwrap();
        assert_eq!(received, request(1, &[127, 0, 0, 1]));
    }
}
