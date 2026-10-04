//! These exercise the private query state, not a fabricated public execution
//! guard. Control lifecycle/opaque snapshot tests belong to the host fixture.
use super::*;
use crate::decode_query;
use hickory_proto::{
    op::{MessageType, OpCode, Query as Question, ResponseCode},
    rr::Name,
};
use meshway_configuration::{DnsConfiguration, DnsServer, DnsTransport, execution::DnsExecution};
use meshway_protocol::NodeDnsRecord;

fn addresses() -> NodeAddresses {
    NodeAddresses {
        ipv4: Some("100.64.0.2".parse().unwrap()),
        ipv6: None,
    }
}

fn state() -> State {
    let execution = DnsExecution {
        settings: DnsConfiguration {
            magic_dns: true,
            domain: "mesh.example".into(),
            servers: BTreeMap::from([(
                "resolver".into(),
                DnsServer {
                    transport: DnsTransport::Udp,
                    server: "192.0.2.53".parse().unwrap(),
                    server_port: 53.try_into().unwrap(),
                    use_with_exit: false,
                },
            )]),
            default: vec!["resolver".into()],
            routes: BTreeMap::new(),
            search_domains: vec![],
            override_local: true,
        },
        mesh_hosts: BTreeMap::from([("known.mesh.example".into(), addresses())]),
        associations: vec![],
        profile: "not-route-authority".into(),
        egress: meshway_configuration::execution::DnsEgress::NoExit,
    };
    State {
        binding: Binding {
            network: "00000000-0000-0000-0000-000000000001".parse().unwrap(),
            source: "00000000-0000-0000-0000-000000000002".parse().unwrap(),
            revision: Revision::new(7).unwrap(),
            self_name: node_dns_name("source", "mesh.example").unwrap(),
            self_addresses: NodeAddresses {
                ipv4: Some("100.64.0.1".parse().unwrap()),
                ipv6: None,
            },
            peers: BTreeMap::new(),
        },
        published: PublishedDns::from_execution(&execution).unwrap(),
        limits: Limits {
            max_pending: 2,
            query_timeout: Duration::from_secs(2),
        },
        generation: Arc::new(()),
        cancelled: CancellationToken::new(),
        sequence: 0,
        pending: BTreeMap::new(),
    }
}

fn query(name: &str, kind: RecordType) -> Query {
    query_name(Name::from_ascii(name).unwrap(), kind)
}

fn query_name(name: Name, kind: RecordType) -> Query {
    let mut message = Message::new(123, MessageType::Query, OpCode::Query);
    message.add_query(Question::query(name, kind));
    decode_query(&message.to_vec().unwrap()).unwrap()
}

fn begin(state: &mut State, name: &str, kind: RecordType, now: Instant) -> DiscoveryRequest {
    match state.begin(
        query(name, kind),
        ExitSelection::NoExit,
        now,
        now + Duration::from_secs(60),
    ) {
        Outcome::Discovery(request) => request,
        _ => panic!("expected discovery"),
    }
}

fn dns_response(state: &State, request: &DiscoveryRequest) -> NodeDnsQueryResponse {
    NodeDnsQueryResponse {
        request_id: "00000000-0000-0000-0000-000000000004".parse().unwrap(),
        network_id: state.binding.network,
        source_node_id: state.binding.source,
        configuration_revision: state.binding.revision,
        as_of: "1970-01-01T00:00:01.002Z".parse().unwrap(),
        record: Some(NodeDnsRecord {
            node_id: "00000000-0000-0000-0000-000000000003".parse().unwrap(),
            name: request.name.clone(),
            addresses: addresses(),
        }),
    }
}

fn unavailable(outcome: Outcome, expected: Unavailable) {
    let Outcome::Unavailable(actual) = outcome else {
        panic!("expected unavailable {expected:?}");
    };
    assert_eq!(actual, expected);
}

fn answer(outcome: Outcome) -> ReadyAnswer {
    let Outcome::Answer(answer) = outcome else {
        panic!("expected answer");
    };
    *answer
}

#[test]
fn parsed_questions_discover_exact_names_without_caching_or_request_id_aliasing() {
    let mut state = state();
    let now = Instant::now();
    let first = begin(&mut state, "UnKnOwN.MeSh.ExAmPlE.", RecordType::A, now);
    let second = begin(&mut state, "other.mesh.example", RecordType::A, now);
    assert_eq!(first.name, "unknown.mesh.example");
    // Both DNS IDs and both Server request IDs are identical: only tickets bind
    // operations, and a consumed ticket cannot consume another operation.
    let first_response = dns_response(&state, &first);
    let second_response = dns_response(&state, &second);
    assert_eq!(first_response.request_id, second_response.request_id);
    let ready = answer(state.complete(&first.ticket, first_response.clone(), now));
    let wire = encode_response(&ready.query, &ready.message, ResponseTransport::Udp, 1232).unwrap();
    let message = Message::from_vec(&wire).unwrap();
    assert_eq!(message.metadata.id, 123);
    assert_eq!(
        message.queries[0].name().to_ascii(),
        "UnKnOwN.MeSh.ExAmPlE."
    );
    assert_eq!(message.answers[0].ttl, 0);
    assert!(!message.metadata.authoritative);
    assert!(!message.metadata.authentic_data);
    assert!(!message.metadata.recursion_available);
    assert!(message.authorities.is_empty());
    unavailable(
        state.complete(&first.ticket, first_response, now),
        Unavailable::InvalidTicket,
    );
    assert_eq!(state.pending.len(), 1);
    answer(state.complete(&second.ticket, second_response, now));
    assert!(state.pending.is_empty());
    begin(&mut state, "unknown.mesh.example", RecordType::A, now);
}

#[test]
fn null_family_is_nodata_and_authenticated_null_is_uncached_refused() {
    let mut state = state();
    let now = Instant::now();
    let request = begin(&mut state, "v4.mesh.example", RecordType::AAAA, now);
    let response = dns_response(&state, &request);
    let ready = answer(state.complete(&request.ticket, response, now));
    let bytes =
        encode_response(&ready.query, &ready.message, ResponseTransport::Tcp, 1232).unwrap();
    let message = Message::from_vec(&bytes).unwrap();
    assert_eq!(message.metadata.response_code, ResponseCode::NoError);
    assert!(message.answers.is_empty() && message.authorities.is_empty());
    let request = begin(&mut state, "missing.mesh.example", RecordType::A, now);
    let mut response = dns_response(&state, &request);
    response.record = None;
    let ready = answer(state.complete(&request.ticket, response, now));
    assert_eq!(ready.message.metadata.response_code, ResponseCode::Refused);
    assert!(!ready.message.metadata.authoritative);
    assert!(ready.message.answers.is_empty() && ready.message.authorities.is_empty());
    // Neither the positive record nor the null creates a cache entry.
    begin(&mut state, "v4.mesh.example", RecordType::AAAA, now);
    begin(&mut state, "missing.mesh.example", RecordType::A, now);
}

#[test]
fn discovery_rejects_wrong_binding_revision_and_noncanonical_names() {
    for (mutate, expected) in [
        (0, Unavailable::BindingMismatch),
        (1, Unavailable::BindingMismatch),
        (2, Unavailable::BindingMismatch),
        (3, Unavailable::BindingMismatch),
        (4, Unavailable::ReloadRequired),
        (5, Unavailable::ReloadRequired),
        (6, Unavailable::BindingMismatch),
    ] {
        let mut state = state();
        let now = Instant::now();
        let request = begin(&mut state, "node.mesh.example", RecordType::A, now);
        let mut response = dns_response(&state, &request);
        match mutate {
            0 => response.network_id = "00000000-0000-0000-0000-000000000009".parse().unwrap(),
            1 => response.source_node_id = "00000000-0000-0000-0000-000000000009".parse().unwrap(),
            2 => response.record.as_mut().unwrap().name = "other.mesh.example".into(),
            3 => response.record.as_mut().unwrap().name = "NODE.mesh.example".into(),
            4 => response.configuration_revision = Revision::new(6).unwrap(),
            5 => response.configuration_revision = Revision::new(8).unwrap(),
            6 => response.record.as_mut().unwrap().name.push('.'),
            _ => unreachable!(),
        }
        unavailable(
            state.complete(&request.ticket, response.clone(), now),
            expected,
        );
        unavailable(
            state.complete(&request.ticket, response, now),
            Unavailable::InvalidTicket,
        );
        assert!(state.pending.is_empty());
    }
}

#[test]
fn unauthenticated_binding_or_wrong_revision_null_never_becomes_refused() {
    for wrong_revision in [false, true] {
        let mut state = state();
        let now = Instant::now();
        let request = begin(&mut state, "hidden.mesh.example", RecordType::A, now);
        let mut response = dns_response(&state, &request);
        response.record = None;
        let expected = if wrong_revision {
            response.configuration_revision = Revision::new(8).unwrap();
            Unavailable::ReloadRequired
        } else {
            response.source_node_id = "00000000-0000-0000-0000-000000000009".parse().unwrap();
            Unavailable::BindingMismatch
        };
        unavailable(state.complete(&request.ticket, response, now), expected);
        assert_eq!(state.pending.len(), 0);
    }
}

#[test]
fn server_clock_skew_does_not_supply_lifetime_or_bypass_local_checks() {
    let client_wall = std::time::SystemTime::now();
    for server_wall in [
        client_wall - Duration::from_secs(60),
        client_wall + Duration::from_secs(60),
        // The wire date-time schema also has no nonnegative epoch constraint.
        std::time::UNIX_EPOCH - Duration::from_secs(1),
    ] {
        let mut state = state();
        let now = Instant::now();
        let request = begin(&mut state, "node.mesh.example", RecordType::A, now);
        let mut response = dns_response(&state, &request);
        response.as_of = server_wall.into();
        let ready = answer(state.complete(&request.ticket, response.clone(), now));
        assert_eq!(ready.message.answers[0].ttl, 0);
        assert_eq!(ready.deadline, now + state.limits.query_timeout);

        let request = begin(&mut state, "node.mesh.example", RecordType::A, now);
        let mut wrong_binding = response.clone();
        wrong_binding.source_node_id = "00000000-0000-0000-0000-000000000009".parse().unwrap();
        unavailable(
            state.complete(&request.ticket, wrong_binding, now),
            Unavailable::BindingMismatch,
        );

        let request = begin(&mut state, "node.mesh.example", RecordType::A, now);
        state.cancel(&request.ticket);
        unavailable(
            state.complete(&request.ticket, response.clone(), now),
            Unavailable::InvalidTicket,
        );

        let request = begin(&mut state, "node.mesh.example", RecordType::A, now);
        unavailable(
            state.complete(&request.ticket, response, request.ticket.expires_at()),
            Unavailable::Expired,
        );
        assert!(state.pending.is_empty());
    }
}

#[test]
fn self_and_known_peer_identity_are_bound_without_guessing_missing_inventory() {
    let mut state = state();
    let now = Instant::now();
    let request = begin(&mut state, "source.mesh.example", RecordType::A, now);
    let mut response = dns_response(&state, &request);
    unavailable(
        state.complete(&request.ticket, response.clone(), now),
        Unavailable::BindingMismatch,
    );
    let request = begin(&mut state, "source.mesh.example", RecordType::A, now);
    response.record.as_mut().unwrap().node_id = state.binding.source;
    unavailable(
        state.complete(&request.ticket, response.clone(), now),
        Unavailable::BindingMismatch,
    );
    let request = begin(&mut state, "source.mesh.example", RecordType::A, now);
    response.record.as_mut().unwrap().addresses = state.binding.self_addresses.clone();
    answer(state.complete(&request.ticket, response, now));
    let request = begin(&mut state, "not-source.mesh.example", RecordType::A, now);
    let mut response = dns_response(&state, &request);
    response.record.as_mut().unwrap().node_id = state.binding.source;
    unavailable(
        state.complete(&request.ticket, response, now),
        Unavailable::BindingMismatch,
    );

    let request = begin(&mut state, "peer.mesh.example", RecordType::A, now);
    let response = dns_response(&state, &request);
    state.binding.peers.insert(
        response.record.as_ref().unwrap().node_id,
        state.binding.self_addresses.clone(),
    );
    unavailable(
        state.complete(&request.ticket, response, now),
        Unavailable::BindingMismatch,
    );
}

#[test]
fn cancellation_capacity_deadlines_and_generation_replacement_are_bounded() {
    let mut original = state();
    let now = Instant::now();
    let first = begin(&mut original, "one.mesh.example", RecordType::A, now);
    let second = begin(&mut original, "two.mesh.example", RecordType::A, now);
    unavailable(
        original.begin(
            query("three.mesh.example", RecordType::A),
            ExitSelection::NoExit,
            now,
            now + Duration::from_secs(60),
        ),
        Unavailable::Capacity,
    );
    assert!(original.cancel(&first.ticket));
    assert!(!original.cancel(&first.ticket));
    let reply = dns_response(&original, &first);
    unavailable(
        original.complete(&first.ticket, reply, now),
        Unavailable::InvalidTicket,
    );
    let third = begin(&mut original, "three.mesh.example", RecordType::A, now);
    let reply = dns_response(&original, &third);
    unavailable(
        original.complete(&third.ticket, reply, now + Duration::from_secs(2)),
        Unavailable::Expired,
    );
    original.reap(now + Duration::from_secs(2));
    assert!(original.pending.is_empty());
    assert!(second.ticket.cancelled.is_cancelled());

    original = state();
    let request = begin(&mut original, "held.mesh.example", RecordType::A, now);
    let reply = dns_response(&original, &request);
    let ready = answer(original.complete(&request.ticket, reply.clone(), now));
    let mut replacement = state();
    let fresh = begin(&mut replacement, "held.mesh.example", RecordType::A, now);
    assert_eq!(request.ticket.sequence, fresh.ticket.sequence);
    unavailable(
        replacement.complete(&request.ticket, reply, now),
        Unavailable::InvalidTicket,
    );
    assert!(!replacement.cancel(&request.ticket));
    drop(original);
    assert!(ready.cancelled.is_cancelled());
    assert!(!fresh.ticket.cancelled.is_cancelled());
    replacement.stop();
    let reply = dns_response(&replacement, &fresh);
    unavailable(
        replacement.complete(&fresh.ticket, reply, now),
        Unavailable::Inactive,
    );
    assert!(replacement.pending.is_empty());
}

#[test]
fn refresh_cancels_pending_and_unsent_answers_before_admitting_a_new_generation() {
    let mut state = state();
    let now = Instant::now();
    let request = begin(&mut state, "ready.mesh.example", RecordType::A, now);
    let response = dns_response(&state, &request);
    let ready = answer(state.complete(&request.ticket, response, now));
    let pending = begin(&mut state, "pending.mesh.example", RecordType::A, now);
    let pending_response = dns_response(&state, &pending);
    let published = answer(state.begin(
        query("known.mesh.example", RecordType::A),
        ExitSelection::NoExit,
        now,
        now + Duration::from_secs(60),
    ));

    state.refresh_generation();
    assert!(state.pending.is_empty());
    assert!(pending.ticket.cancelled.is_cancelled());
    assert!(ready.cancelled.is_cancelled());
    assert!(published.cancelled.is_cancelled());
    assert!(!state.cancelled.is_cancelled());
    unavailable(
        state.complete(&pending.ticket, pending_response, now),
        Unavailable::InvalidTicket,
    );

    // Sequences can repeat only under a different opaque generation.
    let fresh = begin(&mut state, "ready.mesh.example", RecordType::A, now);
    assert_eq!(fresh.ticket.sequence, request.ticket.sequence);
    assert!(!state.cancel(&request.ticket));
    let response = dns_response(&state, &fresh);
    let fresh_ready = answer(state.complete(&fresh.ticket, response, now));
    assert!(!fresh_ready.cancelled.is_cancelled());
    assert_eq!(fresh_ready.message.answers[0].ttl, 0);
}

#[test]
fn unsent_answers_cancel_and_query_timeout_is_capped_by_snapshot_expiry() {
    let mut state = state();
    let now = Instant::now();
    let expiry = now + Duration::from_millis(5);
    let Outcome::Discovery(request) = state.begin(
        query("node.mesh.example", RecordType::A),
        ExitSelection::NoExit,
        now,
        expiry,
    ) else {
        panic!("discovery");
    };
    assert_eq!(state.pending[&request.ticket.sequence].deadline, expiry);
    let reply = dns_response(&state, &request);
    let ready = answer(state.complete(&request.ticket, reply, now));
    assert_eq!(ready.deadline, expiry);
    assert!(!state.cancel(&request.ticket)); // consumed, but unsent output cancels
    assert!(ready.cancelled.is_cancelled());
    let ready = answer(state.begin(
        query("known.mesh.example", RecordType::A),
        ExitSelection::NoExit,
        now,
        expiry,
    ));
    assert_eq!(ready.message.answers[0].ttl, 0);
    assert_eq!(ready.deadline, expiry);
    state.stop();
    assert!(ready.cancelled.is_cancelled());
}

#[test]
fn arbitrary_wire_labels_never_become_lossy_or_inverse_discovery_names() {
    let mut state = state();
    let now = Instant::now();
    for label in [
        b"\xff".as_slice(),
        b"two.labels",
        b"_service",
        b"*",
        "two。labels".as_bytes(),
        "two．labels".as_bytes(),
        "two｡labels".as_bytes(),
    ] {
        let name = Name::from_labels([label, b"mesh", b"example"]).unwrap();
        unavailable(
            state.begin(
                query_name(name, RecordType::A),
                ExitSelection::NoExit,
                now,
                now + Duration::from_secs(60),
            ),
            Unavailable::InvalidDiscoveryName,
        );
    }
    let request = begin(&mut state, "XN--BCHER-KVA.mesh.example", RecordType::A, now);
    assert_eq!(request.name, "xn--bcher-kva.mesh.example");
    assert_eq!(state.pending.len(), 1);
    unavailable(
        state.begin(
            query("node.mesh.example", RecordType::TXT),
            ExitSelection::NoExit,
            now,
            now + Duration::from_secs(60),
        ),
        Unavailable::UnsupportedRecordType,
    );
    unavailable(
        state.begin(
            query("2.0.64.100.in-addr.arpa", RecordType::PTR),
            ExitSelection::NoExit,
            now,
            now + Duration::from_secs(60),
        ),
        Unavailable::MissingResolverPlan,
    );
    assert_eq!(state.pending.len(), 1);
}

#[test]
fn resolver_candidates_and_profile_never_become_an_executable_dns_route() {
    let mut state = state();
    let now = Instant::now();
    unavailable(
        state.begin(
            query("outside.example", RecordType::A),
            ExitSelection::NoExit,
            now,
            now + Duration::from_secs(60),
        ),
        Unavailable::MissingResolverPlan,
    );
    unavailable(
        state.begin(
            query("outside.example", RecordType::A),
            ExitSelection::Exit,
            now,
            now + Duration::from_secs(60),
        ),
        Unavailable::MissingExitPlan,
    );
    assert!(state.pending.is_empty());
}
