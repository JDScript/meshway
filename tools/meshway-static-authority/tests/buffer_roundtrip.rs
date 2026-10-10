//! The same allocation crosses the real source/relay/terminal path. Revocation
//! drops the guarded plaintext/ciphertext allocation back into its bounded pool.
use meshway_dataplane::{
    Action, DEFAULT_HEADROOM, DEFAULT_TAILROOM, Packet, PacketPool,
    secure::{Received, SecureNode},
};
use meshway_static_authority::fixture::{Chain, tcp_ipv4};
use std::time::Duration;

fn grant(chain: &Chain) -> meshway_dataplane::secure::CircuitGrant {
    meshway_dataplane::secure::CircuitGrant {
        purpose: meshway_dataplane::Purpose::Business,
        target: chain.target.clone(),
    }
}

#[test]
fn pooled_packet_survives_three_roles_without_payload_copy_and_revocation_returns_it() {
    let chain = Chain::new(3, Duration::from_secs(60)).unwrap();
    let mut nodes: Vec<_> = chain
        .path
        .iter()
        .enumerate()
        .map(|(index, id)| {
            SecureNode::new(
                chain.authority.identity(*id).unwrap(),
                Default::default(),
                chain.carriers_for(index).unwrap(),
            )
            .unwrap()
        })
        .collect();
    let mut pending = nodes[0].start_grant(grant(&chain), chain.path.clone(), &chain.carriers[0]).unwrap();
    let cid = pending.circuit();
    loop {
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
        match received {
            Received::Forward(frame)
            | Received::Established {
                response: Some(frame),
                ..
            } => pending = frame,
            Received::Established { response: None, .. } => break,
            _ => panic!("unexpected handshake result"),
        }
    }
    let pool = PacketPool::new(1, 1280, DEFAULT_HEADROOM, DEFAULT_TAILROOM).unwrap();
    let plaintext = tcp_ipv4(chain.addresses[0], chain.addresses[2], false, 100, 0, 2, 52).unwrap();
    let mut buffer = pool.acquire(plaintext.len()).unwrap();
    buffer.copy_from_slice(&plaintext);
    let pointer = buffer.as_ptr() as usize;
    let borrowed = Packet::parse(buffer.payload()).unwrap();
    assert_eq!(borrowed.as_bytes().as_ptr() as usize, pointer);
    let packet = Packet::parse(buffer).unwrap();
    let outgoing = nodes[0].send(cid, packet).unwrap();
    assert!(pool.acquire(52).is_err());
    let Received::Forward(outgoing) = outgoing
        .release(|wire| {
            assert_eq!(wire.as_ptr() as usize + 54, pointer);
            assert_eq!(wire.len(), plaintext.len() + 86);
            nodes[1].receive(&chain.carriers[0], wire)
        })
        .unwrap()
        .unwrap()
    else {
        panic!("relay must forward");
    };
    assert_eq!(nodes[1].retained_flows(), 0);
    let Received::Packet(delivery) = outgoing
        .release(|wire| {
            assert_eq!(wire.as_ptr() as usize + 54, pointer);
            nodes[2].receive(&chain.carriers[1], wire)
        })
        .unwrap()
        .unwrap()
    else {
        panic!("terminal must decrypt");
    };
    delivery
        .release(|action| {
            let Action::Deliver { packet, .. } = action else {
                panic!("wrong endpoint");
            };
            assert_eq!(packet.as_bytes().as_ptr() as usize, pointer);
            assert_eq!(packet.as_bytes(), plaintext);
        })
        .unwrap();
    let mut buffer = pool.acquire(52).unwrap();
    assert!(buffer.iter().all(|byte| *byte == 0));
    buffer.copy_from_slice(&plaintext);
    let output = nodes[0].send(cid, Packet::parse(buffer).unwrap()).unwrap();
    chain.authority.revoke(chain.path[0]).unwrap();
    assert!(
        output
            .release(|_| panic!("revoked buffer released"))
            .is_err()
    );
    let buffer = pool.acquire(52).unwrap();
    assert!(buffer.iter().all(|byte| *byte == 0));
}
