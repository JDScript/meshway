use std::{
    future::{Future, poll_fn},
    sync::atomic::Ordering,
    task::Poll,
};

use meshway_transport::spec::{ListenerSecret, SecretString, TlsMode, TlsVersion};
use meshway_transport_stream::tls::{Client, Error, Server, verify_listener};
use rcgen::{CertificateParams, KeyPair};
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

mod support;
use support::{NAME, TrackedStream, authority, client, issued, listener, self_signed};

#[test]
fn certificate_chain_never_accepts_key_blocks_or_skipped_text_for_dialer_distribution() {
    let original = self_signed();
    let certificate = original.certificate.as_ref().unwrap().expose();
    let key = original
        .private_key
        .as_ref()
        .and_then(ListenerSecret::value)
        .unwrap()
        .expose();
    for pem in [
        format!("{certificate}{key}"),
        format!("{key}{certificate}"),
        format!("private preamble\n{certificate}"),
        format!("{certificate}private trailing text"),
        certificate.replace(
            "-----END CERTIFICATE-----",
            "-----END CERTIFICATE-----PRIVATE-SENTINEL",
        ),
        format!(
            "-----BEGIN CERTIFICATE-----\n-----BEGIN UNSUPPORTED-----\nYWJj\n-----END UNSUPPORTED-----\n{certificate}"
        ),
        format!(
            "-----BEGIN CERTIFICATE EXTRA-----\nYWJj\n-----END CERTIFICATE EXTRA-----\n{certificate}"
        ),
        format!("{certificate}-----BEGIN UNSUPPORTED-----\nYWJj\n-----END UNSUPPORTED-----\n"),
    ] {
        let mut tls = original.clone();
        tls.certificate = Some(SecretString::new(pem));
        assert!(matches!(verify_listener(&tls), Err(Error::Certificate)));
        assert!(matches!(Server::new(&tls), Err(Error::Certificate)));
        assert!(matches!(
            Client::new(&client(&tls)),
            Err(Error::Certificate)
        ));
    }
    let mut whitespace = original.clone();
    whitespace.certificate = Some(SecretString::new(format!("\n \t{certificate}\r\n")));
    assert!(verify_listener(&whitespace).is_ok());
}

#[tokio::test]
async fn exact_material_exchanges_bytes_with_each_supported_version_and_alpn_order() {
    for (min, max, negotiated) in [
        (
            TlsVersion::Tls12,
            TlsVersion::Tls12,
            rustls::ProtocolVersion::TLSv1_2,
        ),
        (
            TlsVersion::Tls13,
            TlsVersion::Tls13,
            rustls::ProtocolVersion::TLSv1_3,
        ),
        (
            TlsVersion::Tls12,
            TlsVersion::Tls13,
            rustls::ProtocolVersion::TLSv1_3,
        ),
    ] {
        for mut tls in [self_signed(), issued(&authority(), NAME)] {
            tls.min_version = min;
            tls.max_version = max;
            tls.alpn = vec!["meshway/1".into(), "http/1.1".into()];
            let mut input = client(&tls);
            input.options.alpn.reverse();
            let client = Client::new(&input).unwrap();
            let server = Server::new(&tls).unwrap();
            let (left, right) = duplex(16 * 1024);
            let (client, server) = tokio::join!(client.connect(left), server.accept(right));
            let mut client = client.unwrap();
            let mut server = server.unwrap();
            for stream in [&client, &server] {
                assert_eq!(stream.get_ref().1.protocol_version(), Some(negotiated));
                assert_eq!(
                    stream.get_ref().1.alpn_protocol(),
                    Some(b"meshway/1".as_slice())
                );
            }
            assert_eq!(
                client.get_ref().1.peer_certificates().unwrap()[0].as_ref(),
                verify_listener(&tls).unwrap(),
            );
            client.write_all(b"request").await.unwrap();
            client.flush().await.unwrap();
            let mut request = [0; 7];
            server.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"request");
            server.write_all(b"response").await.unwrap();
            server.flush().await.unwrap();
            let mut response = [0; 8];
            client.read_exact(&mut response).await.unwrap();
            assert_eq!(&response, b"response");
        }
    }
}

#[tokio::test]
async fn empty_alpn_succeeds_without_claiming_a_protocol() {
    let tls = self_signed();
    let client = Client::new(&client(&tls)).unwrap();
    let server = Server::new(&tls).unwrap();
    let (left, right) = duplex(16 * 1024);
    let (client, server) = tokio::join!(client.connect(left), server.accept(right));
    assert!(client.unwrap().get_ref().1.alpn_protocol().is_none());
    assert!(server.unwrap().get_ref().1.alpn_protocol().is_none());
}

#[tokio::test]
async fn cloned_cached_configs_perform_full_handshakes_after_input_is_dropped() {
    let tls = self_signed();
    let client = Client::new(&client(&tls)).unwrap();
    let server = Server::new(&tls).unwrap();
    drop(tls);
    for (client, server) in [(client.clone(), server.clone()), (client, server)] {
        let (left, right) = duplex(16 * 1024);
        let (client, server) = tokio::join!(client.connect(left), server.accept(right));
        let mut client = client.unwrap();
        let mut server = server.unwrap();
        assert_eq!(
            client.get_ref().1.handshake_kind(),
            Some(rustls::HandshakeKind::Full)
        );
        // Reuse both caches after application traffic; neither side enables
        // resumption that could bypass fresh material checks.
        server.write_all(b"x").await.unwrap();
        server.flush().await.unwrap();
        assert_eq!(client.read_u8().await.unwrap(), b'x');
    }
}

#[tokio::test]
async fn incompatible_version_and_alpn_fail_the_handshake() {
    for incompatible_alpn in [false, true] {
        let mut tls = self_signed();
        let mut input = client(&tls);
        if incompatible_alpn {
            tls.alpn = vec!["meshway/1".into()];
            input.options.alpn = vec!["http/1.1".into()];
        } else {
            tls.max_version = TlsVersion::Tls12;
            input.options.min_version = TlsVersion::Tls13;
        }
        let client = Client::new(&input).unwrap();
        let server = Server::new(&tls).unwrap();
        let (left, right) = duplex(16 * 1024);
        let (client, server) = tokio::join!(client.connect(left), server.accept(right));
        assert!(matches!(client, Err(Error::Handshake)));
        assert!(matches!(server, Err(Error::Handshake)));
    }
}

#[tokio::test]
async fn another_leaf_from_same_ca_and_name_is_not_exact_trust() {
    let ca = authority();
    let expected = issued(&ca, NAME);
    // Both are valid under the same CA for the same hostname. Only the pin
    // distinguishes them; a plain root-store implementation would accept both.
    let other = issued(&ca, NAME);
    verify_listener(&expected).unwrap();
    verify_listener(&other).unwrap();
    let client = Client::new(&client(&expected)).unwrap();
    let server = Server::new(&other).unwrap();
    let (left, right) = duplex(16 * 1024);
    let (left, dropped) = TrackedStream::new(left);
    let (client, server) = tokio::join!(client.connect(left), server.accept(right));
    assert!(matches!(client, Err(Error::Handshake)));
    assert!(server.is_err());
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn wrong_peer_name_and_unrelated_root_fail() {
    let ca = authority();
    let expected = issued(&ca, NAME);
    for other in [
        issued(&ca, "other.example.test"),
        issued(&authority(), NAME),
    ] {
        let client = Client::new(&client(&expected)).unwrap();
        let server = Server::new(&other).unwrap();
        let (left, right) = duplex(16 * 1024);
        let (client, server) = tokio::join!(client.connect(left), server.accept(right));
        assert!(matches!(client, Err(Error::Handshake)));
        assert!(server.is_err());
    }
}

#[test]
fn wrong_name_key_bad_or_expired_material_is_rejected_before_use() {
    let original = self_signed();
    let mut wrong_name = original.clone();
    wrong_name.server_name = "other.example.test".into();
    assert_eq!(
        verify_listener(&wrong_name),
        Err(Error::CertificateVerification)
    );
    assert!(matches!(
        Client::new(&client(&wrong_name)),
        Err(Error::CertificateVerification)
    ));
    assert!(matches!(
        Server::new(&wrong_name),
        Err(Error::CertificateVerification)
    ));

    let mut wrong_key = original.clone();
    wrong_key.private_key =
        Some(SecretString::new(KeyPair::generate().unwrap().serialize_pem()).into());
    assert_eq!(verify_listener(&wrong_key), Err(Error::PrivateKey));
    assert!(matches!(Server::new(&wrong_key), Err(Error::PrivateKey)));

    for (from, to) in [(2000, 2001), (2090, 2091)] {
        let mut params = CertificateParams::new(vec![NAME.into()]).unwrap();
        params.not_before = rcgen::date_time_ymd(from, 1, 1);
        params.not_after = rcgen::date_time_ymd(to, 1, 1);
        let key = KeyPair::generate().unwrap();
        let tls = listener(&params.self_signed(&key).unwrap(), &key);
        assert_eq!(verify_listener(&tls), Err(Error::CertificateVerification));
        assert!(matches!(
            Server::new(&tls),
            Err(Error::CertificateVerification)
        ));
        assert!(matches!(
            Client::new(&client(&tls)),
            Err(Error::CertificateVerification)
        ));
    }

    for mode in [TlsMode::Managed, TlsMode::Provided] {
        let mut tls = original.clone();
        tls.mode = mode;
        tls.certificate = Some(SecretString::new("private-config-sentinel".into()));
        assert_eq!(verify_listener(&tls), Err(Error::Certificate));
        assert!(matches!(Server::new(&tls), Err(Error::Certificate)));
        assert!(matches!(
            Client::new(&client(&tls)),
            Err(Error::Certificate)
        ));
        tls = original.clone();
        tls.mode = mode;
        tls.private_key = None;
        assert_eq!(verify_listener(&tls), Err(Error::PrivateKey));
        assert!(matches!(Server::new(&tls), Err(Error::PrivateKey)));
    }
}

#[test]
fn material_caps_and_missing_inputs_are_enforced() {
    let original = self_signed();
    let mut tls = original.clone();
    tls.certificate = None;
    assert_eq!(verify_listener(&tls), Err(Error::Certificate));
    tls = original.clone();
    tls.certificate = Some(SecretString::new("x".repeat(128 * 1024 + 1)));
    assert_eq!(verify_listener(&tls), Err(Error::Certificate));
    assert!(matches!(
        Client::new(&client(&tls)),
        Err(Error::Certificate)
    ));
    tls = original.clone();
    tls.private_key = Some(SecretString::new("x".repeat(32 * 1024 + 1)).into());
    assert_eq!(verify_listener(&tls), Err(Error::PrivateKey));
    tls = original.clone();
    tls.certificate = Some(SecretString::new(
        original.certificate.unwrap().expose().repeat(9),
    ));
    assert_eq!(verify_listener(&tls), Err(Error::Certificate));
    assert!(matches!(
        Client::new(&client(&tls)),
        Err(Error::Certificate)
    ));
}

#[test]
fn an_incorrect_chain_signature_is_rejected_by_both_material_consumers() {
    let ca = authority();
    let impostor_ca = authority();
    let mut tls = issued(&ca, NAME);
    let pem = tls.certificate.as_ref().unwrap().expose();
    let leaf_pem = pem.strip_suffix(&ca.pem()).unwrap();
    tls.certificate = Some(SecretString::new(leaf_pem.to_owned() + &impostor_ca.pem()));
    assert_eq!(verify_listener(&tls), Err(Error::CertificateVerification));
    assert!(matches!(
        Server::new(&tls),
        Err(Error::CertificateVerification)
    ));
    assert!(matches!(
        Client::new(&client(&tls)),
        Err(Error::CertificateVerification)
    ));
}

#[test]
fn invalid_version_alpn_and_unrepresentable_timeout_are_rejected() {
    let original = self_signed();
    let mut tls = original.clone();
    tls.min_version = TlsVersion::Tls13;
    tls.max_version = TlsVersion::Tls12;
    assert!(matches!(Server::new(&tls), Err(Error::VersionRange)));
    assert!(matches!(
        Client::new(&client(&tls)),
        Err(Error::VersionRange)
    ));
    for alpn in [
        vec![String::new()],
        vec!["x".repeat(256)],
        vec!["h2\n".into()],
        vec!["x".repeat(255); 65],
    ] {
        tls = original.clone();
        tls.alpn = alpn;
        assert!(matches!(Server::new(&tls), Err(Error::Alpn)));
        assert!(matches!(Client::new(&client(&tls)), Err(Error::Alpn)));
    }
    tls = original;
    tls.handshake_timeout = format!("{}ms", u64::MAX).parse().unwrap();
    assert!(matches!(Server::new(&tls), Err(Error::TimeoutRange)));
    assert!(matches!(
        Client::new(&client(&tls)),
        Err(Error::TimeoutRange)
    ));
}

#[tokio::test]
async fn cancellation_and_timeout_release_each_injected_stream() {
    let mut tls = self_signed();
    tls.handshake_timeout = "10ms".parse().unwrap();
    let client = Client::new(&client(&tls)).unwrap();
    let server = Server::new(&tls).unwrap();

    let (stream, _peer) = duplex(16 * 1024);
    let (stream, dropped) = TrackedStream::new(stream);
    let mut handshake = Box::pin(client.connect(stream));
    poll_fn(|cx| {
        assert!(handshake.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(!dropped.load(Ordering::SeqCst));
    drop(handshake);
    assert!(dropped.load(Ordering::SeqCst));

    let (stream, _peer) = duplex(16 * 1024);
    let (stream, dropped) = TrackedStream::new(stream);
    let mut handshake = Box::pin(server.accept(stream));
    poll_fn(|cx| {
        assert!(handshake.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(!dropped.load(Ordering::SeqCst));
    drop(handshake);
    assert!(dropped.load(Ordering::SeqCst));

    let (stream, _peer) = duplex(16 * 1024);
    let (stream, dropped) = TrackedStream::new(stream);
    assert!(matches!(client.connect(stream).await, Err(Error::Timeout)));
    assert!(dropped.load(Ordering::SeqCst));

    let (stream, _peer) = duplex(16 * 1024);
    let (stream, dropped) = TrackedStream::new(stream);
    assert!(matches!(server.accept(stream).await, Err(Error::Timeout)));
    assert!(dropped.load(Ordering::SeqCst));
}

#[test]
fn error_formatting_is_bounded_and_has_no_secret_source() {
    let mut tls = self_signed();
    tls.private_key = Some(SecretString::new("private-config-sentinel".into()).into());
    let error = verify_listener(&tls).unwrap_err();
    assert_eq!(error, Error::PrivateKey);
    assert_eq!(format!("{error:?}"), "PrivateKey");
    assert_eq!(
        error.to_string(),
        "TLS private key is missing, unsupported, malformed or mismatched"
    );
    assert!(std::error::Error::source(&error).is_none());
}
