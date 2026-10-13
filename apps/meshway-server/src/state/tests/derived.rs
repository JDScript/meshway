use super::*;
use meshway_model::resources::{AddressScope, LocationAbsentReason, LocationSource};

fn derive_full(state: &NodeState, now: DateTime<Utc>) -> DerivedNodeState {
    super::super::derived::derive(
        Some(&VisibleReport::Full {
            report: Box::new(state.clone()),
        }),
        true,
        &BTreeSet::from(["100.64.0.1".parse::<IpAddr>().unwrap()]),
        now,
    )
}

fn add(
    state: &mut NodeState,
    socket: &str,
    address: &str,
    origin: EndpointSource,
    now: DateTime<Utc>,
) {
    let mut value = state.sockets["s1"].clone();
    value.endpoints = vec![EndpointCandidate {
        address: address.into(),
        origin,
        observed_at: now,
        expires_at: now + Duration::seconds(10),
    }];
    state.sockets.insert(socket.into(), value);
}

fn stun() -> EndpointSource {
    EndpointSource::Stun {
        observer: "8.8.8.8:3478".into(),
    }
}

fn interface() -> EndpointSource {
    EndpointSource::Interface {
        interface: "eth0".into(),
    }
}

#[test]
fn location_names_the_observed_address_without_any_dataset() {
    let (admission, report, scope, now) = setup();
    let mut hub = StateHub::new().unwrap();
    hub.accept_report(&admission, report, now).unwrap();
    let view = hub.query(&scope, None, now).unwrap();
    let derived = &view.nodes[&admission.node_id].derived;
    let location = derived.location.as_ref().unwrap();
    assert_eq!(location.address, "1.1.1.1");
    assert_eq!(location.endpoint_source, LocationSource::Stun);
    assert_eq!(location.source_socket_id, "s1");
    assert_eq!(location.source_generation, Sequence::new(1).unwrap());
    assert_eq!(location.expires_at, now + Duration::seconds(10));
    assert_eq!(derived.location_absent, None);
    assert_eq!(
        derived.address_scope,
        BTreeMap::from([("1.1.1.1".to_owned(), AddressScope::Public)])
    );
    let json = serde_json::to_value(derived).unwrap();
    assert!(json.get("ipInfo").is_none());
    assert!(json["location"].get("data").is_none());
    assert!(json["location"].get("datasetId").is_none());
    let later = hub
        .query(&scope, None, now + Duration::seconds(11))
        .unwrap();
    let derived = &later.nodes[&admission.node_id].derived;
    assert_eq!(derived.location, None);
    assert_eq!(
        derived.location_absent,
        Some(LocationAbsentReason::ObservationExpired)
    );
}

#[test]
fn address_scope_covers_only_endpoints_the_caller_sees() {
    let (admission, mut report, mut scope, now) = setup();
    report
        .state
        .sockets
        .get_mut("s1")
        .unwrap()
        .endpoints
        .push(EndpointCandidate {
            address: "1.1.1.1:9999".into(),
            origin: EndpointSource::Configured {
                config_path: "/listeners/udp/endpoints/0".into(),
            },
            observed_at: now,
            expires_at: now + Duration::seconds(10),
        });
    let mut hub = StateHub::new().unwrap();
    hub.accept_report(&admission, report, now).unwrap();
    scope.full.clear();
    scope.endpoint_resources = Some(BTreeMap::new());
    let view = hub.query(&scope, None, now).unwrap();
    let derived = &view.nodes[&admission.node_id].derived;
    assert!(derived.address_scope.is_empty());
    assert_eq!(derived.location, None);
    assert_eq!(
        derived.location_absent,
        Some(LocationAbsentReason::NoObservation)
    );
    scope.endpoint_resources = None;
    let view = hub.query(&scope, None, now).unwrap();
    assert_eq!(
        view.nodes[&admission.node_id].derived.address_scope,
        BTreeMap::from([("1.1.1.1".to_owned(), AddressScope::Public)])
    );
}

#[test]
fn scopes_classify_mesh_private_and_configured_addresses() {
    let (_, mut report, _, now) = setup();
    let state = &mut report.state;
    state.sockets.get_mut("s1").unwrap().endpoints.clear();
    add(state, "a", "100.64.0.1:1", interface(), now);
    add(state, "b", "192.168.1.2:1", interface(), now);
    add(
        state,
        "c",
        "1.1.1.1:2",
        EndpointSource::Configured {
            config_path: "/listeners/udp/endpoints/0".into(),
        },
        now,
    );
    add(state, "d", "[2606:4700::1]:3", interface(), now);
    let derived = derive_full(state, now);
    assert_eq!(
        derived.address_scope,
        BTreeMap::from([
            ("100.64.0.1".to_owned(), AddressScope::Mesh),
            ("192.168.1.2".to_owned(), AddressScope::NonPublic),
            ("1.1.1.1".to_owned(), AddressScope::Public),
            ("2606:4700::1".to_owned(), AddressScope::Public),
        ])
    );
    // A configured declaration is never a location; the interface one is.
    assert_eq!(derived.location.unwrap().address, "2606:4700::1");
}

#[test]
fn several_addresses_select_by_rank_or_are_ambiguous() {
    let (_, report, _, now) = setup();
    let mut state = report.state;
    // STUN outranks interface, IPv4 outranks IPv6.
    add(&mut state, "a", "9.9.9.9:1", interface(), now);
    add(&mut state, "b", "[2606:4700::1]:1", stun(), now);
    let derived = derive_full(&state, now);
    assert_eq!(derived.location.as_ref().unwrap().address, "1.1.1.1");
    // The same address on two sockets: the later deadline wins.
    add(&mut state, "c", "1.1.1.1:2", stun(), now);
    state.sockets.get_mut("c").unwrap().endpoints[0].expires_at = now + Duration::seconds(20);
    let location = derive_full(&state, now).location.unwrap();
    assert_eq!(location.source_socket_id, "c");
    // Two distinct addresses at the best rank are ambiguous.
    add(&mut state, "d", "8.8.4.4:1", stun(), now);
    let derived = derive_full(&state, now);
    assert_eq!(derived.location, None);
    assert_eq!(
        derived.location_absent,
        Some(LocationAbsentReason::AmbiguousObservation)
    );
    // Only expired observations at a rank do not count toward it.
    state.sockets.get_mut("d").unwrap().endpoints[0].expires_at = now;
    assert_eq!(
        derive_full(&state, now).location.unwrap().address,
        "1.1.1.1"
    );
}
