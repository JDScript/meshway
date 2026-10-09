use super::*;

fn retain(table: &mut FlowTable, old: &NodeConfiguration, new: &NodeConfiguration, time: FlowTime) {
    let now_ms = time.now_ms;
    let instant = time.instant;
    table.retain(&time, |flow| {
        let FlowGrant::Legacy(demand) = flow.grant else {
            return Keep::Retire;
        };
        match authorization_unchanged(old, new, *demand, flow.path, now_ms) {
            Ok(true) => Keep::Retain(Cap::until_ms(
                instant,
                now_ms,
                new.permits[demand].expires_at_ms,
            )),
            _ => Keep::Retire,
        }
    });
}

#[test]
fn retention_keeps_tcp_state_key_and_original_deadlines_in_place() {
    let mut scene = Scene::new(BusinessProtocol::Tcp, false, true);
    forward(&mut scene, tcp(false, 100, 0, 2, 0));
    reply(&mut scene, tcp(true, 900, 101, 0x12, 0));
    forward(&mut scene, tcp(false, 101, 901, 0x10, 0));
    for id in [node(1), node(2), node(3)] {
        let old = scene.configs[&id].clone();
        let mut new = old.clone();
        new.publication = "00000000-0000-4004-8000-000000000002".parse().unwrap();
        new.permits.get_mut(&request()).unwrap().expires_at_ms += 60_000;
        let (key, original) = scene.tables[&id].entries.first_key_value().unwrap();
        let (key, original) = (key.clone(), original.cap);
        let time = scene.time();
        retain(scene.tables.get_mut(&id).unwrap(), &old, &new, time);
        let table = &scene.tables[&id];
        assert_eq!(table.len(), 1);
        let (retained, flow) = table.entries.first_key_value().unwrap();
        // The key never names a publication; nothing is relabeled.
        assert_eq!(*retained, key);
        assert_eq!(flow.cap, original);
        scene.configs.insert(id, new);
    }
    // A fresh TCP state cannot admit this payload/ACK without the old handshake.
    forward(&mut scene, tcp(false, 101, 901, 0x18, 7));
    reply(&mut scene, tcp(true, 901, 108, 0x18, 5));
    for table in scene.tables.values() {
        assert_eq!(table.len(), 1);
        assert_eq!(table.entries.values().next().unwrap().cap.ms, 160_000);
    }
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
