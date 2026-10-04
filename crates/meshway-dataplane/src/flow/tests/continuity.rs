use super::*;

#[test]
fn relabel_preserves_tcp_state_and_original_deadlines_until_current_packet_admission() {
    let mut scene = Scene::new(BusinessProtocol::Tcp, false, true);
    forward(&mut scene, tcp(false, 100, 0, 2, 0));
    reply(&mut scene, tcp(true, 900, 101, 0x12, 0));
    forward(&mut scene, tcp(false, 101, 901, 0x10, 0));
    for id in [node(1), node(2), node(3)] {
        let old = scene.configs[&id].clone();
        let mut new = old.clone();
        new.publication = "00000000-0000-4004-8000-000000000002".parse().unwrap();
        new.permits.get_mut(&request()).unwrap().expires_at_ms += 60_000;
        let original = scene.tables[&id].entries.values().next().unwrap();
        let original_deadline = original.expires_at;
        let original_wall = original.permit_expires_at_ms;
        let table = scene.tables[&id]
            .reconciled(
                &old,
                &new,
                FlowTime {
                    instant: scene.instant,
                    now_ms: scene.now_ms,
                    guard_expires_at: scene.guard_expires_at,
                    guard_expires_at_ms: scene.guard_expires_at_ms,
                },
            )
            .unwrap();
        assert_eq!(table.len(), 1);
        let (key, flow) = table.entries.first_key_value().unwrap();
        assert_eq!(key.publication, new.publication);
        assert_eq!(flow.expires_at, original_deadline);
        assert_eq!(flow.permit_expires_at_ms, original_wall);
        scene.tables.insert(id, table);
        scene.configs.insert(id, new);
    }
    // A fresh TCP state cannot admit this payload/ACK without the old handshake.
    forward(&mut scene, tcp(false, 101, 901, 0x18, 7));
    reply(&mut scene, tcp(true, 901, 108, 0x18, 5));
    for table in scene.tables.values() {
        assert_eq!(table.len(), 1);
        assert_eq!(
            table.entries.values().next().unwrap().permit_expires_at_ms,
            160_000
        );
    }
}

#[test]
fn changed_authorization_peers_and_expired_retention_are_never_migrated() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    forward(&mut scene, udp(1, 3, 50000, 443));
    let old = &scene.configs[&node(1)];
    let table = &scene.tables[&node(1)];
    let empty = |new: &NodeConfiguration, instant: Instant, wall: u64| {
        assert_eq!(
            table
                .reconciled(
                    old,
                    new,
                    FlowTime {
                        instant,
                        now_ms: wall,
                        guard_expires_at: scene.guard_expires_at,
                        guard_expires_at_ms: scene.guard_expires_at_ms,
                    }
                )
                .unwrap()
                .len(),
            0
        );
    };
    let mut new = old.clone();
    new.permits.remove(&request());
    empty(&new, scene.instant, scene.now_ms);
    new = old.clone();
    new.permits.get_mut(&request()).unwrap().port = Some(444.try_into().unwrap());
    empty(&new, scene.instant, scene.now_ms);
    new = old.clone();
    new.permits.get_mut(&request()).unwrap().paths[0]
        .dial_candidates
        .clear();
    empty(&new, scene.instant, scene.now_ms);
    new = old.clone();
    new.peers.get_mut(&node(2)).unwrap().addresses.ipv4 = Some("100.64.0.44".parse().unwrap());
    empty(&new, scene.instant, scene.now_ms);
    new = old.clone();
    new.peers
        .get_mut(&node(3))
        .unwrap()
        .dataplane_key
        .generation = meshway_model::Version::new(2).unwrap();
    empty(&new, scene.instant, scene.now_ms);
    new = old.clone();
    new.permits.get_mut(&request()).unwrap().expires_at_ms += 60_000;
    empty(&new, scene.instant + Duration::from_secs(60), scene.now_ms);
    empty(&new, scene.instant, 100_000);
    // Failed migration candidates never modify the source table.
    assert_eq!(table.len(), 1);
}

#[test]
fn authorization_digest_excludes_only_expiry_and_never_serializes_a_fake_timestamp() {
    let scene = Scene::new(BusinessProtocol::Udp, false, true);
    let permit = &scene.configs[&node(1)].permits[&request()];
    let digest = meshway_policy::permit_authorization_digest(permit).unwrap();
    let mut changed = permit.clone();
    changed.expires_at_ms = u64::MAX;
    // The actual wire digest rejects this invalid UTC timestamp; the pure
    // authorization projection must not pass expiry through that serializer.
    assert!(meshway_configuration::digest(&changed).is_err());
    assert_eq!(
        meshway_policy::permit_authorization_digest(&changed).unwrap(),
        digest
    );
    changed.profile.push_str("-changed");
    assert_ne!(
        meshway_policy::permit_authorization_digest(&changed).unwrap(),
        digest
    );
}
