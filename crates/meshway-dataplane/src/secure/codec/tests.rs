use super::*;
use crate::fixtures;

fn frame(kind: Kind) -> Frame {
    let a = "00000000-0000-4000-8000-000000000001".parse().unwrap();
    let b = "00000000-0000-4000-8000-000000000002".parse().unwrap();
    let diagnostic = matches!(kind, Kind::ProbeRequest | Kind::ProbeReply);
    let context = Context {
        network: "00000000-0000-4000-8000-000000000003".parse().unwrap(),
        authorization: if diagnostic {
            CircuitAuthorization::Diagnostic(
                "00000000-0000-4000-8000-000000000004".parse().unwrap(),
            )
        } else {
            CircuitAuthorization::Business("00000000-0000-4000-8000-000000000004".parse().unwrap())
        },
        path: vec![a, b],
        circuit: [9; 32],
        permit: [7; 32].into(),
        keys: [8; 32].into(),
    };
    let carrier = Carrier {
        dialer: a,
        listener: b,
        listener_key: "udp".into(),
        protocol: TransportProtocol::NativeUdp,
    };
    let facts = (kind == Kind::Data).then(|| fixtures::udp(1, 2, 50000, 443).facts());
    Frame {
        context: Arc::new(context),
        kind,
        reply: matches!(kind, Kind::Response | Kind::ProbeReply),
        sequence: 0,
        probe: diagnostic.then(|| ProbeMetadata {
            measurement_id: EventId::new(a.into_uuid()).unwrap(),
            expires_at_ms: 10_000,
        }),
        body: vec![
            0;
            if kind == Kind::Init {
                96
            } else {
                facts.map_or(48, |facts| facts.len + 16)
            }
        ]
        .into(),
        exchange: (kind == Kind::Response).then(|| Exchange {
            init: [2; 32],
            public: vec![[3; 32], [4; 32]],
        }),
        proofs: if kind.handshake() {
            vec![Proof {
                key: KeyId::new(a.into_uuid()).unwrap(),
                generation: Version::MIN,
                carrier: carrier.clone(),
                signature: [1; 64],
                ephemeral: [3; 32],
            }]
        } else {
            vec![]
        },
        auth: if kind.handshake() {
            HopAuth::default()
        } else {
            HopAuth { tag: Some([5; 16]) }
        },
    }
}

#[test]
fn every_kind_rejects_truncation_trailing_bytes_unknown_versions_and_missing_session() {
    for kind in [
        Kind::Init,
        Kind::Response,
        Kind::Data,
        Kind::ProbeRequest,
        Kind::ProbeReply,
    ] {
        let f = frame(kind);
        let bytes = f.encode().unwrap();
        for n in 0..bytes.len() {
            assert!(
                Frame::decode(&bytes[..n], Some(&f.context)).is_err(),
                "prefix {n}"
            );
        }
        assert_eq!(
            Frame::decode(&bytes, Some(&f.context))
                .unwrap()
                .encode()
                .unwrap(),
            bytes
        );
        assert_eq!(bytes[7], 5);
        for version in [1, 2, 3, 4, 6] {
            let mut wrong = bytes.clone();
            wrong[7] = version;
            assert!(Frame::decode(&wrong, Some(&f.context)).is_err());
            assert!(session_id(&wrong).is_err());
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(Frame::decode(&extra, Some(&f.context)).is_err());
        let mut unknown = bytes.clone();
        unknown[8] = 255;
        assert!(Frame::decode(&unknown, Some(&f.context)).is_err());
        if !kind.handshake() {
            assert!(Frame::decode(&bytes, None).is_err());
            assert_eq!(session_id(&bytes).unwrap(), Some(f.context.circuit));
            let mut wrong = f.context.clone();
            Arc::make_mut(&mut wrong).circuit[0] ^= 1;
            assert!(Frame::decode(&bytes, Some(&wrong)).is_err());
        }
    }
    assert!(Frame::decode(&vec![0; MAX_FRAME + 1], None).is_err());
}

#[test]
fn handshake_canonical_context_keys_and_lengths_are_bounded() {
    let mut f = frame(Kind::Init);
    let bytes = f.encode().unwrap();
    let mut bad = bytes.clone();
    bad[58] = 9;
    assert!(Frame::decode(&bad, None).is_err());
    let mut bad = bytes.clone();
    bad[9..25].fill(0);
    assert!(Frame::decode(&bad, None).is_err());
    let mut bad = bytes;
    bad[42..58].copy_from_slice(f.context.path[1].as_uuid().as_bytes());
    assert!(Frame::decode(&bad, None).is_err());
    let context = Arc::make_mut(&mut f.context);
    context.path[1] = context.path[0];
    assert!(f.encode().is_err());
    let mut f = frame(Kind::Init);
    f.proofs.clear();
    assert!(Frame::decode(&f.encode().unwrap(), None).is_err());
    for kind in [Kind::Init, Kind::Response] {
        let mut f = frame(kind);
        f.body.extend(1).unwrap()[0] = 0;
        assert!(f.encode().is_err());
        f.body.truncate(f.body.len() - 1);
        f.reply = !f.reply;
        assert!(f.encode().is_err());
        f.reply = !f.reply;
        f.sequence = 1;
        assert!(f.encode().is_err());
    }
    let mut f = frame(Kind::Response);
    f.exchange.as_mut().unwrap().public.pop();
    assert!(f.encode().is_err());
    // Maximum configured listener names and all eight path nodes still fit the
    // unchanged bounded handshake retry cache, including all contributions.
    let mut f = frame(Kind::Response);
    Arc::make_mut(&mut f.context).path = (1..=8)
        .map(|n| NodeId::new(uuid::Uuid::from_u128(n)).unwrap())
        .collect();
    f.exchange.as_mut().unwrap().public = vec![[7; 32]; 8];
    let mut proof = f.proofs.pop().unwrap();
    proof.carrier.listener_key = "x".repeat(255);
    f.proofs = (0..7)
        .map(|_| Proof {
            key: proof.key,
            generation: proof.generation,
            carrier: proof.carrier.clone(),
            signature: proof.signature,
            ephemeral: proof.ephemeral,
        })
        .collect();
    assert!(f.encode().unwrap().len() <= 4096);
}

#[test]
fn fixed_data_layout_round_trips_and_rejects_reserved_flags_and_lengths() {
    for packet in [
        fixtures::udp(1, 2, 50000, 443),
        fixtures::tcp(false, 17, 0, 2, 0),
        fixtures::echo(false, 6, 8),
    ] {
        for reply in [false, true] {
            let mut f = frame(Kind::Data);
            f.reply = reply;
            f.body = vec![0; packet.facts().len + 16].into();
            let bytes = f.encode().unwrap();
            let decoded = Frame::decode(&bytes, Some(&f.context)).unwrap();
            assert_eq!(decoded.auth.tag, f.auth.tag);
            assert_eq!(decoded.encode().unwrap(), bytes);
            assert_eq!(bytes.len(), packet.as_bytes().len() + 86);
            assert_eq!(crate::secure::DATA_FRAME_OVERHEAD, 86);
            assert_eq!(crate::secure::DEFAULT_INNER_MTU, 1366);
            assert_eq!(&bytes[..8], MAGIC);
            assert_eq!(bytes[8], 3);
            assert_eq!(bytes[9], u8::from(reply));
            assert_eq!(&bytes[10..42], &f.context.circuit);
            assert_eq!(&bytes[42..50], &f.sequence.to_be_bytes());
            assert_eq!(&bytes[50..54], &(f.body.len() as u32).to_be_bytes());
            assert_eq!(&bytes[54..bytes.len() - 16], f.body.payload());
            let mut bad = bytes.clone();
            bad[9] |= 2;
            assert!(Frame::decode(&bad, Some(&f.context)).is_err());
            let mut bad = bytes.clone();
            bad[50..54].copy_from_slice(&u32::MAX.to_be_bytes());
            assert!(Frame::decode(&bad, Some(&f.context)).is_err());
            let mut bad = bytes;
            bad[50..54].copy_from_slice(&15u32.to_be_bytes());
            assert!(Frame::decode(&bad, Some(&f.context)).is_err());
        }
    }
    let mut f = frame(Kind::Data);
    f.auth.tag = None;
    assert!(f.encode().is_err());
    let mut f = frame(Kind::Data);
    f.sequence = u64::MAX;
    assert!(f.encode().is_err());
}

#[test]
fn diagnostic_metadata_cannot_alias_business_or_reply_direction() {
    let mut f = frame(Kind::ProbeRequest);
    Arc::make_mut(&mut f.context).authorization = frame(Kind::Data).context.authorization;
    assert!(f.encode().is_err());
    let mut f = frame(Kind::ProbeRequest);
    f.reply = true;
    assert!(f.encode().is_err());
    let mut f = frame(Kind::ProbeReply);
    f.probe = None;
    assert!(f.encode().is_err());
    let mut f = frame(Kind::Data);
    Arc::make_mut(&mut f.context).authorization = frame(Kind::ProbeRequest).context.authorization;
    assert!(f.encode().is_err());
}
