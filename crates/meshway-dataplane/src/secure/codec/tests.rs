use super::*;
use crate::fixtures;

fn frame() -> Frame {
    let a = "00000000-0000-4000-8000-000000000001".parse().unwrap();
    let b = "00000000-0000-4000-8000-000000000002".parse().unwrap();
    let facts = fixtures::udp(1, 2, 50000, 443).facts();
    Frame {
        context: Context {
            network: "00000000-0000-4000-8000-000000000003".parse().unwrap(),
            demand: "00000000-0000-4000-8000-000000000004".parse().unwrap(),
            path: vec![a, b],
            circuit: [9; 32],
            permit: Digest::from_bytes([7; 32]),
            keys: Digest::from_bytes([8; 32]),
        },
        kind: Kind::Data,
        reply: false,
        sequence: 0,
        facts: Some(facts),
        body: vec![0; facts.len + 16],
        proofs: vec![Proof {
            key: "00000000-0000-4000-8000-000000000005".parse().unwrap(),
            generation: Version::MIN,
            carrier: Carrier {
                dialer: a,
                listener: b,
                listener_key: "udp".to_owned(),
                protocol: TransportProtocol::NativeUdp,
            },
            signature: [1; 64],
        }],
    }
}

#[test]
fn every_truncated_prefix_trailing_bytes_and_version_rejected() {
    let bytes = frame().encode().unwrap();
    for n in 0..bytes.len() {
        assert!(Frame::decode(&bytes[..n]).is_err(), "prefix {n}");
    }
    assert_eq!(Frame::decode(&bytes).unwrap().encode().unwrap(), bytes);
    assert_eq!(bytes[7], 2);
    let mut v1 = bytes.clone();
    v1[7] = 1;
    assert!(Frame::decode(&v1).is_err());
    assert!(super::super::incoming_hint(&v1).is_err());
    let hint = super::super::incoming_hint(&bytes).unwrap();
    assert_eq!(hint.demand, frame().context.demand);
    assert_eq!(hint.path, frame().context.path);
    assert_eq!(hint.last_carrier, frame().proofs[0].carrier);
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(Frame::decode(&extra).is_err());
    let mut unknown = bytes;
    unknown[7] += 1;
    assert!(Frame::decode(&unknown).is_err());
    unknown[7] -= 1;
    unknown[8] = 255;
    assert!(Frame::decode(&unknown).is_err());
    let mut oversized_path = frame().encode().unwrap();
    oversized_path[9 + 108] = 9;
    assert!(Frame::decode(&oversized_path).is_err());
    let mut nil_network = frame().encode().unwrap();
    nil_network[9..45].copy_from_slice(b"00000000-0000-0000-0000-000000000000");
    assert!(Frame::decode(&nil_network).is_err());
    assert!(Frame::decode(&vec![0; MAX_FRAME + 1]).is_err());
    assert!(super::super::incoming_hint(&vec![0; MAX_FRAME + 1]).is_err());
}

#[test]
fn canonical_origin_path_proofs_and_handshake_lengths() {
    let mut f = frame();
    let mut bytes = f.encode().unwrap();
    bytes[9 + 72..9 + 108].copy_from_slice(f.context.path[1].to_string().as_bytes());
    assert!(Frame::decode(&bytes).is_err());
    f.context.path[1] = f.context.path[0];
    assert!(f.encode().is_err());
    let mut f = frame();
    f.proofs.clear();
    assert!(Frame::decode(&f.encode().unwrap()).is_err());
    f = frame();
    f.kind = Kind::Init;
    f.facts = None;
    f.body = vec![0; 96];
    assert!(Frame::decode(&f.encode().unwrap()).is_ok());
    f.body.push(0);
    assert!(f.encode().is_err());
    f.body.pop();
    f.reply = true;
    assert!(f.encode().is_err());
    f.kind = Kind::Response;
    f.body = vec![0; 48];
    assert!(Frame::decode(&f.encode().unwrap()).is_ok());
    f.sequence = 1;
    assert!(f.encode().is_err());
}

#[test]
fn all_packet_facts_and_carriers_round_trip_without_aliases() {
    for packet in [
        fixtures::udp(1, 2, 50000, 443),
        fixtures::tcp(false, 17, 0, 2, 0),
        fixtures::echo(false, 6, 8),
    ] {
        let mut f = frame();
        f.facts = Some(packet.facts());
        f.body = vec![0; packet.facts().len + 16];
        for protocol in [
            TransportProtocol::NativeUdp,
            TransportProtocol::NativeTcp,
            TransportProtocol::Vless,
            TransportProtocol::Trojan,
        ] {
            f.proofs[0].carrier.protocol = protocol;
            let bytes = f.encode().unwrap();
            let decoded = Frame::decode(&bytes).unwrap();
            assert_eq!(decoded.facts, f.facts);
            assert_eq!(decoded.proofs[0].carrier, f.proofs[0].carrier);
            assert_eq!(decoded.encode().unwrap(), bytes);
        }
    }
    let mut f = frame();
    f.proofs[0].carrier.listener_key = "x".repeat(256);
    assert!(f.encode().is_err());
    f = frame();
    f.sequence = u64::MAX;
    assert!(f.encode().is_err());
    let mut unknown_carrier = frame().encode().unwrap();
    let protocol_offset = unknown_carrier.len() - 64 - 3 - 1 - 1;
    unknown_carrier[protocol_offset] = 255;
    assert!(Frame::decode(&unknown_carrier).is_err());
}
