//! Private transcript fixtures, not evidence of control/daemon convergence.
use super::*;
use ed25519_dalek::{Signer, SigningKey};
use meshway_model::TransportProtocol;
use meshway_model::{KeyAlgorithm, NodePublicKey, Version};

fn node(n: u8) -> NodeId {
    format!("00000000-0000-4000-8000-{n:012}").parse().unwrap()
}
fn network(n: u8) -> NetworkId {
    format!("00000000-0000-4001-8000-{n:012}").parse().unwrap()
}
fn key(n: u8, algorithm: KeyAlgorithm) -> BoundNodeKey {
    BoundNodeKey {
        id: format!("00000000-0000-4002-8000-{n:012}").parse().unwrap(),
        generation: Version::MIN,
        public_key: NodePublicKey {
            algorithm,
            bytes: SigningKey::from_bytes(&[n; 32])
                .verifying_key()
                .to_bytes()
                .into(),
        },
    }
}
fn carrier() -> Carrier {
    // Physical R -> A is independent of the A -> R business direction.
    Carrier {
        dialer: node(2),
        listener: node(1),
        listener_key: "tcp".into(),
        protocol: TransportProtocol::NativeTcp,
    }
}

#[test]
fn proofs_reject_context_role_nonce_peer_network_and_generation_changes() {
    let secret = SigningKey::from_bytes(&[2; 32]);
    let control = key(2, KeyAlgorithm::Ed25519);
    let other = key(1, KeyAlgorithm::Ed25519);
    let dp = key(12, KeyAlgorithm::X25519);
    let other_dp = key(11, KeyAlgorithm::X25519);
    let keys = [(&control, &dp), (&other, &other_dp)];
    let context = encode_context(network(1), &carrier(), keys).unwrap();
    let client = [5; 32];
    let server = [6; 32];
    let ack = digest(ACK, &context, &client, &server);
    let signature = secret
        .sign(&packet_proof_message(network(1), node(2), &control, ack))
        .to_bytes();
    verify_key(network(1), node(2), &control, ack, &signature).unwrap();
    for altered in [
        digest(INIT, &context, &client, &[0; 32]),
        digest(CHALLENGE, &context, &client, &server),
        digest(ACK, &context, &server, &client),
        digest(ACK, &context, &[7; 32], &server),
        digest(ACK, &context, &client, &[8; 32]),
    ] {
        assert!(verify_key(network(1), node(2), &control, altered, &signature).is_err());
    }
    assert!(verify_key(network(2), node(2), &control, ack, &signature).is_err());
    assert!(verify_key(network(1), node(1), &control, ack, &signature).is_err());
    assert!(verify_key(network(1), node(2), &other, ack, &signature).is_err());
    let mut changed = control.clone();
    changed.generation = Version::new(2).unwrap();
    assert!(verify_key(network(1), node(2), &changed, ack, &signature).is_err());

    let mut altered_carriers = vec![carrier(); 4];
    altered_carriers[0].dialer = node(3);
    altered_carriers[1].listener = node(3);
    altered_carriers[2].listener_key = "other".into();
    altered_carriers[3].protocol = TransportProtocol::Vless;
    for carrier in altered_carriers {
        let context = encode_context(network(1), &carrier, keys).unwrap();
        assert!(
            verify_key(
                network(1),
                node(2),
                &control,
                digest(ACK, &context, &client, &server),
                &signature
            )
            .is_err()
        );
    }
    let altered = encode_context(
        network(1),
        &carrier(),
        [(&changed, &dp), (&other, &other_dp)],
    )
    .unwrap();
    assert_ne!(context, altered);
    let mut changed_dp = dp.clone();
    changed_dp.generation = Version::new(2).unwrap();
    assert_ne!(
        context,
        encode_context(
            network(1),
            &carrier(),
            [(&control, &changed_dp), (&other, &other_dp)]
        )
        .unwrap()
    );
    changed_dp = dp.clone();
    changed_dp.id = other_dp.id;
    assert_ne!(
        context,
        encode_context(
            network(1),
            &carrier(),
            [(&control, &changed_dp), (&other, &other_dp)]
        )
        .unwrap()
    );
    changed_dp = dp.clone();
    changed_dp.public_key.bytes = [99; 32].into();
    assert_ne!(
        context,
        encode_context(
            network(1),
            &carrier(),
            [(&control, &changed_dp), (&other, &other_dp)]
        )
        .unwrap()
    );
}

#[test]
fn response_requires_exact_context_nonces_and_complete_structure() {
    let context = b"fixed private test context";
    let response = response(context, &[1; 32], &[2; 32], [3; 64]);
    assert_eq!(
        parse_response(&response, context, &[1; 32]).unwrap(),
        [2; 32]
    );
    assert!(parse_response(&response, b"other", &[1; 32]).is_err());
    assert!(parse_response(&response, context, &[2; 32]).is_err());
    for length in 0..response.len() {
        assert!(parse_response(&response[..length], context, &[1; 32]).is_err());
    }
    let mut extra = response.clone();
    extra.push(0);
    assert!(parse_response(&extra, context, &[1; 32]).is_err());
}

#[tokio::test]
async fn framing_rejects_early_mesh_wrong_phase_oversize_and_truncation() {
    for bytes in [
        b"MWCIRv2\0\x01\x00\x01x".to_vec(),
        [MAGIC.as_slice(), &[2, 0, 1, 7]].concat(),
        [MAGIC.as_slice(), &[1, 8, 0]].concat(),
        [MAGIC.as_slice(), &[1, 0, 0]].concat(),
        [MAGIC.as_slice(), &[1, 0, 2, 7]].concat(),
    ] {
        let (mut writer, reader) = tokio::io::duplex(4096);
        writer.write_all(&bytes).await.unwrap();
        drop(writer);
        let mut reader: Box<dyn Stream> = Box::new(reader);
        assert!(read(&mut reader, 1).await.is_err());
    }
}

#[tokio::test]
async fn framing_reads_one_complete_message_without_consuming_following_frame() {
    let (writer, reader) = tokio::io::duplex(4096);
    let mut writer: Box<dyn Stream> = Box::new(writer);
    let mut reader: Box<dyn Stream> = Box::new(reader);
    write(&mut writer, 3, &[9; 160]).await.unwrap();
    writer.write_all(b"MWCIRv2").await.unwrap();
    assert_eq!(read(&mut reader, 3).await.unwrap(), vec![9; 160]);
    let mut following = [0; 7];
    reader.read_exact(&mut following).await.unwrap();
    assert_eq!(&following, b"MWCIRv2");
}

#[tokio::test]
async fn cancelled_or_timed_out_owned_exchange_releases_stream() {
    let (writer, mut peer) = tokio::io::duplex(1);
    let cancellation = tokio_util::sync::CancellationToken::new();
    let stopped = cancellation.clone();
    let worker = tokio::spawn(async move {
        let mut stream: Box<dyn Stream> = Box::new(writer);
        tokio::select! { biased;
            _ = stopped.cancelled() => {},
            _ = write(&mut stream, 1, &[7; 500]) => panic!("bounded writer should remain blocked"),
        }
    });
    cancellation.cancel();
    worker.await.unwrap();
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).await.unwrap();
    assert!(bytes.is_empty());

    let (writer, mut peer) = tokio::io::duplex(1);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), async move {
            let mut stream: Box<dyn Stream> = Box::new(writer);
            read(&mut stream, 1).await
        })
        .await
        .is_err()
    );
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
}

#[test]
fn datagram_codec_has_exact_lengths_and_distinct_udp_phase_ids() {
    for kind in [
        Kind::Init,
        Kind::Challenge,
        Kind::Ack,
        Kind::Ready,
        Kind::Ping,
        Kind::Pong,
    ] {
        let bytes = encode(kind, &[7; 160]).unwrap();
        assert_eq!(decode(&bytes).unwrap(), (kind, &[7; 160][..]));
        for size in 0..bytes.len() {
            assert!(decode(&bytes[..size]).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        // Only a Ping is filled (a path size probe), and only with zeros.
        assert_eq!(decode(&trailing).is_ok(), kind == Kind::Ping);
        let mut nonzero = trailing.clone();
        *nonzero.last_mut().unwrap() = 1;
        assert!(decode(&nonzero).is_err());
        if kind == Kind::Ping {
            let probe = encode_padded(kind, &[7; 160], 1329).unwrap();
            assert_eq!(probe.len(), 1329);
            assert_eq!(decode(&probe).unwrap(), (kind, &[7; 160][..]));
        } else {
            assert!(encode_padded(kind, &[7; 160], 1329).is_err());
        }
        let mut unknown = bytes;
        unknown[8] = 7;
        assert!(decode(&unknown).is_err());
    }
    assert!(encode(Kind::Init, &[]).is_err());
    assert!(encode(Kind::Init, &[0; MAX_MESSAGE]).is_err());
    let control = key(1, KeyAlgorithm::Ed25519);
    let dp = key(2, KeyAlgorithm::X25519);
    let mut udp = carrier();
    udp.protocol = TransportProtocol::NativeUdp;
    let context = encode_context(network(1), &udp, [(&control, &dp), (&control, &dp)]).unwrap();
    assert_eq!(context[48], 4);
}

#[test]
fn claimed_dialer_is_the_context_dialer_and_only_a_lookup_key() {
    let control = key(2, KeyAlgorithm::Ed25519);
    let dp = key(12, KeyAlgorithm::X25519);
    let context =
        encode_context(network(1), &carrier(), [(&control, &dp), (&control, &dp)]).unwrap();
    assert_eq!(claimed_dialer(&context), Some(node(2)));
    assert_eq!(claimed_dialer(&context[..20]), None);
}
