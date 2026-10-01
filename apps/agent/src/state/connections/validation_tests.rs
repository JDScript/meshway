use meshway_protocol::{
    AgentConnectionsSnapshot, ConnectionCapability, ConnectionEndpoint, ConnectionListener,
    ConnectionPeer, ConnectionSemantics, ConnectionTransport, IncomingConnectionApproval,
    NodeConnections, OutgoingConnectionCandidate, connection_digest,
};

use super::{super::Identity, validation};

fn public_key(seed: u8) -> String {
    let secret = x25519_dalek::StaticSecret::from([seed; 32]);
    x25519_dalek::PublicKey::from(&secret)
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn fixture() -> (Identity, AgentConnectionsSnapshot) {
    let identity = Identity {
        format_version: 1,
        controller: "http://127.0.0.1:1".into(),
        name: "own".into(),
        enrollment_version: "0.1.0".into(),
        enrollment_token_hash: vec![0; 32],
        node_credential: "1".repeat(64),
        node_id: Some("2".repeat(64)),
    };
    let payload = NodeConnections {
        schema_version: 1,
        revision: 10,
        node_id: identity.node_id.clone().unwrap(),
        public_key: Some(public_key(1)),
        capabilities: vec![ConnectionCapability {
            transport: ConnectionTransport::Vless,
            can_dial: true,
            can_accept: true,
            semantics: ConnectionSemantics::Stream,
        }],
        valid: true,
        diagnostics: vec![],
        listeners: vec![ConnectionListener {
            id: "local".into(),
            node_id: identity.node_id.clone().unwrap(),
            transport: ConnectionTransport::Vless,
            bind: "0.0.0.0:8421".into(),
            advertised_endpoints: vec![ConnectionEndpoint {
                host: "local.example".into(),
                port: 8421,
            }],
            credential_ref: Some("own-accept-profile".into()),
        }],
        outgoing: vec![OutgoingConnectionCandidate {
            id: "dial".into(),
            listener_id: "remote".into(),
            peer_node_id: "3".repeat(64),
            transport: ConnectionTransport::Vless,
            endpoint: ConnectionEndpoint {
                host: "remote.example".into(),
                port: 8421,
            },
            priority: 0,
            credential_ref: Some("own-dial-profile".into()),
        }],
        incoming: vec![IncomingConnectionApproval {
            candidate_id: "accept".into(),
            listener_id: "local".into(),
            peer_node_id: "3".repeat(64),
            transport: ConnectionTransport::Vless,
        }],
        peers: vec![ConnectionPeer {
            node_id: "3".repeat(64),
            public_key: public_key(2),
        }],
    };
    let digest = connection_digest(&payload).unwrap();
    (identity, AgentConnectionsSnapshot { digest, payload })
}

#[test]
fn scoped_projection_accepts_owned_listener_and_direct_peer_grants_only() {
    let (identity, snapshot) = fixture();
    let own_key = snapshot.payload.public_key.as_deref().unwrap();
    validation::validate(&snapshot, &identity, own_key, None).unwrap();
    let mut saved = snapshot.clone();
    saved.payload.revision += 1;
    saved.digest = connection_digest(&saved.payload).unwrap();
    validation::validate(&saved, &identity, own_key, Some(&snapshot)).unwrap();
    validation::validate(&saved, &identity, own_key, Some(&saved)).unwrap();
    assert!(validation::validate(&snapshot, &identity, own_key, Some(&saved)).is_err());

    let mutations: &[fn(&mut NodeConnections)] = &[
        |p| p.node_id = "4".repeat(64),
        |p| p.public_key = Some(public_key(3)),
        |p| p.public_key = None,
        |p| p.schema_version = 2,
        |p| p.revision = 0,
        |p| p.capabilities[0].semantics = ConnectionSemantics::Datagram,
        |p| p.capabilities[0].can_dial = false,
        |p| p.capabilities[0].can_accept = false,
        |p| p.capabilities.push(p.capabilities[0].clone()),
        |p| p.peers[0].public_key = "0".repeat(64),
        |p| p.peers[0].public_key = p.public_key.clone().unwrap(),
        |p| p.peers[0].node_id = p.node_id.clone(),
        |p| p.peers.push(p.peers[0].clone()),
        |p| {
            p.peers.push(ConnectionPeer {
                node_id: "4".repeat(64),
                public_key: public_key(4),
            })
        },
        |p| p.listeners[0].node_id = "3".repeat(64),
        |p| p.listeners[0].id = "../../private".into(),
        |p| p.listeners[0].bind = "[ff02::1]:8421".into(),
        |p| p.listeners[0].bind = "0.0.0.0:0".into(),
        |p| p.listeners[0].advertised_endpoints[0].host = "0.0.0.0".into(),
        |p| p.listeners[0].advertised_endpoints[0].host = "224.0.0.1".into(),
        |p| p.listeners[0].advertised_endpoints[0].host = "Local.Example".into(),
        |p| p.listeners[0].advertised_endpoints[0].host = "example.com.".into(),
        |p| p.listeners[0].advertised_endpoints[0].host = "invalid_host".into(),
        |p| p.listeners[0].advertised_endpoints[0].host = "[::1]".into(),
        |p| p.listeners[0].advertised_endpoints[0].host = "::ffff:0.0.0.0".into(),
        |p| p.listeners[0].advertised_endpoints[0].port = 0,
        |p| p.listeners[0].advertised_endpoints.clear(),
        |p| p.listeners[0].credential_ref = Some("../../other-node-secret".into()),
        |p| p.listeners[0].credential_ref = None,
        |p| p.outgoing[0].credential_ref = None,
        |p| p.outgoing[0].peer_node_id = "4".repeat(64),
        |p| p.outgoing[0].listener_id = "local".into(),
        |p| p.outgoing[0].endpoint.host = "255.255.255.255".into(),
        |p| p.outgoing[0].endpoint.host = "::ffff:224.0.0.1".into(),
        |p| p.outgoing.push(p.outgoing[0].clone()),
        |p| p.incoming[0].candidate_id = "dial".into(),
        |p| p.incoming[0].listener_id = "remote".into(),
        |p| p.incoming[0].transport = ConnectionTransport::NativeTcp,
        |p| p.incoming[0].peer_node_id = "4".repeat(64),
        |p| p.valid = false,
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut invalid = snapshot.clone();
        mutate(&mut invalid.payload);
        invalid.digest = connection_digest(&invalid.payload).unwrap();
        assert!(
            validation::validate(&invalid, &identity, own_key, None).is_err(),
            "mutation {index}"
        );
    }
    let mut invalid = snapshot.clone();
    invalid.digest = "0".repeat(64);
    assert!(validation::validate(&invalid, &identity, own_key, None).is_err());
    let mut invalid = snapshot.clone();
    invalid.payload.outgoing[0].priority += 1;
    invalid.digest = connection_digest(&invalid.payload).unwrap();
    assert!(validation::validate(&invalid, &identity, own_key, Some(&snapshot)).is_err());
    invalid.payload.revision += 1;
    invalid.payload.peers[0].public_key = public_key(3);
    invalid.digest = connection_digest(&invalid.payload).unwrap();
    assert!(validation::validate(&invalid, &identity, own_key, Some(&snapshot)).is_err());
}

#[test]
fn invalid_source_must_withdraw_all_intent_and_have_bounded_diagnostics() {
    let (identity, mut snapshot) = fixture();
    let own_key = snapshot.payload.public_key.clone().unwrap();
    snapshot.payload.valid = false;
    snapshot.payload.listeners.clear();
    snapshot.payload.outgoing.clear();
    snapshot.payload.incoming.clear();
    snapshot.payload.peers.clear();
    snapshot
        .payload
        .diagnostics
        .push(meshway_protocol::ConnectionDiagnostic {
            item_id: None,
            field: "configuration".into(),
            message: "Source is invalid; all adjacency intent withdrawn".into(),
        });
    snapshot.digest = connection_digest(&snapshot.payload).unwrap();
    validation::validate(&snapshot, &identity, &own_key, None).unwrap();
    snapshot.payload.diagnostics[0].message = "a".repeat(1025);
    snapshot.digest = connection_digest(&snapshot.payload).unwrap();
    assert!(validation::validate(&snapshot, &identity, &own_key, None).is_err());
}

#[test]
fn native_profiles_are_null_and_compiled_capability_advertisement_is_truthful() {
    let (identity, mut snapshot) = fixture();
    let own_key = snapshot.payload.public_key.clone().unwrap();
    snapshot.payload.capabilities[0].transport = ConnectionTransport::NativeTcp;
    snapshot.payload.listeners[0].transport = ConnectionTransport::NativeTcp;
    snapshot.payload.outgoing[0].transport = ConnectionTransport::NativeTcp;
    snapshot.payload.incoming[0].transport = ConnectionTransport::NativeTcp;
    snapshot.digest = connection_digest(&snapshot.payload).unwrap();
    assert!(validation::validate(&snapshot, &identity, &own_key, None).is_err());
    snapshot.payload.listeners[0].credential_ref = None;
    snapshot.payload.outgoing[0].credential_ref = None;
    snapshot.digest = connection_digest(&snapshot.payload).unwrap();
    validation::validate(&snapshot, &identity, &own_key, None).unwrap();
    let actual = super::compiled_connection_capabilities();
    #[cfg(not(feature = "transport-lab"))]
    assert!(actual.is_empty());
    #[cfg(feature = "transport-lab")]
    {
        assert_eq!(actual.len(), 3);
        assert_eq!(
            actual
                .iter()
                .map(|capability| capability.transport)
                .collect::<Vec<_>>(),
            [
                ConnectionTransport::NativeUdp,
                ConnectionTransport::NativeTcp,
                ConnectionTransport::Vless
            ]
        );
        assert!(
            actual
                .iter()
                .all(|capability| capability.can_dial && capability.can_accept)
        );
        assert_eq!(actual[0].semantics, ConnectionSemantics::Datagram);
        assert_eq!(actual[1].semantics, ConnectionSemantics::Stream);
        assert_eq!(actual[2].semantics, ConnectionSemantics::Stream);
    }
}
