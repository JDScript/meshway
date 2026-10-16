use super::*;

/// A peer that roamed: its signed Ping of the existing channel arrives
/// from a new source. The owner challenges that source with a fresh Ping
/// of the same channel and moves the channel there only on the peer's
/// Pong over that nonce from it: same channel identity and boot, new
/// remote. An unsolicited Pong or an unanswered challenge moves nothing.
#[tokio::test]
async fn a_roaming_peer_is_followed_only_after_its_new_source_answers_a_challenge() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let (shared, _receiver) = shared(&fixture.desired[1], &fixture.keys[1]);
    let socket = endpoint(fixture.take_udp(1)).await;
    let local = socket.local_addr().unwrap();
    let id = adjacency::nonce().unwrap();
    let mut owner = Owner::new(shared.clone(), Box::new(socket), id).unwrap();
    owner.listener = Some(ListenerFact {
        socket_id: id,
        key: "udp".into(),
        protocol: TransportProtocol::NativeUdp,
        local,
        buffers: None,
    });
    let dialer = Plan::new(&fixture.desired[0]).unwrap();
    let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
    let old = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let (initiation, intro) =
        Initiation::new(&fixture.keys[0], &dialer, &carrier, &DIALER_BOOT).unwrap();
    owner.init(old.local_addr().unwrap(), &intro).await.unwrap();
    let mut bytes = [0; adjacency::MAX_MESSAGE];
    let (len, _) = control(&old, &mut bytes).await;
    let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
    let transcript = initiation.challenge(&dialer, ids[1], challenge).unwrap();
    let ack = transcript
        .message(&fixture.keys[0], &dialer, Kind::Ack)
        .unwrap();
    owner.ack(old.local_addr().unwrap(), &ack).await.unwrap();
    let channel = owner.channels[&old.local_addr().unwrap()].fact.clone();
    let ping = |nonce: &[u8; 32]| {
        transcript
            .health(&fixture.keys[0], &dialer, Kind::Ping, nonce)
            .unwrap()
    };
    // An unsolicited Pong from anywhere changes nothing.
    let moved = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let source = moved.local_addr().unwrap();
    let pong = transcript
        .health(
            &fixture.keys[0],
            &dialer,
            Kind::Pong,
            &adjacency::nonce().unwrap(),
        )
        .unwrap();
    owner.health(source, Kind::Pong, &pong).await.unwrap();
    assert!(owner.channels.contains_key(&old.local_addr().unwrap()));
    // A Ping from the new source: the source is challenged, not trusted.
    owner
        .health(source, Kind::Ping, &ping(&adjacency::nonce().unwrap()))
        .await
        .unwrap();
    assert!(owner.channels.contains_key(&old.local_addr().unwrap()));
    assert!(!owner.channels.contains_key(&source));
    let nonce = loop {
        let (len, from) = control(&moved, &mut bytes).await;
        assert_eq!(from, local);
        let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
        if kind == Kind::Ping {
            break transcript
                .verify_health(&dialer, ids[1], Kind::Ping, body)
                .unwrap();
        }
    };
    // A Pong over another nonce, or from another source, moves nothing.
    let wrong = transcript
        .health(
            &fixture.keys[0],
            &dialer,
            Kind::Pong,
            &adjacency::nonce().unwrap(),
        )
        .unwrap();
    owner.health(source, Kind::Pong, &wrong).await.unwrap();
    let answer = transcript
        .health(&fixture.keys[0], &dialer, Kind::Pong, &nonce)
        .unwrap();
    let elsewhere = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    owner
        .health(elsewhere.local_addr().unwrap(), Kind::Pong, &answer)
        .await
        .unwrap();
    assert!(owner.channels.contains_key(&old.local_addr().unwrap()));
    // The answer from the challenged source moves the same channel.
    owner.health(source, Kind::Pong, &answer).await.unwrap();
    assert!(!owner.channels.contains_key(&old.local_addr().unwrap()));
    let followed = &owner.channels[&source].fact;
    assert_eq!(followed.id, channel.id);
    assert_eq!(followed.peer_boot, channel.peer_boot);
    assert_eq!(followed.remote, source);
    let link = shared.links.lock().unwrap()[&ids[0]].fact.clone();
    assert_eq!((link.id, link.remote), (channel.id, source));
    owner.channels[&source].scope.check().unwrap();
    owner.shutdown().await.unwrap();
    fixture.close().await;
}

/// After a path change a channel this Node can acquire again must answer
/// its roaming probes within `ROAM_ANSWER`, or it ends at once rather than
/// after the health window; an answered one stays. An inbound channel is
/// the peer's to dial again and only keeps probing.
#[tokio::test]
async fn an_unanswered_roam_ends_an_acquirable_channel_and_keeps_an_inbound_one() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let (shared, _receiver) = shared(&fixture.desired[1], &fixture.keys[1]);
    let socket = endpoint(fixture.take_udp(1)).await;
    let local = socket.local_addr().unwrap();
    let id = adjacency::nonce().unwrap();
    let mut owner = Owner::new(shared.clone(), Box::new(socket), id).unwrap();
    owner.listener = Some(ListenerFact {
        socket_id: id,
        key: "udp".into(),
        protocol: TransportProtocol::NativeUdp,
        local,
        buffers: None,
    });
    let dialer = Plan::new(&fixture.desired[0]).unwrap();
    let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
    let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let remote = peer.local_addr().unwrap();
    let (initiation, intro) =
        Initiation::new(&fixture.keys[0], &dialer, &carrier, &DIALER_BOOT).unwrap();
    owner.init(remote, &intro).await.unwrap();
    let mut bytes = [0; adjacency::MAX_MESSAGE];
    let (len, _) = control(&peer, &mut bytes).await;
    let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
    let transcript = initiation.challenge(&dialer, ids[1], challenge).unwrap();
    let ack = transcript
        .message(&fixture.keys[0], &dialer, Kind::Ack)
        .unwrap();
    owner.ack(remote, &ack).await.unwrap();
    assert_eq!(
        owner.channels[&remote].fact.acquisition,
        LinkAcquisition::AcceptedInbound
    );
    owner.roamed();
    assert!(owner.channels[&remote].roam_until.is_none(), "inbound");
    // The same channel as one this Node dialed: answered probes keep it.
    owner.channels.get_mut(&remote).unwrap().fact.acquisition = LinkAcquisition::PublishedEndpoint;
    owner.roamed();
    assert!(owner.channels[&remote].roam_until.is_some());
    owner.tick().await.unwrap();
    let nonce = loop {
        let (len, _) = control(&peer, &mut bytes).await;
        let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
        if kind == Kind::Ping {
            break transcript
                .verify_health(&dialer, ids[1], Kind::Ping, body)
                .unwrap();
        }
    };
    let pong = transcript
        .health(&fixture.keys[0], &dialer, Kind::Pong, &nonce)
        .unwrap();
    owner.health(remote, Kind::Pong, &pong).await.unwrap();
    assert!(owner.channels[&remote].roam_until.is_none(), "answered");
    // Unanswered past the bound: the channel and its link end now.
    owner.roamed();
    owner.channels.get_mut(&remote).unwrap().roam_until = Some(Instant::now());
    owner.channels[&remote].scope.check().unwrap();
    owner.tick().await.unwrap();
    assert!(!owner.channels.contains_key(&remote));
    assert!(!shared.links.lock().unwrap().contains_key(&ids[0]));
    owner.shutdown().await.unwrap();
    fixture.close().await;
}

/// A real UDP endpoint with a fixed 1500-byte route that records the
/// send grants of roaming peers, at most four live of each kind (the
/// Underlay's bound).
struct Granting<T> {
    inner: T,
    challenges: Grants,
    sources: Grants,
}

type Grants = Arc<Mutex<Vec<(SocketAddr, Arc<dyn meshway_transport::SendLifetime>)>>>;

fn grant(
    grants: &Grants,
    peer: SocketAddr,
    lifetime: Arc<dyn meshway_transport::SendLifetime>,
) -> std::io::Result<()> {
    let mut grants = grants.lock().unwrap();
    grants.retain(|(address, lifetime)| lifetime.is_current() && *address != peer);
    if grants.len() >= 4 {
        return Err(std::io::ErrorKind::QuotaExceeded.into());
    }
    grants.push((peer, lifetime));
    Ok(())
}

fn live(grants: &Grants) -> Vec<SocketAddr> {
    grants
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, lifetime)| lifetime.is_current())
        .map(|(address, _)| *address)
        .collect()
}

impl<T: Datagram> Datagram for Granting<T> {
    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.inner.local_addr()
    }
    fn poll_send_to(
        &self,
        cx: &mut std::task::Context<'_>,
        payload: &[u8],
        peer: SocketAddr,
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.inner.poll_send_to(cx, payload, peer)
    }
    fn poll_recv_from(
        &self,
        cx: &mut std::task::Context<'_>,
        buffer: &mut [u8],
    ) -> std::task::Poll<std::io::Result<meshway_transport::raw::DatagramRead>> {
        self.inner.poll_recv_from(cx, buffer)
    }
    fn path_mtu(&self, _peer: SocketAddr) -> Option<u16> {
        Some(1500)
    }
    fn authorize_challenge(
        &self,
        peer: SocketAddr,
        lifetime: Arc<dyn meshway_transport::SendLifetime>,
    ) -> std::io::Result<()> {
        grant(&self.challenges, peer, lifetime)
    }
    fn authorize_source(
        &self,
        peer: SocketAddr,
        lifetime: Arc<dyn meshway_transport::SendLifetime>,
    ) -> std::io::Result<()> {
        grant(&self.sources, peer, lifetime)
    }
}

/// One accepted channel from node 0 over a [`Granting`] endpoint.
struct Roaming {
    fixture: Netmaps,
    shared: Arc<Shared>,
    owner: Owner,
    transcript: Transcript,
    dialer: Arc<Plan>,
    sources: Grants,
    peer: UdpSocket,
}

async fn roaming() -> Roaming {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let (shared, _receiver) = shared(&fixture.desired[1], &fixture.keys[1]);
    let socket = endpoint(fixture.take_udp(1)).await;
    let local = socket.local_addr().unwrap();
    let sources = Grants::default();
    let endpoint = Granting {
        inner: socket,
        challenges: Grants::default(),
        sources: sources.clone(),
    };
    let id = adjacency::nonce().unwrap();
    let mut owner = Owner::new(shared.clone(), Box::new(endpoint), id).unwrap();
    owner.listener = Some(ListenerFact {
        socket_id: id,
        key: "udp".into(),
        protocol: TransportProtocol::NativeUdp,
        local,
        buffers: None,
    });
    let dialer = Plan::new(&fixture.desired[0]).unwrap();
    let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
    let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let (initiation, intro) =
        Initiation::new(&fixture.keys[0], &dialer, &carrier, &DIALER_BOOT).unwrap();
    owner
        .init(peer.local_addr().unwrap(), &intro)
        .await
        .unwrap();
    let mut bytes = [0; adjacency::MAX_MESSAGE];
    let (len, _) = control(&peer, &mut bytes).await;
    let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
    let transcript = initiation.challenge(&dialer, ids[1], challenge).unwrap();
    let ack = transcript
        .message(&fixture.keys[0], &dialer, Kind::Ack)
        .unwrap();
    owner.ack(peer.local_addr().unwrap(), &ack).await.unwrap();
    Roaming {
        fixture,
        shared,
        owner,
        transcript,
        dialer,
        sources,
        peer,
    }
}

/// F6a B3: a channel follows its peer through six moves. Each move
/// grants its new source and revokes the previous one, so one live grant
/// remains (the endpoint's bound of four is never reached).
#[tokio::test]
async fn a_channel_follows_six_moves_holding_one_source_grant() {
    let Roaming {
        fixture,
        mut owner,
        transcript,
        dialer,
        sources,
        peer,
        ..
    } = roaming().await;
    let ids = fixture.ids();
    let id = owner.channels[&peer.local_addr().unwrap()].fact.id;
    let mut bytes = [0; adjacency::MAX_MESSAGE];
    let mut sockets = vec![peer];
    for step in 1..=6 {
        let moved = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let source = moved.local_addr().unwrap();
        let ping = transcript
            .health(
                &fixture.keys[0],
                &dialer,
                Kind::Ping,
                &adjacency::nonce().unwrap(),
            )
            .unwrap();
        owner.health(source, Kind::Ping, &ping).await.unwrap();
        let nonce = loop {
            let (len, _) = control(&moved, &mut bytes).await;
            let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
            if kind == Kind::Ping {
                break transcript
                    .verify_health(&dialer, ids[1], Kind::Ping, body)
                    .unwrap();
            }
        };
        let answer = transcript
            .health(&fixture.keys[0], &dialer, Kind::Pong, &nonce)
            .unwrap();
        owner.health(source, Kind::Pong, &answer).await.unwrap();
        assert_eq!(owner.channels[&source].fact.id, id, "move {step} followed");
        assert_eq!(live(&sources), vec![source], "move {step}: one grant");
        sockets.push(moved);
    }
    owner.shutdown().await.unwrap();
    assert!(
        live(&sources).is_empty(),
        "the channel's end revokes its grant"
    );
    fixture.close().await;
}

/// F6a B4: this Node's own path change (a roam, or a STUN remap) starts
/// every channel's size search again and withdraws the old measurement
/// at once: the new path may be smaller while the route MTU stays.
#[tokio::test]
async fn an_own_path_change_searches_the_path_size_again() {
    let Roaming {
        fixture,
        shared,
        mut owner,
        transcript,
        dialer,
        peer,
        ..
    } = roaming().await;
    let ids = fixture.ids();
    let remote = peer.local_addr().unwrap();
    let id = owner.channels[&remote].fact.id;
    let mut bytes = vec![0; 65_536];
    let probe = async |owner: &mut Owner, bytes: &mut Vec<u8>| {
        owner.tick().await.unwrap();
        loop {
            let (len, _) = control(&peer, bytes).await;
            let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
            if kind == Kind::Probe {
                break (
                    len,
                    transcript.verify_probe(&dialer, ids[1], len, body).unwrap(),
                );
            }
        }
    };
    let (len, nonce) = probe(&mut owner, &mut bytes).await;
    assert_eq!(len, 1500 - 28, "the route MTU first");
    let pong = transcript
        .health(&fixture.keys[0], &dialer, Kind::Pong, &nonce)
        .unwrap();
    owner.health(remote, Kind::Pong, &pong).await.unwrap();
    assert_eq!(
        shared
            .path_mtu
            .lock()
            .unwrap()
            .get(&id)
            .map(|known| known.1),
        Some(1500)
    );
    owner.roamed();
    assert!(
        !shared.path_mtu.lock().unwrap().contains_key(&id),
        "the old path's size is withdrawn"
    );
    let (len, _) = probe(&mut owner, &mut bytes).await;
    assert_eq!(
        len,
        1500 - 28,
        "searched again now, not after the raise timer"
    );
    owner.shutdown().await.unwrap();
    fixture.close().await;
}
