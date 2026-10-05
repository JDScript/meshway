use super::*;

fn lookup(scene: &Scene, local: u8, packet: Packet) -> Result<Option<LocalFlow>, Error> {
    scene.tables[&node(local)]
        .local_flow(
            &scene.configs[&node(local)],
            &packet.facts(),
            FlowTime {
                instant: scene.instant,
                now_ms: scene.now_ms,
                guard_expires_at: scene.guard_expires_at,
                guard_expires_at_ms: scene.guard_expires_at_ms,
            },
        )
        .map(|found| found.map(|found| found.route))
}

fn original() -> LocalFlow {
    LocalFlow {
        demand: request(),
        path: vec![node(1), node(2), node(3)],
        circuit: None,
    }
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
    assert!(matches!(
        lookup(&scene, 2, udp(1, 3, 50000, 443)),
        Err(Error::Flow)
    ));
    assert!(matches!(
        lookup(&scene, 2, udp(3, 1, 443, 50000)),
        Err(Error::Flow)
    ));
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
    for packet in [
        tcp(false, 101, 901, 0x10, 0),
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
    assert_eq!(after.expires_at, before.expires_at);
    assert_eq!(after.permit_expires_at_ms, before.permit_expires_at_ms);
    assert_eq!(after.guard_expires_at_ms, before.guard_expires_at_ms);
    assert!(matches!(
        scene.process(1, None, 1, &[1, 2, 3], tcp(false, 108, 901, 0x18, 1)),
        Err(Error::Flow)
    ));
    forward(&mut scene, tcp(false, 101, 901, 0x18, 7));
}

#[test]
fn expired_or_removed_flows_are_not_returned_and_lookup_does_not_collect_entries() {
    for expiry in 0..5 {
        let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
        forward(&mut scene, udp(1, 3, 50000, 443));
        match expiry {
            0 => scene.instant += Duration::from_secs(60),
            1 => scene.now_ms = 100_000,
            2 => {
                scene
                    .tables
                    .get_mut(&node(1))
                    .unwrap()
                    .entries
                    .values_mut()
                    .next()
                    .unwrap()
                    .guard_expires_at_ms = scene.now_ms
            }
            3 => {
                scene
                    .configs
                    .get_mut(&node(1))
                    .unwrap()
                    .permits
                    .remove(&request());
            }
            4 => {
                scene
                    .configs
                    .get_mut(&node(1))
                    .unwrap()
                    .permits
                    .get_mut(&request())
                    .unwrap()
                    .expires_at_ms = scene.now_ms
            }
            _ => unreachable!(),
        }
        assert_eq!(lookup(&scene, 1, udp(1, 3, 50000, 443)).unwrap(), None);
        assert_eq!(scene.tables[&node(1)].len(), 1);
    }
}

#[test]
fn two_valid_demands_cannot_rebind_one_live_tuple_to_another_path_or_duplicate_state() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    let direct = Scene::new(BusinessProtocol::Udp, false, false);
    let alternate: RequestId = "00000000-0000-4001-8000-000000000002".parse().unwrap();
    // Compose two independently compiled grants in one node slice. Each is
    // permitted by production admission before any state exists.
    for id in [node(1), node(3)] {
        let mut permit = direct.configs[&id].permits[&request()].clone();
        permit.demand_id = alternate;
        let config = scene.configs.get_mut(&id).unwrap();
        config.permits.insert(alternate, permit);
        config.dialers.extend(direct.configs[&id].dialers.clone());
        assert!(
            admit_forward(
                config,
                alternate,
                &[node(1), node(3)],
                node(1),
                if id == node(1) { None } else { Some(node(1)) },
                &udp(1, 3, 50000, 443).facts().traffic,
                scene.now_ms
            )
            .is_ok()
        );
    }
    forward(&mut scene, udp(1, 3, 50000, 443));
    let ingress = || Ingress::Local {
        demand: alternate,
        path: vec![node(1), node(3)],
    };
    assert!(matches!(
        scene.admit(1, ingress(), udp(1, 3, 50000, 443)),
        Err(Error::Flow)
    ));
    assert!(matches!(
        scene.admit(
            3,
            Ingress::Peer {
                demand: alternate,
                path: vec![node(1), node(3)],
                identity: AuthenticatedPeer {
                    origin: node(1),
                    sender: node(1),
                    previous: node(1)
                }
            },
            udp(1, 3, 50000, 443)
        ),
        Err(Error::Flow)
    ));
    assert_eq!(
        lookup(&scene, 1, udp(1, 3, 50000, 443)).unwrap(),
        Some(original())
    );
    assert_eq!(
        lookup(&scene, 3, udp(3, 1, 443, 50000)).unwrap(),
        Some(original())
    );
    reply(&mut scene, udp(3, 1, 443, 50000));
    for table in scene.tables.values() {
        assert_eq!(table.len(), 1);
    }
    // Even the same path under a fresh ID cannot duplicate the original state.
    let config = scene.configs.get_mut(&node(1)).unwrap();
    let mut same_path = config.permits[&request()].clone();
    same_path.demand_id = alternate;
    config.permits.insert(alternate, same_path);
    assert!(matches!(
        scene.admit(
            1,
            Ingress::Local {
                demand: alternate,
                path: original().path
            },
            udp(1, 3, 50000, 443)
        ),
        Err(Error::Flow)
    ));
    scene
        .admit(
            1,
            Ingress::Local {
                demand: alternate,
                path: original().path,
            },
            udp(1, 3, 50001, 443),
        )
        .unwrap();
    assert_eq!(scene.tables[&node(1)].len(), 2);
    scene.advance(Duration::from_secs(60));
    scene
        .admit(
            1,
            Ingress::Local {
                demand: alternate,
                path: original().path,
            },
            udp(1, 3, 50000, 443),
        )
        .unwrap();
    assert_eq!(
        lookup(&scene, 1, udp(1, 3, 50000, 443))
            .unwrap()
            .unwrap()
            .demand,
        alternate
    );
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
    table.entries.insert(duplicate, flow);
    assert!(matches!(
        lookup(&scene, 1, udp(1, 3, 50000, 443)),
        Err(Error::Flow)
    ));
}

#[test]
fn fresh_demand_and_syn_cannot_restart_an_established_tcp_tuple_on_another_path() {
    let mut scene = Scene::new(BusinessProtocol::Tcp, false, true);
    let direct = Scene::new(BusinessProtocol::Tcp, false, false);
    let alternate: RequestId = "00000000-0000-4001-8000-000000000002".parse().unwrap();
    let mut permit = direct.configs[&node(1)].permits[&request()].clone();
    permit.demand_id = alternate;
    let config = scene.configs.get_mut(&node(1)).unwrap();
    config.permits.insert(alternate, permit);
    config
        .dialers
        .extend(direct.configs[&node(1)].dialers.clone());
    assert!(
        admit_forward(
            config,
            alternate,
            &[node(1), node(3)],
            node(1),
            None,
            &tcp(false, 400, 0, 2, 0).facts().traffic,
            scene.now_ms
        )
        .is_ok()
    );
    forward(&mut scene, tcp(false, 100, 0, 2, 0));
    reply(&mut scene, tcp(true, 900, 101, 0x12, 0));
    forward(&mut scene, tcp(false, 101, 901, 0x10, 0));
    assert_eq!(
        lookup(&scene, 1, tcp(false, 400, 0, 2, 0)).unwrap(),
        Some(original())
    );
    assert!(matches!(
        scene.admit(
            1,
            Ingress::Local {
                demand: alternate,
                path: vec![node(1), node(3)],
            },
            tcp(false, 400, 0, 2, 0)
        ),
        Err(Error::Flow)
    ));
    forward(&mut scene, tcp(false, 101, 901, 0x18, 7));
    reply(&mut scene, tcp(true, 901, 108, 0x18, 5));
    assert_eq!(scene.tables[&node(1)].len(), 1);
}
