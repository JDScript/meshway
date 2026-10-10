use meshway_dataplane::{
    Action, Packet,
    secure::{PendingFrame, Received, SecureNode},
};
use meshway_static_authority::fixture::{Chain, tcp_ipv4};
use std::time::Duration;

fn grant(chain: &Chain) -> meshway_dataplane::secure::CircuitGrant {
    meshway_dataplane::secure::CircuitGrant {
        purpose: meshway_dataplane::Purpose::Business,
        target: chain.target.clone(),
    }
}

fn nodes(chain: &Chain) -> Vec<SecureNode> {
    chain
        .path
        .iter()
        .enumerate()
        .map(|(i, id)| {
            SecureNode::new(
                chain.authority.identity(*id).unwrap(),
                Default::default(),
                chain.carriers_for(i).unwrap(),
            )
            .unwrap()
        })
        .collect()
}

fn drive(chain: &Chain, nodes: &mut [SecureNode], mut pending: PendingFrame) -> Received {
    for _ in 0..16 {
        let next = chain
            .path
            .iter()
            .position(|id| *id == pending.next())
            .unwrap();
        let carrier = pending.carrier().clone();
        let received = pending
            .release(|bytes| nodes[next].receive(&carrier, bytes))
            .unwrap()
            .unwrap();
        pending = match received {
            Received::Forward(frame)
            | Received::Established {
                response: Some(frame),
                ..
            } => frame,
            done => return done,
        };
    }
    panic!("bounded chain did not finish")
}

#[test]
fn real_compiler_noise_mac_tcp_roundtrip_and_revocation() {
    let chain = Chain::new(3, Duration::from_secs(60)).unwrap();
    let mut nodes = nodes(&chain);
    let init = nodes[0]
        .start_grant(grant(&chain), chain.path.clone(), &chain.carriers[0])
        .unwrap();
    let circuit = init.circuit();
    assert!(matches!(
        drive(&chain, &mut nodes, init),
        Received::Established { response: None, .. }
    ));
    for (reply, seq, ack, flags, length) in [
        (false, 100, 0, 2, 52),
        (true, 200, 101, 0x12, 52),
        (false, 101, 201, 0x10, 52),
        (false, 101, 201, 0x18, 1280),
        (true, 201, 1329, 0x18, 1280),
    ] {
        let (from, to) = if reply { (2, 0) } else { (0, 2) };
        let bytes = tcp_ipv4(
            chain.addresses[from],
            chain.addresses[to],
            reply,
            seq,
            ack,
            flags,
            length,
        )
        .unwrap();
        let pending = nodes[from]
            .send(circuit, Packet::parse(bytes.clone()).unwrap())
            .unwrap();
        let Received::Packet(pending) = drive(&chain, &mut nodes, pending) else {
            panic!("not delivered")
        };
        pending
            .release(|action| {
                let Action::Deliver { packet, route } = action else {
                    panic!("not a delivery")
                };
                assert_eq!(packet.as_bytes(), bytes);
                assert_eq!(route.original_path, chain.path);
            })
            .unwrap();
        assert_eq!(nodes[1].retained_flows(), 0);
    }
    let bytes = tcp_ipv4(
        chain.addresses[0],
        chain.addresses[2],
        false,
        1329,
        1429,
        0x10,
        52,
    )
    .unwrap();
    let pending = nodes[0]
        .send(circuit, Packet::parse(bytes.clone()).unwrap())
        .unwrap();
    let snapshot = chain.authority.snapshot(chain.path[0]).unwrap();
    chain.authority.revoke(chain.path[0]).unwrap();
    assert!(snapshot.ensure_current().is_err());
    assert!(
        pending
            .release(|_| panic!("revoked output released"))
            .is_err()
    );
    assert!(
        nodes[0]
            .send(circuit, Packet::parse(bytes).unwrap())
            .is_err()
    );
    assert!(chain.authority.identity(chain.path[0]).is_err());
}

#[test]
fn lease_deadline_and_issuer_drop_invalidate_clones() {
    let chain = Chain::new(2, Duration::from_secs(1)).unwrap();
    let identity = chain.authority.identity(chain.path[0]).unwrap();
    let mut running = nodes(&chain);
    let pending = running[0]
        .start_grant(grant(&chain), chain.path.clone(), &chain.carriers[0])
        .unwrap();
    chain.authority.wait_expired(chain.path[0]).unwrap();
    assert!(
        pending
            .release(|_| panic!("expired handshake released"))
            .is_err()
    );
    assert!(
        running[0]
            .start_grant(grant(&chain), chain.path.clone(), &chain.carriers[0])
            .is_err()
    );
    assert!(SecureNode::new(identity, Default::default(), chain.carriers_for(0).unwrap()).is_err());
    let chain = Chain::new(2, Duration::from_secs(60)).unwrap();
    let snapshot = chain.authority.snapshot(chain.path[0]).unwrap();
    drop(chain);
    assert!(snapshot.ensure_current().is_err());
}

#[test]
fn revoked_relay_rejects_authenticated_incoming_handshake() {
    let chain = Chain::new(3, Duration::from_secs(60)).unwrap();
    let mut nodes = nodes(&chain);
    let init = nodes[0]
        .start_grant(grant(&chain), chain.path.clone(), &chain.carriers[0])
        .unwrap();
    chain.authority.revoke(chain.path[1]).unwrap();
    assert!(
        init.release(|bytes| nodes[1].receive(&chain.carriers[0], bytes))
            .unwrap()
            .is_err()
    );
}
