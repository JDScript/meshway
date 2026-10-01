use std::{net::Ipv4Addr, time::Duration};

use meshway_transport_shadowsocks::{
    Cipher, ClientConfig, Error, SecretPassword, ServerConfig, Target, client_stream, server_stream,
};
use shadowsocks::{
    config::{ServerConfig as BackendConfig, ServerType},
    context::Context,
    crypto::CipherKind,
    relay::{socks5::Address, tcprelay::ProxyClientStream},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

const PASSWORD: &str = "unit-test-only-carrier-secret";
const CIPHERS: [Cipher; 3] = [
    Cipher::Chacha20IetfPoly1305,
    Cipher::Aes128Gcm,
    Cipher::Aes256Gcm,
];

fn target() -> Target {
    Target::domain("meshway.internal", 19001).unwrap()
}

fn configs(cipher: Cipher, target: Target) -> (ClientConfig, ServerConfig) {
    (
        ClientConfig::new(cipher, PASSWORD.parse().unwrap(), target.clone()).unwrap(),
        ServerConfig::new(cipher, PASSWORD.parse().unwrap(), target).unwrap(),
    )
}

async fn deadline<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(15), future)
        .await
        .expect("test deadline")
}

async fn capture(config: &ClientConfig, payload: &[u8]) -> Vec<u8> {
    let (io, mut wire) = duplex(131072);
    let mut client = client_stream(io, config).await.unwrap();
    client.write_all(payload).await.unwrap();
    client.shutdown().await.unwrap();
    let mut bytes = Vec::new();
    wire.read_to_end(&mut bytes).await.unwrap();
    bytes
}

async fn inbound(bytes: &[u8], config: &ServerConfig) -> Result<Vec<u8>, Error> {
    let (mut wire, io) = duplex(131072);
    wire.write_all(bytes).await.unwrap();
    wire.shutdown().await.unwrap();
    let mut server = server_stream(io, config).await?;
    let mut payload = Vec::new();
    server.read_to_end(&mut payload).await?;
    Ok(payload)
}

#[test]
fn strict_configuration_and_redaction() {
    for cipher in CIPHERS {
        assert_eq!(cipher.as_str().parse::<Cipher>().unwrap(), cipher);
    }
    for cipher in [
        "none",
        "plain",
        "rc4-md5",
        "aes-128-cfb",
        "2022-blake3-aes-128-gcm",
        "AES-128-GCM",
        "",
        PASSWORD,
    ] {
        let error = cipher.parse::<Cipher>().unwrap_err();
        assert!(matches!(error, Error::UnsupportedCipher));
        assert!(!format!("{error:?} {error}").contains(PASSWORD));
    }
    for domain in [
        "",
        ".",
        "bad..name",
        "-bad",
        "bad-",
        "bad_name",
        "bad/name",
        "bad\0name",
        "é.example",
        "*.example",
    ] {
        assert!(Target::domain(domain, 1).is_err(), "{domain:?}");
    }
    assert!(Target::domain(&"a".repeat(64), 1).is_err());
    let max = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    assert_eq!(max.len(), 253);
    assert!(Target::domain(&max, 1).is_ok());
    assert!(Target::domain(&(max + "e"), 1).is_err());
    assert!(Target::domain("valid", 0).is_err());
    assert!(Target::ip(Ipv4Addr::LOCALHOST.into(), 0).is_err());
    assert_eq!(
        Target::domain("Meshway.Internal.", 19001).unwrap(),
        target()
    );
    assert!(SecretPassword::new(String::new()).is_err());
    assert!(SecretPassword::new("s".repeat(1025)).is_err());
    let secret: SecretPassword = PASSWORD.parse().unwrap();
    let (client, server) = configs(Cipher::Aes128Gcm, target());
    assert!(!format!("{secret:?} {client:?} {server:?}").contains(PASSWORD));
}

#[tokio::test]
async fn encrypted_fragmented_full_duplex_all_ciphers_and_address_kinds() {
    deadline(async {
        let targets = [
            Target::ip(Ipv4Addr::LOCALHOST.into(), 19001).unwrap(),
            Target::ip("::1".parse().unwrap(), 19001).unwrap(),
            target(),
        ];
        for cipher in CIPHERS {
            for target in &targets {
                let (client_config, server_config) = configs(cipher, target.clone());
                // One-byte capacity fragments salt, encrypted length, tag and payload.
                let (client, server) = duplex(1);
                let (client, server) = tokio::try_join!(
                    client_stream(client, &client_config),
                    server_stream(server, &server_config),
                )
                .unwrap();
                let (mut cr, mut cw) = tokio::io::split(client);
                let (mut sr, mut sw) = tokio::io::split(server);
                // Larger than an AEAD record, exercising native partial writes.
                let forward: Vec<u8> = (0..20000).map(|n| (n % 251) as u8).collect();
                let reverse: Vec<u8> = forward.iter().copied().rev().collect();
                let ((), (), received_forward, received_reverse) = tokio::join!(
                    async {
                        cw.write_all(&forward).await.unwrap();
                        cw.shutdown().await.unwrap();
                    },
                    async {
                        sw.write_all(&reverse).await.unwrap();
                        sw.shutdown().await.unwrap();
                    },
                    async {
                        let mut bytes = Vec::new();
                        sr.read_to_end(&mut bytes).await.unwrap();
                        bytes
                    },
                    async {
                        let mut bytes = Vec::new();
                        cr.read_to_end(&mut bytes).await.unwrap();
                        bytes
                    },
                );
                assert_eq!(received_forward, forward);
                assert_eq!(received_reverse, reverse);
            }
        }
    })
    .await;
}

#[tokio::test]
async fn client_does_not_wait_for_response_and_acceptor_can_write_first() {
    deadline(async {
        let (client_config, server_config) = configs(Cipher::Chacha20IetfPoly1305, target());
        let (client, server) = duplex(1024);
        // No server future is polled until this returns.
        let mut client = client_stream(client, &client_config).await.unwrap();
        let mut server = server_stream(server, &server_config).await.unwrap();
        server.write_all(b"\0acceptor-first\xff").await.unwrap();
        let mut payload = [0; b"\0acceptor-first\xff".len()];
        client.read_exact(&mut payload).await.unwrap();
        assert_eq!(&payload, b"\0acceptor-first\xff");
        client.shutdown().await.unwrap();
        assert_eq!(server.read(&mut [0]).await.unwrap(), 0);
        // A client half-close does not prevent the opposite business direction.
        server.write_all(b"after-half-close").await.unwrap();
        server.shutdown().await.unwrap();
        let mut rest = Vec::new();
        client.read_to_end(&mut rest).await.unwrap();
        assert_eq!(rest, b"after-half-close");
    })
    .await;
}

#[tokio::test]
async fn ciphertext_not_plaintext_wrong_password_and_exact_target_fail() {
    deadline(async {
        for cipher in CIPHERS {
            let (client, server) = configs(cipher, target());
            let payload = b"binary\0Meshway\xfffirst-payload";
            let wire = capture(&client, payload).await;
            assert!(!wire.windows(payload.len()).any(|w| w == payload));
            assert!(
                !wire
                    .windows(PASSWORD.len())
                    .any(|w| w == PASSWORD.as_bytes())
            );
            let wrong =
                ServerConfig::new(cipher, "incorrect-password".parse().unwrap(), target()).unwrap();
            assert!(inbound(&wire, &wrong).await.is_err());
            assert_eq!(inbound(&wire, &server).await.unwrap(), payload);
            for mismatch in [
                Target::domain("other.internal", 19001).unwrap(),
                Target::domain("meshway.internal", 19002).unwrap(),
                Target::ip(Ipv4Addr::LOCALHOST.into(), 19001).unwrap(),
            ] {
                let wrong = ServerConfig::new(cipher, PASSWORD.parse().unwrap(), mismatch).unwrap();
                assert!(matches!(
                    inbound(&wire, &wrong).await,
                    Err(Error::TargetMismatch)
                ));
            }
        }
    })
    .await;
}

#[tokio::test]
async fn replay_is_rejected_across_cloned_server_configs() {
    deadline(async {
        for cipher in CIPHERS {
            let (client, server) = configs(cipher, target());
            let wire = capture(&client, b"replay-test").await;
            assert_eq!(inbound(&wire, &server).await.unwrap(), b"replay-test");
            assert!(inbound(&wire, &server.clone()).await.is_err());
            // Fresh random salt is accepted by the same shared filter.
            assert_eq!(
                inbound(&capture(&client, b"fresh").await, &server)
                    .await
                    .unwrap(),
                b"fresh"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn response_replay_is_rejected_across_cloned_client_configs() {
    deadline(async {
        let (client_config, server_config) = configs(Cipher::Aes256Gcm, target());
        let request = capture(&client_config, &[]).await;
        let (mut wire, io) = duplex(4096);
        wire.write_all(&request).await.unwrap();
        let mut server = server_stream(io, &server_config).await.unwrap();
        server.write_all(b"authenticated-response").await.unwrap();
        server.shutdown().await.unwrap();
        let mut response = Vec::new();
        wire.read_to_end(&mut response).await.unwrap();
        for replay in [false, true] {
            let (io, mut wire) = duplex(4096);
            let mut client = client_stream(io, &client_config.clone()).await.unwrap();
            wire.write_all(&response).await.unwrap();
            wire.shutdown().await.unwrap();
            let mut received = Vec::new();
            let result = client.read_to_end(&mut received).await;
            if replay {
                assert!(result.is_err());
                assert!(received.is_empty());
            } else {
                result.unwrap();
                assert_eq!(received, b"authenticated-response");
            }
        }
    })
    .await;
}

#[tokio::test]
async fn mid_record_truncation_fails_but_classic_aead_has_no_authenticated_eof() {
    deadline(async {
        let (client_config, server_config) = configs(Cipher::Aes128Gcm, target());
        let complete = capture(&client_config, b"complete-record").await;
        let truncated = &complete[..complete.len() - 1];
        assert!(inbound(truncated, &server_config).await.is_err());
        // New filter for the same previously authenticated target record.
        let (_, fresh_server) = configs(Cipher::Aes128Gcm, target());
        assert_eq!(
            inbound(&complete, &fresh_server).await.unwrap(),
            b"complete-record"
        );
        // Whole records can be removed without AEAD detecting stream truncation.
        // The Meshway framing/Noise layer must enforce expected message lengths.
        let (_, fresh_server) = configs(Cipher::Aes128Gcm, target());
        let header_only_length = capture(&client_config, &[]).await.len();
        assert!(
            inbound(&complete[..header_only_length], &fresh_server)
                .await
                .unwrap()
                .is_empty()
        );
    })
    .await;
}

#[tokio::test]
async fn every_truncated_request_prefix_and_each_tampered_header_byte_rejected() {
    deadline(async {
        for cipher in CIPHERS {
            let (client, server) = configs(cipher, target());
            let wire = capture(&client, &[]).await;
            for length in 0..wire.len() {
                assert!(
                    inbound(&wire[..length], &server).await.is_err(),
                    "{cipher:?} prefix {length}"
                );
            }
            for offset in 0..wire.len() {
                let mut tampered = wire.clone();
                tampered[offset] ^= 0x80;
                assert!(
                    inbound(&tampered, &server).await.is_err(),
                    "{cipher:?} tampered {offset}"
                );
            }
            assert!(inbound(&wire, &server).await.unwrap().is_empty());
        }
    })
    .await;
}

#[tokio::test]
async fn payload_tamper_releases_no_bytes_and_poisons_both_directions() {
    deadline(async {
        let (client, server) = configs(Cipher::Aes128Gcm, target());
        let mut bytes = capture(&client, b"untrusted-payload").await;
        *bytes.last_mut().unwrap() ^= 1;
        let (mut wire, io) = duplex(4096);
        wire.write_all(&bytes).await.unwrap();
        wire.shutdown().await.unwrap();
        let mut server = server_stream(io, &server).await.unwrap();
        let mut destination = Vec::new();
        assert!(server.read_to_end(&mut destination).await.is_err());
        assert!(destination.is_empty());
        assert_eq!(
            server.write(b"no-response").await.unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            server.read(&mut [0]).await.unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
    })
    .await;
}

// Use the real upstream encoder, including its coalesced address/first-payload
// path, to exercise request parsing independently of this adapter's constructor.
async fn native_request(address: Address, payload: &[u8]) -> Vec<u8> {
    let config = BackendConfig::new(
        std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, 1)),
        PASSWORD,
        CipherKind::AES_128_GCM,
    )
    .unwrap();
    let (io, mut wire) = duplex(4096);
    let mut client = ProxyClientStream::from_stream(
        Context::new_shared(ServerType::Local),
        io,
        &config,
        address,
    );
    client.write_all(payload).await.unwrap();
    client.shutdown().await.unwrap();
    let mut bytes = Vec::new();
    wire.read_to_end(&mut bytes).await.unwrap();
    bytes
}

#[tokio::test]
async fn coalesced_native_first_payload_preserved_and_authenticated_bad_targets_rejected() {
    deadline(async {
        let (_, server) = configs(Cipher::Aes128Gcm, target());
        let payload: Vec<u8> = (0..=255).collect();
        let wire = native_request(
            Address::DomainNameAddress("Meshway.Internal.".into(), 19001),
            &payload,
        )
        .await;
        assert_eq!(inbound(&wire, &server).await.unwrap(), payload);
        for (domain, port) in [("", 19001), ("bad..name", 19001), ("meshway.internal", 0)] {
            let wire =
                native_request(Address::DomainNameAddress(domain.into(), port), b"payload").await;
            assert!(inbound(&wire, &server).await.is_err());
        }
        let wire = native_request(
            Address::DomainNameAddress("a".repeat(255), 19001),
            b"payload",
        )
        .await;
        assert!(matches!(
            inbound(&wire, &server).await,
            Err(Error::InvalidDomain)
        ));
    })
    .await;
}

#[tokio::test]
async fn timeout_cancellation_drops_owned_partial_handshakes() {
    let (client_config, server_config) = configs(Cipher::Aes128Gcm, target());
    let (io, mut peer) = duplex(1);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), client_stream(io, &client_config))
            .await
            .is_err()
    );
    let mut partial = Vec::new();
    deadline(peer.read_to_end(&mut partial)).await.unwrap();
    assert_eq!(partial.len(), 1);

    let (io, mut peer) = duplex(1);
    peer.write_all(&[42]).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), server_stream(io, &server_config))
            .await
            .is_err()
    );
    assert_eq!(deadline(peer.read(&mut [0])).await.unwrap(), 0);
}
