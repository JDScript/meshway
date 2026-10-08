use super::*;

pub(in crate::manager::tests) async fn converged(
    a: &Manager,
    b: &Manager,
    a_id: NodeId,
    b_id: NodeId,
    old: Option<[u8; 32]>,
) {
    let result = tokio::time::timeout(RECOVERY, async {
        loop {
            let af = a.snapshot().unwrap();
            let bf = b.snapshot().unwrap();
            if let Some(left) = af.links.iter().find(|link| link.peer == b_id)
                && let Some(right) = bf.links.iter().find(|link| link.peer == a_id)
                && left.id == right.id
                && Some(left.id) != old
            {
                assert_eq!(left.carrier, right.carrier);
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "reattach before the healthy predecessor expires: {a_id} -> {b_id}; left={:?}; right={:?}",
        a.snapshot(),
        b.snapshot()
    );
}

pub(in crate::manager::tests) fn secure(
    identity: &meshway_authority::DataPlaneIdentity,
    manager: &Manager,
) -> SecureNode {
    SecureNode::new(
        identity.clone(),
        Default::default(),
        manager.carriers().unwrap(),
    )
    .unwrap()
}

pub(in crate::manager::tests) async fn handshake(
    a: &mut Manager,
    b: &mut Manager,
    sa: &mut SecureNode,
    sb: &mut SecureNode,
    demand: meshway_model::RequestId,
    path: Vec<NodeId>,
) -> [u8; 32] {
    let first = sa.start(demand, path).unwrap();
    let cid = first.circuit();
    a.send(first).await.unwrap();
    let (link, packet) = frame(b).await;
    let Received::Established {
        response: Some(response),
        ..
    } = sb.receive(&link.carrier, packet.payload).unwrap()
    else {
        panic!("terminal handshake");
    };
    b.send(response).await.unwrap();
    let (link, packet) = frame(a).await;
    assert!(
        matches!(sa.receive(&link.carrier, packet.payload).unwrap(), Received::Established { circuit, .. } if circuit == cid)
    );
    cid
}

pub(in crate::manager::tests) fn packet(fixture: &fixture::Fixture, reply: bool) -> Packet {
    packet_between(fixture, 0, 1, reply)
}

pub(in crate::manager::tests) fn packet_between(
    fixture: &fixture::Fixture,
    source: usize,
    target: usize,
    reply: bool,
) -> Packet {
    let address = |index: usize| {
        fixture.identities[index]
            .snapshot()
            .configuration()
            .unwrap()
            .addresses
            .ipv4
            .unwrap()
    };
    let (source, destination, sport, dport) = if reply {
        (address(target), address(source), 42424u16, 50000u16)
    } else {
        (address(source), address(target), 50000u16, 42424u16)
    };
    let payload = b"same protected circuit after UDP reattachment";
    let mut bytes = vec![0; 28];
    bytes[0] = 0x45;
    bytes[2..4].copy_from_slice(&u16::try_from(28 + payload.len()).unwrap().to_be_bytes());
    bytes[8] = 64;
    bytes[9] = 17;
    bytes[12..16].copy_from_slice(&source.octets());
    bytes[16..20].copy_from_slice(&destination.octets());
    let sum = bytes[..20]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u32::from(u16::from_be_bytes([pair[0], pair[1]])))
        .sum::<u32>();
    let folded = (sum & 65535) + (sum >> 16);
    let checksum = !(((folded & 65535) + (folded >> 16)) as u16);
    bytes[10..12].copy_from_slice(&checksum.to_be_bytes());
    bytes[20..22].copy_from_slice(&sport.to_be_bytes());
    bytes[22..24].copy_from_slice(&dport.to_be_bytes());
    bytes[24..26].copy_from_slice(&u16::try_from(8 + payload.len()).unwrap().to_be_bytes());
    // IPv4 permits an omitted UDP checksum; production Packet parses the wire.
    bytes.extend(payload);
    Packet::parse(bytes).unwrap()
}

pub(in crate::manager::tests) async fn exchange(
    from: &Manager,
    to: &mut Manager,
    source: &mut SecureNode,
    target: &mut SecureNode,
    cid: [u8; 32],
    packet: Packet,
) {
    let expected = packet.as_bytes().to_vec();
    from.send(source.send(cid, packet).unwrap()).await.unwrap();
    let (link, received) = frame(to).await;
    let Received::Packet(pending) = target.receive(&link.carrier, received.payload).unwrap() else {
        panic!("business packet");
    };
    pending
        .release(|action| match action {
            Action::Deliver { packet, .. } => assert_eq!(packet.as_bytes(), expected),
            _ => panic!("endpoint delivery"),
        })
        .unwrap();
}

pub(in crate::manager::tests) async fn control(socket: &UdpSocket, kind: Kind) -> Vec<u8> {
    tokio::time::timeout(RECOVERY, async {
        let mut bytes = vec![0; adjacency::MAX_MESSAGE];
        loop {
            let (size, _) = socket.recv_from(&mut bytes).await.unwrap();
            if let Ok((actual, body)) = adjacency::decode(&bytes[..size])
                && actual == kind
            {
                return body.to_vec();
            }
        }
    })
    .await
    .unwrap()
}

pub(in crate::manager::tests) async fn challenge(
    fixture: &fixture::Fixture,
    socket: &UdpSocket,
    listener: SocketAddr,
    key: &str,
) -> (Transcript, Vec<u8>, Vec<u8>) {
    let ids = fixture.ids();
    let carrier = Carrier {
        dialer: ids[0],
        listener: ids[1],
        listener_key: key.into(),
        protocol: TransportProtocol::NativeUdp,
    };
    let (initiation, intro) = Initiation::new(&fixture.identities[0], &carrier).unwrap();
    let init = adjacency::encode(Kind::Init, &intro).unwrap();
    socket.send_to(&init, listener).await.unwrap();
    let response = control(socket, Kind::Challenge).await;
    let transcript = initiation
        .challenge(
            fixture.identities[0].snapshot().configuration().unwrap(),
            ids[1],
            &response,
        )
        .unwrap();
    let ack = adjacency::encode(
        Kind::Ack,
        &transcript
            .message(&fixture.identities[0], Kind::Ack)
            .unwrap(),
    )
    .unwrap();
    (transcript, init, ack)
}

pub(in crate::manager::tests) async fn selected(manager: &Manager, peer: NodeId, id: [u8; 32]) {
    tokio::time::timeout(RECOVERY, async {
        loop {
            if manager
                .snapshot()
                .unwrap()
                .links
                .iter()
                .any(|link| link.peer == peer && link.id == id)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

pub(in crate::manager::tests) async fn route_packet(
    managers: &mut [Manager],
    nodes: &mut [SecureNode],
    cid: [u8; 32],
    reply: bool,
    packet: Packet,
) {
    let (from, to) = if reply { (1, 0) } else { (0, 1) };
    let expected = packet.as_bytes().to_vec();
    managers[from]
        .send(nodes[from].send(cid, packet).unwrap())
        .await
        .unwrap();
    let (link, data) = frame(&mut managers[2]).await;
    let Received::Forward(pending) = nodes[2].receive(&link.carrier, data.payload).unwrap() else {
        panic!("relay");
    };
    managers[2].send(pending).await.unwrap();
    let (link, data) = frame(&mut managers[to]).await;
    let Received::Packet(pending) = nodes[to].receive(&link.carrier, data.payload).unwrap() else {
        panic!("endpoint");
    };
    pending
        .release(|action| match action {
            Action::Deliver { packet, .. } => assert_eq!(packet.as_bytes(), expected),
            _ => panic!("exact-path delivery"),
        })
        .unwrap();
}

pub(in crate::manager::tests) async fn all_links(managers: &[Manager], ids: &[NodeId]) {
    for (a, b) in [(0, 2), (2, 1)] {
        converged(&managers[a], &managers[b], ids[a], ids[b], None).await;
    }
}
