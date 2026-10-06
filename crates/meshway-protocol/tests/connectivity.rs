use meshway_model::{
    Digest, Sequence, Version,
    state::{EndpointCandidate, EndpointSource, SocketNetwork},
};
use meshway_protocol::{
    connectivity::*,
    state::{CandidateSource, PeerCandidate, PeerEndpoint},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::net::SocketAddr;
use utoipa::PartialSchema;

const A: &str = "00000000-0000-4000-8000-000000000001";
const B: &str = "00000000-0000-4000-8000-000000000002";
const C: &str = "00000000-0000-4000-8000-000000000003";

fn socket() -> SocketReference {
    serde_json::from_value(json!({
        "nodeId": A,
        "instance": {"instanceId": B, "generation": 1},
        "configuration": {"revision": 1, "executionPublication": C, "digest": "01".repeat(32)},
        "socketId": "native-udp-outgoing:1",
        "socketGeneration": 1,
        "resource": {"kind": "dial", "key": "native-udp"}
    }))
    .unwrap()
}

fn observation() -> EndpointCandidate {
    serde_json::from_value(json!({
        "address": "198.51.100.7:41000",
        "origin": {"source": "stun", "observer": "203.0.113.8:3478"},
        "observedAt": "2026-10-04T00:00:00Z",
        "expiresAt": "2026-10-04T00:00:30Z"
    }))
    .unwrap()
}

fn binding() -> SocketAddr {
    "0.0.0.0:41000".parse().unwrap()
}

fn reference() -> CandidateReference {
    CandidateReference::from_observation(socket(), SocketNetwork::Udp, binding(), &observation())
        .unwrap()
}

fn offer() -> UdpTraversalOffer {
    let local = reference();
    let mut peer_socket = socket();
    peer_socket.node_id = B.parse().unwrap();
    peer_socket.instance.instance_id = C.parse().unwrap();
    peer_socket.resource = meshway_model::state::ResourceRef::Listener {
        key: "native-udp".into(),
    };
    let mut peer_observation = observation();
    peer_observation.address = "203.0.113.9:42000".into();
    let peer = CandidateReference::from_observation(
        peer_socket,
        SocketNetwork::Udp,
        "0.0.0.0:42000".parse().unwrap(),
        &peer_observation,
    )
    .unwrap();
    let mut offer: UdpTraversalOffer = serde_json::from_value(json!({
        "attemptId": A, "requestId": B, "networkId": C, "stateEpoch": C,
        "version": 1, "digest": "00".repeat(32), "nonce": "02".repeat(32),
        "carrier": {"dialer": A, "listener": B, "listenerKey": "native-udp", "protocol": "native_udp"},
        "local": {"candidate": local, "address": observation().address, "source": "stun"},
        "peer": {"candidate": peer, "address": peer_observation.address, "source": "stun"},
        "expiresAt": "2026-10-04T00:00:10Z"
    })).unwrap();
    offer.digest = offer.compute_digest().unwrap();
    offer
}

// Exercise actual external JSON boundaries, including duplicate/trailing input
// which a serde_json::Value fixture cannot represent.
fn strict_object<T: Serialize + DeserializeOwned>(value: &T) {
    let value = serde_json::to_value(value).unwrap();
    let text = serde_json::to_string(&value).unwrap();
    assert!(serde_json::from_str::<T>(&text).is_ok());
    assert!(serde_json::from_str::<T>(&format!("{text} null")).is_err());
    for invalid in [json!([]), json!(null)] {
        assert!(serde_json::from_value::<T>(invalid).is_err());
    }
    let fields = value.as_object().unwrap();
    let mut unknown = value.clone();
    unknown["unknown"] = json!(true);
    assert!(serde_json::from_value::<T>(unknown).is_err());
    for (key, field) in fields {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<T>(missing).is_err(),
            "missing {key}"
        );
        let mut null = value.clone();
        null[key] = Value::Null;
        assert!(serde_json::from_value::<T>(null).is_err(), "null {key}");
        let duplicate = format!(
            "{{{}:{field},{}",
            serde_json::to_string(key).unwrap(),
            &text[1..]
        );
        assert!(
            serde_json::from_str::<T>(&duplicate).is_err(),
            "duplicate {key}"
        );
    }
}

#[test]
fn traversal_wire_boundaries_are_closed_and_all_fields_are_required_nonnullable() {
    let offer = offer();
    let socket = socket();
    let query = QueryUdpTraversalRequest {
        instance: socket.instance.clone(),
        configuration: socket.configuration.clone(),
    };
    let prepare = PrepareUdpTraversalRequest {
        instance: query.instance.clone(),
        configuration: query.configuration.clone(),
        state_epoch: offer.state_epoch,
        request_id: offer.request_id,
        expires_at_ms: offer.expires_at_ms,
        peer_id: offer.peer.candidate.socket.node_id,
        carrier: offer.carrier.clone(),
        local_candidate: offer.local.candidate.clone(),
        peer_candidate: offer.peer.candidate.clone(),
    };
    let cas = UdpTraversalAttemptRequest {
        instance: query.instance.clone(),
        configuration: query.configuration.clone(),
        state_epoch: offer.state_epoch,
        version: offer.version,
        digest: offer.digest,
    };
    strict_object(&query);
    strict_object(&prepare);
    strict_object(&cas);
    strict_object(&socket);
    strict_object(&reference());
    strict_object(&offer);
    strict_object(&offer.local);
    strict_object(&offer.carrier);
    strict_object(&StunServer {
        id: "operator-a".into(),
        endpoint: serde_json::from_value(json!({"host":"stun.example.test","port":3478})).unwrap(),
    });
    strict_object(&PrepareUdpTraversalResponse::Offer {
        offer: offer.clone(),
    });
    strict_object(&AcceptUdpTraversalResponse::Waiting {
        offer: offer.clone(),
    });
    strict_object(&AcceptUdpTraversalResponse::Active {
        offer: offer.clone(),
    });
    strict_object(&CancelUdpTraversalResponse::Cancelled);
    strict_object(&QueryUdpTraversalResponse::Current {
        state_epoch: offer.state_epoch,
        as_of_ms: offer.expires_at_ms - 10_000,
        expires_at_ms: offer.expires_at_ms,
        stun_servers: Vec::new().try_into().unwrap(),
        attempts: vec![UdpTraversalAttempt {
            offer,
            phase: UdpTraversalPhase::Waiting,
        }]
        .try_into()
        .unwrap(),
    });
    for reason in [
        UdpTraversalUnavailableReason::ScopeChanged,
        UdpTraversalUnavailableReason::CandidateChanged,
        UdpTraversalUnavailableReason::AttemptChanged,
        UdpTraversalUnavailableReason::AttemptExpired,
        UdpTraversalUnavailableReason::NotAvailable,
        UdpTraversalUnavailableReason::Capacity,
        UdpTraversalUnavailableReason::RequestChanged,
    ] {
        strict_object(&QueryUdpTraversalResponse::Unavailable { reason });
        strict_object(&PrepareUdpTraversalResponse::Unavailable { reason });
        strict_object(&AcceptUdpTraversalResponse::Unavailable { reason });
        strict_object(&CancelUdpTraversalResponse::Unavailable { reason });
    }
    assert!(
        serde_json::from_value::<AcceptUdpTraversalResponse>(json!({"status":"active","offer":[]}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<CancelUdpTraversalResponse>(
            json!({"status":"unavailable","reason":"unknown"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<CancelUdpTraversalResponse>(json!({"status":"finished"})).is_err()
    );
}

#[test]
fn socket_ids_and_millisecond_cas_deadlines_are_validated() {
    for invalid in [
        "".to_string(),
        "x".repeat(MAX_SOCKET_ID_BYTES + 1),
        "socket\n".into(),
    ] {
        let mut value = serde_json::to_value(socket()).unwrap();
        value["socketId"] = json!(invalid);
        assert!(serde_json::from_value::<SocketReference>(value).is_err());
    }
    let mut value = serde_json::to_value(socket()).unwrap();
    value["socketId"] = json!("x".repeat(MAX_SOCKET_ID_BYTES));
    assert!(serde_json::from_value::<SocketReference>(value).is_ok());
    for invalid in [
        json!(0),
        json!("2026-10-04T00:00:10.000001Z"),
        json!("1969-12-31T23:59:59Z"),
    ] {
        let mut value = serde_json::to_value(offer()).unwrap();
        value["expiresAt"] = invalid;
        assert!(serde_json::from_value::<UdpTraversalOffer>(value).is_err());
    }
    let mut value = serde_json::to_value(socket()).unwrap();
    value["instance"]["generation"] = json!(0);
    assert!(serde_json::from_value::<SocketReference>(value).is_err());
}

#[test]
fn ascii_identifiers_have_matching_rust_and_external_schema_boundaries() {
    for invalid in [
        "",
        "socket id",
        "é",
        "socket\u{0085}",
        "socket\n",
        "socket\r",
        "socket\u{2028}",
        "socket\u{007f}",
    ] {
        let mut value = serde_json::to_value(socket()).unwrap();
        value["socketId"] = json!(invalid);
        assert!(
            serde_json::from_value::<SocketReference>(value).is_err(),
            "{invalid:?}"
        );
        let mut reference = socket();
        reference.socket_id = invalid.into();
        assert!(
            CandidateReference::from_observation(
                reference,
                SocketNetwork::Udp,
                binding(),
                &observation()
            )
            .is_err()
        );
    }
    for valid in ["abc0123456789", "socket:udp/listener_1-2", "!~"] {
        let mut value = serde_json::to_value(socket()).unwrap();
        value["socketId"] = json!(valid);
        assert_eq!(
            serde_json::from_value::<SocketReference>(value)
                .unwrap()
                .socket_id,
            valid
        );
    }
    for invalid in [
        "".into(),
        "é".into(),
        "server id".into(),
        "a:b".into(),
        "server\n".into(),
        "x".repeat(65),
    ] {
        let value = json!({"id":invalid,"endpoint":{"host":"stun.example.test","port":3478}});
        assert!(serde_json::from_value::<StunServer>(value).is_err());
    }
    let value = json!({"id":"x".repeat(64),"endpoint":{"host":"stun.example.test","port":3478}});
    assert!(serde_json::from_value::<StunServer>(value).is_ok());
    let socket = serde_json::to_value(SocketReference::schema()).unwrap();
    assert_eq!(
        socket["properties"]["socketId"]["pattern"],
        r"^[!-~]{1,256}(?![\s\S])"
    );
    assert_eq!(socket["properties"]["socketId"]["maxLength"], 256);
    assert_eq!(socket["properties"]["socketId"]["minLength"], 1);
    let stun = serde_json::to_value(StunServer::schema()).unwrap();
    assert_eq!(
        stun["properties"]["id"]["pattern"],
        r"^[A-Za-z0-9_-]{1,64}(?![\s\S])"
    );
    assert_eq!(stun["properties"]["id"]["maxLength"], 64);
    assert_eq!(stun["properties"]["id"]["minLength"], 1);
}

#[test]
fn collection_bounds_hold_for_construction_decode_and_schema() {
    let stun: StunServer = serde_json::from_value(
        json!({"id":"operator-a", "endpoint":{"host":"stun.example.test", "port":3478}}),
    )
    .unwrap();
    assert!(StunServers::try_from(vec![stun.clone(); MAX_STUN_SERVERS]).is_ok());
    assert!(StunServers::try_from(vec![stun.clone(); MAX_STUN_SERVERS + 1]).is_err());
    assert!(
        serde_json::from_value::<StunServers>(json!(vec![stun.clone(); MAX_STUN_SERVERS])).is_ok()
    );
    assert!(
        serde_json::from_value::<StunServers>(json!(vec![stun; MAX_STUN_SERVERS + 1])).is_err()
    );
    let attempt = UdpTraversalAttempt {
        offer: offer(),
        phase: UdpTraversalPhase::Waiting,
    };
    assert!(
        UdpTraversalAttempts::try_from(vec![attempt.clone(); MAX_UDP_TRAVERSAL_ATTEMPTS]).is_ok()
    );
    assert!(
        UdpTraversalAttempts::try_from(vec![attempt.clone(); MAX_UDP_TRAVERSAL_ATTEMPTS + 1])
            .is_err()
    );
    assert!(
        serde_json::from_value::<UdpTraversalAttempts>(json!(vec![
            attempt.clone();
            MAX_UDP_TRAVERSAL_ATTEMPTS
        ]))
        .is_ok()
    );
    assert!(
        serde_json::from_value::<UdpTraversalAttempts>(json!(vec![
            attempt;
            MAX_UDP_TRAVERSAL_ATTEMPTS + 1
        ]))
        .is_err()
    );
    assert!(serde_json::from_value::<StunServers>(json!(null)).is_err());
    assert!(serde_json::from_value::<UdpTraversalAttempts>(json!({})).is_err());
    assert_eq!(
        serde_json::to_value(StunServers::schema()).unwrap()["maxItems"],
        MAX_STUN_SERVERS
    );
    assert_eq!(
        serde_json::to_value(UdpTraversalAttempts::schema()).unwrap()["maxItems"],
        MAX_UDP_TRAVERSAL_ATTEMPTS
    );
}

#[test]
fn every_socket_binding_component_changes_the_observation_identity() {
    let original = reference().observation_digest;
    let encoded = serde_json::to_value(socket()).unwrap();
    for (pointer, replacement) in [
        ("/nodeId", json!(B)),
        ("/instance/instanceId", json!(C)),
        ("/instance/generation", json!(2)),
        ("/configuration/revision", json!(2)),
        ("/configuration/executionPublication", json!(A)),
        ("/configuration/digest", json!("03".repeat(32))),
        ("/socketId", json!("native-udp-outgoing:2")),
        ("/socketGeneration", json!(2)),
        ("/resource/kind", json!("listener")),
        ("/resource/key", json!("other")),
    ] {
        let mut value = encoded.clone();
        *value.pointer_mut(pointer).unwrap() = replacement;
        let socket: SocketReference = serde_json::from_value(value).unwrap();
        assert_ne!(
            candidate_observation_digest(&socket, SocketNetwork::Udp, binding(), &observation())
                .unwrap(),
            original,
            "{pointer}"
        );
    }
    assert_ne!(
        candidate_observation_digest(&socket(), SocketNetwork::Tcp, binding(), &observation())
            .unwrap(),
        original
    );
    for bound in ["0.0.0.0:41001", "192.0.2.1:41000"] {
        assert_ne!(
            candidate_observation_digest(
                &socket(),
                SocketNetwork::Udp,
                bound.parse().unwrap(),
                &observation()
            )
            .unwrap(),
            original
        );
    }
}

#[test]
fn original_candidate_origin_address_and_times_are_committed_before_redaction() {
    let original = reference().observation_digest;
    let encoded = serde_json::to_value(observation()).unwrap();
    for (pointer, replacement) in [
        ("/address", json!("198.51.100.7:41001")),
        ("/origin/observer", json!("203.0.113.9:3478")),
        ("/observedAt", json!("2026-10-04T00:00:00.001Z")),
        ("/expiresAt", json!("2026-10-04T00:00:30.001Z")),
        (
            "/origin",
            json!({"source":"configured","configPath":"listeners.native-udp.endpoints[0]"}),
        ),
        ("/origin", json!({"source":"interface","interface":"eth0"})),
    ] {
        let mut value = encoded.clone();
        *value.pointer_mut(pointer).unwrap() = replacement;
        let candidate: EndpointCandidate = serde_json::from_value(value).unwrap();
        assert_ne!(
            candidate_observation_digest(&socket(), SocketNetwork::Udp, binding(), &candidate)
                .unwrap(),
            original,
            "{pointer}"
        );
    }
    for (first, second) in [
        (
            EndpointSource::Configured {
                config_path: "listeners.a".into(),
            },
            EndpointSource::Configured {
                config_path: "listeners.b".into(),
            },
        ),
        (
            EndpointSource::Interface {
                interface: "eth0".into(),
            },
            EndpointSource::Interface {
                interface: "eth1".into(),
            },
        ),
    ] {
        let mut candidate = observation();
        candidate.origin = first;
        let a = candidate_observation_digest(&socket(), SocketNetwork::Udp, binding(), &candidate)
            .unwrap();
        candidate.origin = second;
        assert_ne!(
            candidate_observation_digest(&socket(), SocketNetwork::Udp, binding(), &candidate)
                .unwrap(),
            a
        );
    }
    let candidate = observation();
    let peer = PeerEndpoint {
        socket_id: socket().socket_id,
        resource: socket().resource,
        generation: Sequence::new(1).unwrap(),
        network: SocketNetwork::Udp,
        candidate: PeerCandidate {
            address: candidate.address,
            source: CandidateSource::Stun,
            observed_at: candidate.observed_at,
            expires_at: candidate.expires_at,
        },
        candidate_reference: reference(),
    };
    strict_object(&peer);
    let encoded = serde_json::to_string(&peer).unwrap();
    assert!(!encoded.contains("observer"));
    assert!(!encoded.contains("203.0.113.8"));
    assert!(!encoded.contains("boundAddress"));
}

#[test]
fn observation_identity_survives_json_order_and_timestamp_spelling_changes() {
    let candidate: EndpointCandidate = serde_json::from_str(
        r#"{
        "expiresAt":"2026-10-04T00:00:30.000+00:00",
        "origin":{"observer":"203.0.113.8:3478","source":"stun"},
        "observedAt":"2026-10-04T01:00:00+01:00",
        "address":"198.51.100.7:41000"
    }"#,
    )
    .unwrap();
    assert_eq!(
        candidate_observation_digest(&socket(), SocketNetwork::Udp, binding(), &candidate).unwrap(),
        reference().observation_digest
    );
    // The API accepts no enclosing report sequence or receipt timestamp. A
    // telemetry round trip leaves both the identity and original expiry intact.
    let roundtrip: CandidateReference =
        serde_json::from_value(serde_json::to_value(reference()).unwrap()).unwrap();
    assert_eq!(roundtrip, reference());
}

#[test]
fn offer_digest_binds_all_immutable_scope_and_is_independent_of_acceptance_phase() {
    let original = offer();
    let encoded = serde_json::to_value(&original).unwrap();
    for (pointer, replacement) in [
        ("/attemptId", json!(B)),
        ("/requestId", json!(A)),
        ("/networkId", json!(A)),
        ("/stateEpoch", json!(B)),
        ("/version", json!(2)),
        ("/nonce", json!("04".repeat(32))),
        ("/carrier/listenerKey", json!("different")),
        ("/local/address", json!("198.51.100.7:41001")),
        ("/peer/address", json!("203.0.113.9:42001")),
        ("/local/source", json!("configured")),
        ("/peer/source", json!("interface")),
        ("/local/candidate/observationDigest", json!("05".repeat(32))),
        ("/peer/candidate/socket/socketGeneration", json!(2)),
        ("/expiresAt", json!("2026-10-04T00:00:09Z")),
    ] {
        let mut value = encoded.clone();
        *value.pointer_mut(pointer).unwrap() = replacement;
        let changed: UdpTraversalOffer = serde_json::from_value(value).unwrap();
        assert_ne!(
            changed.compute_digest().unwrap(),
            original.digest,
            "{pointer}"
        );
    }
    let mut attempt = UdpTraversalAttempt {
        offer: original.clone(),
        phase: UdpTraversalPhase::Waiting,
    };
    attempt.phase = UdpTraversalPhase::Active;
    assert_eq!(attempt.offer.compute_digest().unwrap(), original.digest);
    attempt.offer.digest = Digest::from_bytes([99; 32]);
    assert_eq!(attempt.offer.compute_digest().unwrap(), original.digest);
    assert_eq!(original.version, Version::MIN);
}

#[test]
fn traversal_ids_reject_aliases_and_nil() {
    for invalid in [
        "00000000-0000-0000-0000-000000000000",
        "00000000000040008000000000000001",
        "00000000-0000-4000-8000-00000000000A",
    ] {
        let mut value = serde_json::to_value(offer()).unwrap();
        value["attemptId"] = json!(invalid);
        assert!(serde_json::from_value::<UdpTraversalOffer>(value).is_err());
    }
}
