use super::*;

fn lookup(scene: &Scene, local: u8, packet: Packet) -> Result<Option<LocalFlow>, Error> {
    scene.tables[&node(local)]
        .local_flow(node(local), &packet.facts(), scene.time())
        .map(|found| found.map(|found| found.route))
}

fn original() -> LocalFlow {
    LocalFlow::new(
        meshway_policy::compile::netmap::Target::Mesh { node: node(3) },
        vec![node(1), node(2), node(3)],
        None,
    )
}

#[test]
fn local_lookup_returns_original_origin_and_terminal_routes_but_never_relay_state() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    assert_eq!(lookup(&scene, 1, udp(1, 3, 50000, 443)).unwrap(), None);
    forward(&mut scene, udp(1, 3, 50000, 443));
    assert_eq!(
        lookup(&scene, 1, udp(1, 3, 50000, 443)).unwrap(),
        Some(original())
    );
    assert_eq!(
        lookup(&scene, 3, udp(3, 1, 443, 50000)).unwrap(),
        Some(original())
    );
    // The relay forwards the session and holds no business flow.
    assert_eq!(lookup(&scene, 2, udp(1, 3, 50000, 443)).unwrap(), None);
    assert_eq!(lookup(&scene, 2, udp(3, 1, 443, 50000)).unwrap(), None);
    assert!(matches!(
        lookup(&scene, 1, udp(3, 1, 443, 50000)),
        Err(Error::Flow)
    ));
    assert!(matches!(
        lookup(&scene, 3, udp(1, 3, 50000, 443)),
        Err(Error::Flow)
    ));
    assert_eq!(lookup(&scene, 1, udp(1, 3, 50001, 443)).unwrap(), None);
}

#[test]
fn local_lookup_shares_initial_protocol_validation_and_exact_echo_identifiers() {
    let scene = Scene::new(BusinessProtocol::Tcp, false, true);
    assert_eq!(lookup(&scene, 1, tcp(false, 100, 0, 2, 0)).unwrap(), None);
    // An origin ACK resumes an idle-reclaimed flow as a first packet.
    assert_eq!(
        lookup(&scene, 1, tcp(false, 101, 901, 0x10, 0)).unwrap(),
        None
    );
    for packet in [
        tcp(true, 900, 101, 0x12, 0),
        tcp(false, 100, 0, 2, 1),
        tcp(false, 100, 1, 2, 0),
    ] {
        assert!(matches!(lookup(&scene, 1, packet), Err(Error::Flow)));
    }
    let mut scene = Scene::new(BusinessProtocol::IcmpEcho, false, true);
    assert!(matches!(
        lookup(&scene, 3, echo(true, 12, 34)),
        Err(Error::Flow)
    ));
    forward(&mut scene, echo(false, 12, 34));
    assert_eq!(
        lookup(&scene, 3, echo(true, 12, 34)).unwrap(),
        Some(original())
    );
    assert!(matches!(
        lookup(&scene, 3, echo(true, 13, 34)),
        Err(Error::Flow)
    ));
    assert!(matches!(
        lookup(&scene, 3, echo(true, 12, 35)),
        Err(Error::Flow)
    ));
    assert_eq!(lookup(&scene, 1, echo(false, 12, 35)).unwrap(), None);
}

#[test]
fn local_lookup_never_advances_tcp_state_or_refreshes_deadlines() {
    let mut scene = Scene::new(BusinessProtocol::Tcp, false, true);
    forward(&mut scene, tcp(false, 100, 0, 2, 0));
    reply(&mut scene, tcp(true, 900, 101, 0x12, 0));
    forward(&mut scene, tcp(false, 101, 901, 0x10, 0));
    let before = scene.tables[&node(1)]
        .entries
        .values()
        .next()
        .unwrap()
        .clone();
    scene.advance(Duration::from_secs(1));
    for _ in 0..2 {
        assert_eq!(
            lookup(&scene, 1, tcp(false, 101, 901, 0x18, 7)).unwrap(),
            Some(original())
        );
    }
    let after = scene.tables[&node(1)].entries.values().next().unwrap();
    assert_eq!(after.cap, before.cap);
    assert!(matches!(
        scene.process(1, Some(2), 3, &[1, 2, 3], tcp(true, 901, 108, 0x10, 0)),
        Err(Error::Flow)
    ));
    forward(&mut scene, tcp(false, 101, 901, 0x18, 7));
}

#[test]
fn ambiguous_retained_metadata_fails_closed() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    forward(&mut scene, udp(1, 3, 50000, 443));
    let table = scene.tables.get_mut(&node(1)).unwrap();
    let (key, flow) = table.entries.first_key_value().unwrap();
    let mut duplicate = key.clone();
    let flow = flow.clone();
    duplicate.path = vec![node(1), node(3)];
    table.insert(duplicate, flow);
    assert!(matches!(
        lookup(&scene, 1, udp(1, 3, 50000, 443)),
        Err(Error::Flow)
    ));
}
