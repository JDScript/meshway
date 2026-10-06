use super::*;
use crate::{fixtures, secure::codec::Kind};
use ed25519_dalek::{Signer, SigningKey};
use meshway_model::{KeyId, NetworkId, NodePublicKey, Version};
use meshway_transport_spec::TransportProtocol;
use std::collections::BTreeMap;

fn keypair() -> (Zeroizing<[u8; 32]>, [u8; 32]) {
    let builder = snow::Builder::new(NOISE.parse().unwrap());
    let mut pair = builder.generate_keypair().unwrap();
    let private = Zeroizing::new(pair.private.as_slice().try_into().unwrap());
    use zeroize::Zeroize;
    pair.private.zeroize();
    (private, pair.public.as_slice().try_into().unwrap())
}

fn handshake(prologue: &[u8]) -> (snow::StatelessTransportState, snow::StatelessTransportState) {
    let (a, ap) = keypair();
    let (b, bp) = keypair();
    let mut initiator = build_with_key(&a, &bp, prologue, true).unwrap();
    let mut responder = build_with_key(&b, &ap, prologue, false).unwrap();
    let init = write_handshake(&mut initiator, 96).unwrap();
    read_handshake(&mut responder, &init, 96, &ap).unwrap();
    let response = write_handshake(&mut responder, 48).unwrap();
    read_handshake(&mut initiator, &response, 48, &bp).unwrap();
    (
        initiator.into_stateless_transport_mode().unwrap(),
        responder.into_stateless_transport_mode().unwrap(),
    )
}

#[test]
fn noise_protects_payload_both_directions_and_exact_facts() {
    let (a, b) = handshake(b"same full circuit context");
    let packet = fixtures::udp(1, 3, 50000, 443);
    let facts = packet.facts();
    let plain = packet.as_bytes().to_vec();
    let encrypted = encrypt(&a, 0, packet).unwrap();
    assert!(
        !encrypted
            .windows(b"private test payload".len())
            .any(|bytes| bytes == b"private test payload")
    );
    assert_eq!(decrypt(&b, 0, &encrypted, facts).unwrap().as_bytes(), plain);
    let mut tampered = encrypted.clone();
    tampered[0] ^= 1;
    assert!(decrypt(&b, 0, &tampered, facts).is_err());
    assert!(decrypt(&b, 1, &encrypted, facts).is_err());
    let mut wrong = facts;
    wrong.traffic.source = fixtures::address(2);
    assert!(decrypt(&b, 0, &encrypted, wrong).is_err());
    let reply = fixtures::udp(3, 1, 443, 50000);
    let facts = reply.facts();
    let encrypted = encrypt(&b, 0, reply).unwrap();
    decrypt(&a, 0, &encrypted, facts).unwrap();
    let (other, _) = handshake(b"different circuit");
    assert!(decrypt(&other, 0, &encrypted, facts).is_err());
}

#[test]
fn wrong_peer_wrong_prologue_and_application_handshake_payload_rejected() {
    let (a, ap) = keypair();
    let (b, bp) = keypair();
    let (_, wrong) = keypair();
    let mut initiator = build_with_key(&a, &bp, b"context", true).unwrap();
    let init = write_handshake(&mut initiator, 96).unwrap();
    let mut responder = build_with_key(&b, &ap, b"other context", false).unwrap();
    assert!(read_handshake(&mut responder, &init, 96, &ap).is_err());
    let mut responder = build_with_key(&b, &ap, b"context", false).unwrap();
    assert!(read_handshake(&mut responder, &init, 96, &wrong).is_err());
    let mut initiator = build_with_key(&a, &wrong, b"context", true).unwrap();
    let init = write_handshake(&mut initiator, 96).unwrap();
    let mut responder = build_with_key(&b, &ap, b"context", false).unwrap();
    assert!(read_handshake(&mut responder, &init, 96, &ap).is_err());
    let mut initiator = build_with_key(&a, &bp, b"context", true).unwrap();
    let mut bytes = [0; 128];
    let n = initiator
        .write_message(b"forbidden 0-RTT", &mut bytes)
        .unwrap();
    let mut responder = build_with_key(&b, &ap, b"context", false).unwrap();
    assert!(read_handshake(&mut responder, &bytes[..n], 96, &ap).is_err());

    let mut initiator = build_with_key(&a, &bp, b"context", true).unwrap();
    let mut responder = build_with_key(&b, &ap, b"context", false).unwrap();
    let init = write_handshake(&mut initiator, 96).unwrap();
    read_handshake(&mut responder, &init, 96, &ap).unwrap();
    let n = responder
        .write_message(b"forbidden response payload", &mut bytes)
        .unwrap();
    assert!(read_handshake(&mut initiator, &bytes[..n], 48, &bp).is_err());
}

#[test]
fn each_attempt_has_a_fresh_noise_ephemeral() {
    let (a, _) = keypair();
    let (_, bp) = keypair();
    let mut first = build_with_key(&a, &bp, b"same context", true).unwrap();
    let mut second = build_with_key(&a, &bp, b"same context", true).unwrap();
    let first = write_handshake(&mut first, 96).unwrap();
    let second = write_handshake(&mut second, 96).unwrap();
    assert_ne!(&first[..32], &second[..32]);
}

#[test]
fn exact_noise_plaintext_limit_is_enforced() {
    let (a, b) = handshake(b"length boundary");
    let packet = |len| {
        Packet::parse(fixtures::datagram(
            fixtures::address(1),
            fixtures::address(3),
            17,
            fixtures::udp_body(50000, 443, &vec![7; len - 28]),
        ))
        .unwrap()
    };
    let maximum = packet(MAX_PLAINTEXT);
    let facts = maximum.facts();
    let bytes = encrypt(&a, 0, maximum).unwrap();
    assert_eq!(bytes.len(), 65_535);
    decrypt(&b, 0, &bytes, facts).unwrap();
    assert!(encrypt(&a, 1, packet(MAX_PLAINTEXT + 1)).is_err());
}

#[test]
fn successful_handshake_move_keeps_queued_outputs_live_and_crypto_failure_cancels_them() {
    use crate::secure::{State, tests::circuit};
    use tokio_util::sync::CancellationToken;
    for corrupt in [false, true] {
        let (a, ap) = keypair();
        let (b, bp) = keypair();
        let mut initiator = build_with_key(&a, &bp, b"lifecycle", true).unwrap();
        let mut responder = build_with_key(&b, &ap, b"lifecycle", false).unwrap();
        let init = write_handshake(&mut initiator, 96).unwrap();
        read_handshake(&mut responder, &init, 96, &ap).unwrap();
        let mut response = write_handshake(&mut responder, 48).unwrap();
        if corrupt {
            response[0] ^= 1;
        }
        let root = CancellationToken::new();
        let mut circuit = circuit(State::Initiator(Box::new(initiator)), &root);
        let output = circuit.cancellation.clone();
        let result = circuit.complete_handshake(|mut noise| {
            // State::Closed is only a move placeholder here, not cancellation.
            assert!(!output.is_cancelled());
            read_handshake(&mut noise, &response, 48, &bp)?;
            noise
                .into_stateless_transport_mode()
                .map_err(|_| Error::Crypto)
        });
        assert_eq!(result.is_err(), corrupt);
        assert_eq!(output.is_cancelled(), corrupt);
        assert_eq!(matches!(circuit.state, State::Closed), corrupt);
        assert!(!root.is_cancelled());
        if let State::Endpoint(noise) = &circuit.state {
            let responder = responder.into_stateless_transport_mode().unwrap();
            let packet = fixtures::udp(1, 3, 50000, 443);
            let facts = packet.facts();
            let bytes = encrypt(noise, 0, packet).unwrap();
            decrypt(&responder, 0, &bytes, facts).unwrap();
        }
    }
}

fn node(n: u8) -> NodeId {
    format!("00000000-0000-4000-8000-{n:012x}").parse().unwrap()
}
fn bound(n: u8, signing: &SigningKey) -> BoundNodeKey {
    BoundNodeKey {
        id: KeyId::new(node(n).into_uuid()).unwrap(),
        generation: Version::MIN,
        public_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: Digest::from_bytes(signing.verifying_key().to_bytes()),
        },
    }
}

struct ProofFixture {
    signing: [SigningKey; 3],
    keys: BTreeMap<NodeId, BoundNodeKey>,
    selected: LogicalPath,
    frame: Frame,
}
impl ProofFixture {
    fn new(reply: bool) -> Self {
        let signing = [
            SigningKey::from_bytes(&[1; 32]),
            SigningKey::from_bytes(&[2; 32]),
            SigningKey::from_bytes(&[3; 32]),
        ];
        let keys = signing
            .iter()
            .enumerate()
            .map(|(i, key)| (node(i as u8 + 1), bound(i as u8 + 4, key)))
            .collect();
        let selected = LogicalPath {
            nodes: vec![node(1), node(2), node(3)],
            dial_candidates: [
                (node(1), vec![format!("{}/udp", node(2))]),
                (node(2), vec![format!("{}/udp", node(3))]),
            ]
            .into(),
        };
        let facts = fixtures::udp(1, 3, 50000, 443).facts();
        let frame = Frame {
            context: Context {
                network: NetworkId::new(node(10).into_uuid()).unwrap(),
                authorization: meshway_configuration::execution::CircuitAuthorization::Business(
                    "00000000-0000-4000-8000-000000000011".parse().unwrap(),
                ),
                path: selected.nodes.clone(),
                circuit: [1; 32],
                permit: Digest::from_bytes([2; 32]),
                keys: Digest::from_bytes([3; 32]),
            },
            kind: Kind::Data,
            reply,
            sequence: 0,
            facts: Some(facts),
            probe: None,
            body: vec![5; facts.len + 16],
            proofs: vec![],
        };
        let mut fixture = Self {
            signing,
            keys,
            selected,
            frame,
        };
        fixture.sign();
        fixture
    }
    fn sign(&mut self) {
        self.frame.proofs.clear();
        let mut hash = transcript(&self.frame).unwrap();
        for i in 0..2 {
            let position = if self.frame.reply { 2 - i } else { i };
            let edge = if self.frame.reply { 1 - i } else { i };
            let signer = node(position as u8 + 1);
            let key = &self.keys[&signer];
            let mut proof = Proof {
                key: key.id,
                generation: key.generation,
                carrier: Carrier {
                    dialer: node(edge as u8 + 1),
                    listener: node(edge as u8 + 2),
                    listener_key: "udp".into(),
                    protocol: TransportProtocol::NativeUdp,
                },
                signature: [0; 64],
            };
            let digest = proof_digest(&hash, i, &proof).unwrap();
            proof.signature = self.signing[position]
                .sign(&packet_proof_message(
                    self.frame.context.network,
                    signer,
                    key,
                    digest,
                ))
                .to_bytes();
            extend(&mut hash, &proof).unwrap();
            self.frame.proofs.push(proof);
        }
    }
    fn verify(&self) -> Result<NodeId, Error> {
        let local = if self.frame.reply { node(1) } else { node(3) };
        let incoming = &self.frame.proofs.last().ok_or(Error::Proof)?.carrier;
        verify_chain(&self.frame, incoming, local, &self.selected, |node| {
            self.keys.get(&node).ok_or(Error::Proof)
        })
    }
}

#[test]
fn ordered_chain_rejects_missing_reordered_tampered_and_wrong_context_proofs() {
    for reply in [false, true] {
        let mut f = ProofFixture::new(reply);
        assert_eq!(f.verify().unwrap(), node(2));
        f.frame.proofs.remove(0);
        assert!(f.verify().is_err());
        f.sign();
        f.frame.proofs.swap(0, 1);
        assert!(f.verify().is_err());
        f.sign();
        f.frame.body[0] ^= 1;
        assert!(f.verify().is_err());
        f.sign();
        f.frame.context.circuit[0] ^= 1;
        assert!(f.verify().is_err());
        f.sign();
        f.frame.context.permit = Digest::from_bytes([9; 32]);
        assert!(f.verify().is_err());
        f.sign();
        f.frame.context.keys = Digest::from_bytes([19; 32]);
        assert!(f.verify().is_err());
        f.sign();
        f.frame.context.network = NetworkId::new(node(12).into_uuid()).unwrap();
        assert!(f.verify().is_err());
        f.sign();
        f.frame.proofs[0].generation = Version::new(2).unwrap();
        assert!(f.verify().is_err());
        f.sign();
        f.frame.proofs[0].key = bound(42, &f.signing[0]).id;
        assert!(f.verify().is_err());
        f.sign();
        f.frame.proofs[0].carrier.protocol = TransportProtocol::Vless;
        assert!(f.verify().is_err());
        f.sign();
        f.frame.context.path.swap(0, 1);
        assert!(f.verify().is_err());
    }
}

#[test]
fn diagnostic_proofs_bind_grant_kind_measurement_deadline_and_encrypted_challenge() {
    use crate::secure::{CircuitAuthorization, codec::ProbeMetadata};
    for reply in [false, true] {
        for tamper in 0..5 {
            let mut f = ProofFixture::new(reply);
            let CircuitAuthorization::Business(id) = f.frame.context.authorization else {
                unreachable!()
            };
            f.frame.context.authorization = CircuitAuthorization::Diagnostic(
                meshway_model::ObservationId::new(id.into_uuid()).unwrap(),
            );
            f.frame.kind = if reply {
                Kind::ProbeReply
            } else {
                Kind::ProbeRequest
            };
            f.frame.facts = None;
            f.frame.probe = Some(ProbeMetadata {
                measurement_id: meshway_model::EventId::new(id.into_uuid()).unwrap(),
                expires_at_ms: 10_000,
            });
            f.frame.body = vec![7; 48];
            f.sign();
            f.verify().unwrap();
            match tamper {
                0 => f.frame.body[0] ^= 1,
                1 => f.frame.probe.as_mut().unwrap().expires_at_ms += 1,
                2 => {
                    f.frame.probe.as_mut().unwrap().measurement_id =
                        meshway_model::EventId::new(node(55).into_uuid()).unwrap()
                }
                3 => f.frame.context.authorization = CircuitAuthorization::Business(id),
                _ => f.frame.context.path.swap(0, 1),
            }
            assert!(f.verify().is_err());
        }
    }
}

#[test]
fn last_carrier_must_be_actual_and_mandatory_hop_cannot_be_forged() {
    let mut f = ProofFixture::new(false);
    let mut incoming = f.frame.proofs.last().unwrap().carrier.clone();
    incoming.protocol = TransportProtocol::Vless;
    assert!(
        verify_chain(&f.frame, &incoming, node(3), &f.selected, |node| f
            .keys
            .get(&node)
            .ok_or(Error::Proof))
        .is_err()
    );
    // An origin signing with its own key cannot impersonate the relay even
    // when it can supply the complete byte layout and the relay's public key ID.
    let mut hash = transcript(&f.frame).unwrap();
    extend(&mut hash, &f.frame.proofs[0]).unwrap();
    let digest = proof_digest(&hash, 1, &f.frame.proofs[1]).unwrap();
    let message = packet_proof_message(f.frame.context.network, node(2), &f.keys[&node(2)], digest);
    f.frame.proofs[1].signature = f.signing[0].sign(&message).to_bytes();
    assert!(f.verify().is_err());
}
