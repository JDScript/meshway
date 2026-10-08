use super::*;

#[test]
fn digest_includes_capabilities_destinations_ports_and_dns() {
    let c = configuration();
    let inv = inventory();
    let mut ds = BTreeMap::from([(request(1), demand(1, 1, 2))]);
    let first = run(&c, &inv, &ds, &[]).dependency_digest;
    ds.get_mut(&request(1)).unwrap().port = Some(80.try_into().unwrap());
    assert_ne!(first, run(&c, &inv, &ds, &[]).dependency_digest);
    ds.get_mut(&request(1)).unwrap().port = Some(443.try_into().unwrap());
    let mut inv = inv;
    inv.get_mut(&node(2)).unwrap().capabilities.clear();
    assert_ne!(first, run(&c, &inv, &ds, &[]).dependency_digest);
}
#[test]
fn node_delta_removes_permissions_and_checks_the_exact_base_and_target() {
    let c = configuration();
    let inv = inventory();
    let ds = BTreeMap::from([(request(1), demand(1, 1, 2))]);
    let old = run(&c, &inv, &ds, &[]).nodes.remove(&node(1)).unwrap();
    let new = run(&c, &inv, &BTreeMap::new(), &[])
        .nodes
        .remove(&node(1))
        .unwrap();
    let delta = configuration_delta(&old, &new).unwrap();
    assert_eq!(delta.permits.remove, vec![request(1)]);
    assert!(delta.listeners.upsert.is_empty());
    assert_eq!(apply_configuration_delta(&old, delta.clone()).unwrap(), new);
    let mut bad = delta.clone();
    bad.base.digest = [99; 32].into();
    assert!(apply_configuration_delta(&old, bad).is_err());
    let mut bad = delta;
    bad.target.digest = [99; 32].into();
    assert!(apply_configuration_delta(&old, bad).is_err());
    assert!(!old.configuration.permits.is_empty());
}

#[test]
fn incompatible_tls_pair_does_not_poison_udp() {
    let mut c = configuration();
    let l: Listener = serde_json::from_str(r#"{"type":"vless","listenPort":443,"uuid":"00000000-0000-4000-8000-000000000001","tls":{"serverName":"test.example","minVersion":"1.3","maxVersion":"1.3","certificate":"certificate","privateKey":"private-key"}}"#).unwrap();
    c.nodes
        .get_mut(&node(2))
        .unwrap()
        .listeners
        .insert("vless".into(), PreparedListener::prepare(l).unwrap());
    // Structural preparation and restoration cannot substitute for the Server's
    // verification gate before this configuration reaches compilation.
    let restored: NetworkConfiguration =
        serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
    let declaration = restored.nodes[&node(2)].listeners["vless"].declaration();
    let mut context = ValidationContext {
        nodes: restored.nodes.keys().copied().collect(),
        previous: Some(&restored),
        ..Default::default()
    };
    let assert_unverified = |context: &ValidationContext<'_>| {
        assert_eq!(
            validate_configuration(&restored, context),
            Err(ConfigurationError::PreparationRequired(vec![
                PreparationRequirement {
                    path: format!("/nodes/{}/listeners/vless", node(2)),
                    kind: PreparationKind::Material(MaterialRequirement::CertificateValidation),
                }
            ]))
        );
    };
    assert_unverified(&context);
    let mut different_material = declaration.clone();
    let Listener::Vless(listener) = &mut different_material else {
        panic!()
    };
    listener.tls.certificate = Some(SecretString::new("different certificate".into()));
    context
        .verified_listeners
        .insert((node(2), "vless".into()), different_material);
    assert_unverified(&context);
    context
        .verified_listeners
        .insert((node(2), "vless".into()), declaration.clone());
    validate_configuration(&restored, &context).unwrap();
    c.defaults.dialers.vless.as_mut().unwrap().tls.max_version = TlsVersion::Tls12;
    let mut inv = inventory();
    for n in inv.values_mut() {
        n.capabilities.insert(
            "transport:vless".into(),
            Capability::Transport {
                roles: vec![TransportRole::Dial, TransportRole::Listen],
                streams: vec![StreamKind::Tcp],
            },
        );
    }
    let ds = BTreeMap::from([(request(1), demand(1, 1, 2))]);
    let result = run(&c, &inv, &ds, &[]);
    assert!(!result.nodes[&node(1)].configuration.permits.is_empty());
    assert!(
        result.nodes[&node(1)]
            .configuration
            .dialers
            .values()
            .all(|candidate| !matches!(candidate.parameters, DialParameters::Vless { .. }))
    );
    for n in inv.values_mut() {
        n.capabilities.remove("transport:native_udp");
    }
    let unavailable = run(&c, &inv, &ds, &[]);
    assert!(matches!(
        unavailable.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::NoPath
        }
    ));
}
