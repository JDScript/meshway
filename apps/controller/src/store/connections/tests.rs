use super::*;
use crate::NetworkOptions;
use crate::store::test_support::SingleNetworkFixture;

fn directory() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

fn enrolled(store: &mut SingleNetworkFixture, name: &str) -> (String, String) {
    let token = store.create_token(3600).unwrap();
    let credential = local::credential().unwrap();
    let node = store
        .enroll(
            &token.token,
            &EnrollAgentRequest {
                node_credential: credential.clone(),
                name: name.into(),
                agent_version: "test".into(),
            },
        )
        .unwrap();
    (node.id, credential)
}

fn identity(seed: u8) -> PublishConnectionIdentityRequest {
    PublishConnectionIdentityRequest {
        public_key: format!("{seed:02x}{}", "00".repeat(31)),
        capabilities: vec![
            ConnectionCapability {
                transport: ConnectionTransport::Vless,
                can_dial: true,
                can_accept: true,
                semantics: ConnectionSemantics::Stream,
            },
            ConnectionCapability {
                transport: ConnectionTransport::NativeUdp,
                can_dial: true,
                can_accept: true,
                semantics: ConnectionSemantics::Datagram,
            },
            ConnectionCapability {
                transport: ConnectionTransport::NativeTcp,
                can_dial: true,
                can_accept: true,
                semantics: ConnectionSemantics::Stream,
            },
        ],
    }
}

fn plan(lax: &str, sha: &str) -> ConnectionPlan {
    ConnectionPlan {
        listeners: vec![ConnectionListener {
            id: "lax-in".into(),
            node_id: lax.into(),
            transport: ConnectionTransport::Vless,
            bind: "0.0.0.0:443".into(),
            advertised_endpoints: vec![ConnectionEndpoint {
                host: "LAX.Example.COM.".into(),
                port: 443,
            }],
            credential_ref: Some("lax-accept-profile".into()),
        }],
        candidates: vec![ConnectionCandidate {
            id: "sha-dials-lax".into(),
            dialer_node_id: sha.into(),
            listener_id: "lax-in".into(),
            endpoint: ConnectionEndpoint {
                host: "lax.example.com".into(),
                port: 443,
            },
            priority: 10,
            credential_ref: Some("sha-dial-profile".into()),
        }],
    }
}

fn save(
    store: &mut SingleNetworkFixture,
    plan: ConnectionPlan,
) -> Result<ConnectionsSnapshot, Error> {
    store.save_connections(&SaveConnectionsRequest {
        expected_revision: store.revision().unwrap(),
        configuration: plan,
    })
}

#[test]
fn immutable_unique_binding_capability_updates_and_revision_exhaustion_are_atomic() {
    let dir = directory();
    let mut store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (one_id, one) = enrolled(&mut store, "one");
    let (_, two) = enrolled(&mut store, "two");
    let first = identity(9);
    let before = store.revision().unwrap();
    let bound = store.publish_connection_identity(&one, &first).unwrap();
    assert_eq!(bound.node_id, one_id);
    assert_eq!(bound.revision, before + 1);
    assert_eq!(
        bound.capabilities[0].transport,
        ConnectionTransport::NativeUdp
    );
    let mut reordered = first.clone();
    reordered.capabilities.reverse();
    assert_eq!(
        store
            .publish_connection_identity(&one, &reordered)
            .unwrap()
            .revision,
        bound.revision
    );
    assert!(matches!(
        store.publish_connection_identity(&one, &identity(10)),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        store.publish_connection_identity(&two, &first),
        Err(Error::Conflict)
    ));
    // Distinct hex strings for basepoint aliases must never evade SQL uniqueness.
    for alias in [
        format!("f6{}7f", "ff".repeat(30)),
        format!("09{}80", "00".repeat(30)),
    ] {
        let mut request = first.clone();
        request.public_key = alias;
        assert!(matches!(
            store.publish_connection_identity(&two, &request),
            Err(Error::Invalid(_))
        ));
        assert_eq!(store.revision().unwrap(), bound.revision);
    }
    reordered.capabilities[0].can_dial = false;
    assert_eq!(
        store
            .publish_connection_identity(&one, &reordered)
            .unwrap()
            .revision,
        bound.revision + 1
    );
    store.revoke(&one_id).unwrap();
    assert!(matches!(
        store.publish_connection_identity(&one, &first),
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        store.publish_connection_identity(&two, &first),
        Err(Error::Conflict)
    ));
    let tombstone = store
        .connections()
        .unwrap()
        .nodes
        .into_iter()
        .find(|node| node.node.id == one_id)
        .unwrap();
    assert_eq!(
        tombstone.public_key.as_deref(),
        Some(first.public_key.as_str())
    );
    assert!(tombstone.node.revoked_at.is_some());
    store
        .connection
        .execute(
            "UPDATE configuration SET revision = 4294967295 WHERE id = 1",
            [],
        )
        .unwrap();
    assert!(matches!(
        store.publish_connection_identity(&two, &identity(10)),
        Err(Error::Capacity(_))
    ));
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM connection_identities", [], |row| row
                .get::<_, u32>(
                0
            ))
            .unwrap(),
        1
    );
    drop(store);
    let mut reopened = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    assert!(matches!(
        reopened.publish_connection_identity(&two, &first),
        Err(Error::Conflict)
    ));
}

#[test]
fn reverse_dial_projection_is_scoped_deterministic_and_acknowledged_independently() {
    let dir = directory();
    let mut store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (lax_id, lax) = enrolled(&mut store, "LAX");
    let (sha_id, sha) = enrolled(&mut store, "SHA");
    let (_, other) = enrolled(&mut store, "other");
    for (credential, seed) in [(&lax, 9), (&sha, 10), (&other, 11)] {
        store
            .publish_connection_identity(credential, &identity(seed))
            .unwrap();
    }
    let saved = save(&mut store, plan(&lax_id, &sha_id)).unwrap();
    assert!(saved.valid);
    assert_eq!(
        saved.configuration.listeners[0].advertised_endpoints[0].host,
        "lax.example.com"
    );
    let sha_snapshot = store.agent_connections(&sha).unwrap();
    assert_eq!(sha_snapshot.payload.outgoing.len(), 1);
    assert!(sha_snapshot.payload.listeners.is_empty());
    assert!(sha_snapshot.payload.incoming.is_empty());
    assert_eq!(sha_snapshot.payload.peers[0].node_id, lax_id);
    let lax_snapshot = store.agent_connections(&lax).unwrap();
    assert_eq!(lax_snapshot.payload.listeners.len(), 1);
    assert_eq!(lax_snapshot.payload.incoming.len(), 1);
    assert!(lax_snapshot.payload.outgoing.is_empty());
    assert_eq!(lax_snapshot.payload.peers[0].node_id, sha_id);
    let unrelated = store.agent_connections(&other).unwrap();
    assert!(unrelated.payload.peers.is_empty());
    assert!(unrelated.payload.listeners.is_empty());
    assert!(unrelated.payload.outgoing.is_empty());
    let sha_json = serde_json::to_string(&sha_snapshot).unwrap();
    let lax_json = serde_json::to_string(&lax_snapshot).unwrap();
    assert!(!sha_json.contains("lax-accept-profile"));
    assert!(!lax_json.contains("sha-dial-profile"));
    for secret in [&lax, &sha, &other] {
        assert!(!sha_json.contains(secret));
    }
    for transient in [
        "acknowledged",
        "last_seen",
        "registered_at",
        "agent_version",
    ] {
        assert!(!sha_json.contains(transient));
    }
    let ack = AcknowledgeAgentConnectionsRequest {
        revision: saved.revision,
        digest: sha_snapshot.digest.clone(),
    };
    for (revision, digest) in [
        (saved.revision - 1, ack.digest.clone()),
        (saved.revision + 1, ack.digest.clone()),
        (saved.revision, lax_snapshot.digest.clone()),
        (saved.revision, "00".repeat(32)),
    ] {
        assert!(matches!(
            store.acknowledge_connections(
                &sha,
                &AcknowledgeAgentConnectionsRequest { revision, digest }
            ),
            Err(Error::Conflict)
        ));
    }
    let ip_snapshot = store.configuration(&sha).unwrap();
    store
        .acknowledge(
            &sha,
            &AcknowledgeConfigurationRequest {
                revision: ip_snapshot.payload.revision,
                digest: ip_snapshot.digest,
            },
        )
        .unwrap();
    assert_eq!(
        store
            .connections()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.node.id == sha_id)
            .unwrap()
            .acknowledged_revision,
        0
    );
    let result = store.acknowledge_connections(&sha, &ack).unwrap();
    assert_eq!(result.status, ConnectionReceiptStatus::Saved);
    let receipt = store
        .connections()
        .unwrap()
        .nodes
        .into_iter()
        .find(|node| node.node.id == sha_id)
        .unwrap();
    store.acknowledge_connections(&sha, &ack).unwrap();
    assert_eq!(
        store
            .connections()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.node.id == sha_id)
            .unwrap()
            .acknowledged_at,
        receipt.acknowledged_at
    );
    assert_eq!(
        store
            .connections()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.node.id == lax_id)
            .unwrap()
            .acknowledged_revision,
        0
    );
    store.heartbeat(&sha, "new-version").unwrap();
    assert_eq!(store.revision().unwrap(), saved.revision);
    assert_eq!(
        store.agent_connections(&sha).unwrap().digest,
        sha_snapshot.digest
    );
    assert_eq!(
        save(&mut store, plan(&lax_id, &sha_id)).unwrap().revision,
        saved.revision
    );
    assert!(matches!(
        store.save_connections(&SaveConnectionsRequest {
            expected_revision: saved.revision - 1,
            configuration: saved.configuration.clone()
        }),
        Err(Error::Conflict)
    ));
    drop(store);
    let mut reopened = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(
        reopened.agent_connections(&sha).unwrap().digest,
        sha_snapshot.digest
    );
    assert_eq!(
        reopened
            .connections()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.node.id == sha_id)
            .unwrap()
            .acknowledged_at,
        receipt.acknowledged_at
    );
    reopened.revoke(&lax_id).unwrap();
    let invalid = reopened.connections().unwrap();
    assert!(!invalid.valid);
    assert!(!invalid.diagnostics.is_empty());
    assert_eq!(invalid.configuration, saved.configuration);
    let withdrawn = reopened.agent_connections(&sha).unwrap();
    assert!(!withdrawn.payload.valid);
    assert!(
        withdrawn.payload.listeners.is_empty()
            && withdrawn.payload.outgoing.is_empty()
            && withdrawn.payload.incoming.is_empty()
            && withdrawn.payload.peers.is_empty()
    );
    assert!(matches!(
        reopened.acknowledge_connections(&sha, &ack),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        reopened.agent_connections(&lax),
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        reopened.acknowledge_connections(&lax, &ack),
        Err(Error::Forbidden)
    ));
    assert!(
        save(&mut reopened, ConnectionPlan::default())
            .unwrap()
            .valid
    );
}

#[test]
fn validation_rejects_unbound_roles_endpoints_references_duplicates_and_bad_profiles() {
    let dir = directory();
    let mut store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (lax_id, lax) = enrolled(&mut store, "LAX");
    let (sha_id, sha) = enrolled(&mut store, "SHA");
    assert!(matches!(
        save(&mut store, plan(&lax_id, &sha_id)),
        Err(Error::Unprocessable(_))
    ));
    store
        .publish_connection_identity(&lax, &identity(9))
        .unwrap();
    store
        .publish_connection_identity(&sha, &identity(10))
        .unwrap();
    let base = plan(&lax_id, &sha_id);
    let mut cases = Vec::new();
    for host in [
        "0.0.0.0",
        "::",
        "224.0.0.1",
        "ff02::1",
        "255.255.255.255",
        "::ffff:224.0.0.1",
        "*.example.com",
        "bad host",
        "127.1",
        "http://example.com",
        "-bad.example",
        "example..com",
        "0.0.0.0.",
        "0x0",
        "0xe0000001",
        "0177.0.0.1",
    ] {
        let mut value = base.clone();
        value.listeners[0].advertised_endpoints[0].host = host.into();
        cases.push(value);
    }
    let mut value = base.clone();
    value.listeners[0].advertised_endpoints[0].port = 0;
    cases.push(value);
    let mut value = base.clone();
    value.listeners[0].bind = "0.0.0.0:0".into();
    cases.push(value);
    let mut value = base.clone();
    value.listeners[0].bind = "example.com:443".into();
    cases.push(value);
    let mut value = base.clone();
    value.listeners[0].credential_ref = None;
    cases.push(value);
    let mut value = base.clone();
    value.candidates[0].credential_ref = Some("secret/invalid".into());
    cases.push(value);
    let mut value = base.clone();
    value.listeners[0].id = "bad id".into();
    cases.push(value);
    let mut value = base.clone();
    value.listeners[0].node_id = "missing".into();
    cases.push(value);
    let mut value = base.clone();
    value.candidates[0].dialer_node_id = lax_id.clone();
    cases.push(value);
    let mut value = base.clone();
    value.candidates[0].listener_id = "missing".into();
    cases.push(value);
    let mut value = base.clone();
    value.candidates[0].endpoint.port = 444;
    cases.push(value);
    let mut value = base.clone();
    let mut duplicate = value.candidates[0].clone();
    duplicate.id = "duplicate".into();
    value.candidates.push(duplicate);
    cases.push(value);
    let mut value = base.clone();
    value.listeners.push(value.listeners[0].clone());
    cases.push(value);
    let mut value = base.clone();
    value.listeners[0]
        .advertised_endpoints
        .push(ConnectionEndpoint {
            host: "lax.example.com".into(),
            port: 443,
        });
    cases.push(value);
    let mut value = base.clone();
    value.listeners[0].transport = ConnectionTransport::NativeTcp;
    cases.push(value);
    let revision = store.revision().unwrap();
    for case in cases {
        assert!(
            matches!(save(&mut store, case.clone()), Err(Error::Unprocessable(_))),
            "accepted {case:?}"
        );
        assert_eq!(store.revision().unwrap(), revision);
    }
    let mut unicode = base.clone();
    unicode.listeners[0].advertised_endpoints[0].host = "BÜCHER.example.".into();
    unicode.candidates[0].endpoint.host = "xn--bcher-kva.example".into();
    assert_eq!(
        save(&mut store, unicode).unwrap().configuration.listeners[0].advertised_endpoints[0].host,
        "xn--bcher-kva.example"
    );
    let mut native = base.clone();
    native.listeners[0].transport = ConnectionTransport::NativeUdp;
    native.listeners[0].credential_ref = None;
    native.candidates[0].credential_ref = None;
    native.listeners[0].advertised_endpoints[0].host = "127.0.0.1".into();
    native.candidates[0].endpoint.host = "127.0.0.1".into();
    save(&mut store, native).unwrap();
    let mut withdrawn = identity(9);
    withdrawn
        .capabilities
        .retain(|capability| capability.transport != ConnectionTransport::NativeUdp);
    store.publish_connection_identity(&lax, &withdrawn).unwrap();
    assert!(!store.connections().unwrap().valid);
    assert!(!store.agent_connections(&sha).unwrap().payload.valid);
    store
        .publish_connection_identity(&lax, &identity(9))
        .unwrap();
    assert!(store.connections().unwrap().valid);
    let mut no_dial = identity(10);
    no_dial
        .capabilities
        .iter_mut()
        .for_each(|capability| capability.can_dial = false);
    store.publish_connection_identity(&sha, &no_dial).unwrap();
    assert!(!store.connections().unwrap().valid);
    // A standalone listener is permitted and grants no peer adjacency.
    let mut listener_only = base;
    listener_only.candidates.clear();
    save(&mut store, listener_only).unwrap();
    let listener = store.agent_connections(&lax).unwrap().payload;
    assert_eq!(listener.listeners.len(), 1);
    assert!(listener.peers.is_empty() && listener.incoming.is_empty());
}
