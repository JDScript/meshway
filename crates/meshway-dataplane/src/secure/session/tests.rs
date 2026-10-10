use super::*;
use crate::{
    fixtures,
    secure::codec::{HopAuth, Kind, ProbeMetadata, Proof},
};
use meshway_model::TransportProtocol;
use meshway_model::{EventId, KeyId, NetworkId, Version};

fn node(n: usize) -> NodeId {
    NodeId::new(uuid::Uuid::from_u128(n as u128 + 1)).unwrap()
}

struct Fixture {
    sessions: Vec<Session>,
    response: Frame,
    selected: LogicalPath,
}

impl Fixture {
    // The handshake signature layer is exercised by crypto/tests and the real
    // Control/runtime tests. Here its verified contributions feed the actual KDF.
    fn pending(n: usize) -> Self {
        let selected = LogicalPath {
            nodes: (0..n).map(node).collect(),
            dial_candidates: (0..n - 1)
                .map(|i| (node(i), vec![format!("{}/udp", node(i + 1))]))
                .collect(),
        };
        let mut sessions: Vec<_> = (0..n).map(|_| Session::new().unwrap()).collect();
        let mut init = Frame {
            context: Arc::new(Context {
                network: NetworkId::new(node(20).into_uuid()).unwrap(),
                grant: crate::secure::CircuitGrant {
                    purpose: crate::Purpose::Business,
                    target: crate::Target::Mesh { node: node(21) },
                },
                path: selected.nodes.clone(),
                circuit: [1; 32],
                authorization: [2; 32].into(),
                keys: [3; 32].into(),
                initiated_at_ms: 0,
            }),
            kind: Kind::Init,
            reply: false,
            sequence: 0,
            probe: None,
            body: vec![7; 96].into(),
            proofs: vec![],
            exchange: None,
            auth: HopAuth::default(),
        };
        for (position, session) in sessions.iter_mut().enumerate() {
            session.remember_init(&init).unwrap();
            if position + 1 < n {
                init.proofs.push(Proof {
                    key: KeyId::new(node(position).into_uuid()).unwrap(),
                    generation: Version::MIN,
                    carrier: carrier(position),
                    ephemeral: session.public(),
                    signature: [0; 64],
                });
            }
        }
        let response = Frame {
            context: init.context,
            kind: Kind::Response,
            reply: true,
            sequence: 0,
            probe: None,
            body: vec![8; 48].into(),
            proofs: vec![],
            exchange: Some(sessions.last().unwrap().exchange().unwrap()),
            auth: HopAuth::default(),
        };
        Self {
            sessions,
            response,
            selected,
        }
    }

    fn establish(&mut self) {
        for (position, session) in self.sessions.iter_mut().enumerate() {
            let keys = session.derive(&self.response, node(position)).unwrap();
            session.establish(keys);
            assert!(session.pending.is_none());
        }
    }

    fn frame(&self, reply: bool) -> Frame {
        let facts = fixtures::udp(1, 3, 50000, 443).facts();
        Frame {
            context: self.response.context.clone(),
            kind: Kind::Data,
            reply,
            sequence: 17,
            probe: None,
            body: vec![9; facts.len + 16].into(),
            proofs: vec![],
            exchange: None,
            auth: HopAuth::default(),
        }
    }
}

fn carrier(edge: usize) -> Carrier {
    Carrier {
        dialer: node(edge),
        listener: node(edge + 1),
        listener_key: "udp".into(),
        protocol: TransportProtocol::NativeUdp,
    }
}

#[test]
fn prekeyed_hmac_matches_fixed_data_probe_and_dns_transcripts() {
    // Independent Python stdlib HMAC-SHA256 vectors: the data transcript is
    // unchanged, while bounded exchanges bind their v6 domain and exact kind.
    // key=42*32, CID=01*32, sequence=2^32, body=a5*(1296 or 48),
    // probe measurement UUID=26 and deadline=10000. Truncate only after HMAC.
    let expected = [
        [
            121, 132, 108, 153, 184, 214, 155, 63, 216, 22, 230, 198, 173, 143, 106, 124,
        ],
        [
            166, 172, 9, 239, 186, 45, 254, 191, 188, 242, 144, 233, 67, 10, 82, 118,
        ],
        [
            153, 71, 218, 144, 183, 148, 181, 165, 211, 147, 63, 225, 101, 41, 234, 187,
        ],
        [
            19, 122, 100, 29, 16, 127, 35, 52, 26, 65, 144, 50, 103, 54, 201, 245,
        ],
        [
            129, 71, 212, 78, 95, 177, 39, 112, 218, 98, 143, 115, 100, 200, 100, 247,
        ],
        [
            139, 10, 154, 79, 165, 121, 66, 115, 242, 37, 164, 71, 105, 82, 241, 156,
        ],
    ];
    let key = HopMac::new_from_slice(&[0x42; 32]).unwrap();
    let fixture = Fixture::pending(3);
    for kind in 0..3 {
        for reply in [false, true] {
            let mut frame = fixture.frame(reply);
            frame.sequence = 1 << 32;
            frame.body = vec![0xa5; if kind == 0 { 1296 } else { 48 }].into();
            if kind != 0 {
                Arc::make_mut(&mut frame.context).grant = crate::secure::CircuitGrant {
                    purpose: if kind == 1 {
                        crate::Purpose::Diagnostic
                    } else {
                        crate::Purpose::Dns
                    },
                    target: crate::Target::Mesh { node: node(24) },
                };
                frame.kind = match (kind, reply) {
                    (1, false) => Kind::ProbeRequest,
                    (1, true) => Kind::ProbeReply,
                    (_, false) => Kind::DnsRequest,
                    (_, true) => Kind::DnsReply,
                };
                frame.probe = Some(ProbeMetadata {
                    measurement_id: EventId::new(node(25).into_uuid()).unwrap(),
                    expires_at_ms: 10_000,
                });
            }
            for _ in 0..2 {
                assert_eq!(
                    authenticate(&key, &frame).unwrap(),
                    expected[kind * 2 + usize::from(reply)]
                );
            }
        }
    }
}

#[test]
fn every_supported_path_and_direction_verifies_adjacent_mac_with_fixed_wire_size() {
    for n in 2..=8 {
        let mut f = Fixture::pending(n);
        f.establish();
        for reply in [false, true] {
            let mut frame = f.frame(reply);
            for i in 0..n - 1 {
                let from = ordered(n, reply, i);
                let to = ordered(n, reply, i + 1);
                let carrier = carrier(from.min(to));
                f.sessions[from]
                    .append(&mut frame, carrier.clone())
                    .unwrap();
                assert!(frame.auth.tag.is_some());
                let bytes = frame.encode().unwrap();
                assert_eq!(bytes.len(), frame.body.len() + 70);
                frame = Frame::decode(&bytes, Some(&frame.context)).unwrap();
                assert_eq!(
                    f.sessions[to]
                        .verify_chain(&frame, &carrier, &f.selected)
                        .unwrap(),
                    node(from)
                );
            }
        }
    }
}

#[test]
fn skipping_mandatory_neighbour_is_rejected_in_both_directions() {
    let mut f = Fixture::pending(4);
    f.establish();
    for reply in [false, true] {
        let mut frame = f.frame(reply);
        let source = ordered(4, reply, 0);
        let target = ordered(4, reply, 2);
        f.sessions[source]
            .append(&mut frame, carrier(if reply { 2 } else { 0 }))
            .unwrap();
        // Even claiming the expected incoming edge cannot turn the origin's
        // adjacent MAC into the skipped neighbour's MAC. v5 does not promise
        // full-prefix proof against colluding nodes beyond this honest verifier.
        let last = carrier(1);
        assert_eq!(
            f.sessions[target].verify_chain(&frame, &last, &f.selected),
            Err(Error::Proof)
        );
    }
}

#[test]
fn packet_tampering_wrong_verifier_and_actual_carrier_are_rejected() {
    let mut f = Fixture::pending(4);
    f.establish();
    for tamper in 0..7 {
        let mut frame = f.frame(false);
        f.sessions[0].append(&mut frame, carrier(0)).unwrap();
        match tamper {
            0 => frame.body[0] ^= 1,
            1 => frame.sequence += 1,
            2 => frame.reply = true,
            3 => {
                frame.body.extend(1).unwrap()[0] = 0;
            }
            4 => frame.auth.tag = None,
            5 => frame.auth.tag.as_mut().unwrap()[0] ^= 1,
            _ => Arc::make_mut(&mut frame.context).circuit[0] ^= 1,
        }
        assert!(
            f.sessions[1]
                .verify_chain(&frame, &carrier(0), &f.selected)
                .is_err()
        );
    }
}

#[test]
fn session_kdf_binds_full_context_and_both_handshake_messages() {
    for tamper in 0..7 {
        let mut f = Fixture::pending(3);
        let keys = f.sessions[0].derive(&f.response, node(0)).unwrap();
        f.sessions[0].establish(keys);
        let mut frame = f.frame(false);
        f.sessions[0].append(&mut frame, carrier(0)).unwrap();
        match tamper {
            0 => Arc::make_mut(&mut f.response.context).authorization = [13; 32].into(),
            1 => Arc::make_mut(&mut f.response.context).keys = [14; 32].into(),
            2 => {
                Arc::make_mut(&mut f.response.context).network =
                    NetworkId::new(node(22).into_uuid()).unwrap()
            }
            3 => {
                Arc::make_mut(&mut f.response.context).grant = crate::secure::CircuitGrant {
                    purpose: crate::Purpose::Business,
                    target: crate::Target::Mesh { node: node(23) },
                }
            }
            4 => f.response.body[0] ^= 1,
            5 => {
                f.response.exchange.as_mut().unwrap().init[0] ^= 1;
                assert!(f.sessions[1].derive(&f.response, node(1)).is_err());
                continue;
            }
            _ => {
                f.response.exchange.as_mut().unwrap().public[0][0] ^= 1;
                assert!(f.sessions[1].derive(&f.response, node(1)).is_err());
                continue;
            }
        }
        let keys = f.sessions[1].derive(&f.response, node(1)).unwrap();
        f.sessions[1].establish(keys);
        assert_eq!(
            f.sessions[1].verify_chain(&frame, &carrier(0), &f.selected),
            Err(Error::Proof)
        );
    }
}

#[test]
fn low_order_and_substituted_ephemeral_contributions_never_establish_keys() {
    for low in [0, 1] {
        let mut f = Fixture::pending(3);
        let mut public = [0; 32];
        public[0] = low;
        f.response.exchange.as_mut().unwrap().public[2] = public;
        assert!(matches!(
            f.sessions[1].derive(&f.response, node(1)),
            Err(Error::Crypto)
        ));
        assert!(matches!(
            f.sessions[2].derive(&f.response, node(2)),
            Err(Error::Proof)
        ));
    }
    let mut f = Fixture::pending(3);
    f.response.exchange.as_mut().unwrap().public[1] = Session::new().unwrap().public();
    assert!(matches!(
        f.sessions[1].derive(&f.response, node(1)),
        Err(Error::Proof)
    ));
}

#[test]
fn diagnostic_mac_binds_measurement_deadline_kind_and_ciphertext() {
    let mut f = Fixture::pending(3);
    Arc::make_mut(&mut f.response.context).grant = crate::secure::CircuitGrant {
        purpose: crate::Purpose::Diagnostic,
        target: crate::Target::Mesh { node: node(24) },
    };
    f.establish();
    for reply in [false, true] {
        for tamper in 0..5 {
            let mut frame = f.frame(reply);
            frame.kind = if reply {
                Kind::ProbeReply
            } else {
                Kind::ProbeRequest
            };
            frame.body = vec![7; 48].into();
            frame.probe = Some(ProbeMetadata {
                measurement_id: EventId::new(node(25).into_uuid()).unwrap(),
                expires_at_ms: 10_000,
            });
            let edge = carrier(if reply { 1 } else { 0 });
            f.sessions[if reply { 2 } else { 0 }]
                .append(&mut frame, edge.clone())
                .unwrap();
            f.sessions[1]
                .verify_chain(&frame, &edge, &f.selected)
                .unwrap();
            match tamper {
                0 => frame.probe.as_mut().unwrap().expires_at_ms += 1,
                1 => {
                    frame.probe.as_mut().unwrap().measurement_id =
                        EventId::new(node(26).into_uuid()).unwrap()
                }
                2 => frame.body[0] ^= 1,
                3 => frame.kind = Kind::Data,
                _ => frame.reply = !reply,
            }
            assert!(
                f.sessions[1]
                    .verify_chain(&frame, &edge, &f.selected)
                    .is_err()
            );
        }
    }
}
