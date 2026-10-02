use super::*;
use crate::{
    NetworkOptions,
    store::test_support::{SingleNetworkFixture, directory, enroll, request},
};

fn node(
    store: &mut SingleNetworkFixture,
    seed: u8,
    transport: ConnectionTransport,
) -> (NodeSummary, String, MeshExecutionRequest) {
    let request = request(&format!("publication-{seed}"));
    let node = enroll(store, &request);
    store
        .publish_connection_identity(
            &request.node_credential,
            &PublishConnectionIdentityRequest {
                public_key: format!("{seed:02x}{}", "00".repeat(31)),
                capabilities: vec![ConnectionCapability {
                    transport,
                    semantics: if transport == ConnectionTransport::NativeUdp {
                        ConnectionSemantics::Datagram
                    } else {
                        ConnectionSemantics::Stream
                    },
                    can_dial: true,
                    can_accept: true,
                }],
            },
        )
        .unwrap();
    let instance_id = local::credential().unwrap();
    let epoch = store
        .begin_connection_runtime(
            &request.node_credential,
            &BeginConnectionRuntimeRequest {
                instance_id: instance_id.clone(),
                expected_epoch: 0,
            },
        )
        .unwrap()
        .epoch;
    let runtime = MeshExecutionRequest {
        epoch,
        instance_id,
        capabilities: vec![MeshExecutionCapability::Ipv4ExactPathV1],
    };
    store
        .mesh_execution(&request.node_credential, &runtime)
        .unwrap();
    if transport == ConnectionTransport::Vless {
        store
            .publish_managed_capabilities(
                &request.node_credential,
                &PublishManagedTransportCapabilitiesRequest {
                    vless_streams: vec![VlessStreamTransport::Tcp],
                },
            )
            .unwrap();
    }
    (node, request.node_credential, runtime)
}

fn save(
    store: &mut SingleNetworkFixture,
    policy: NetworkPolicy,
) -> Result<crate::policy_v3_api::NetworkPolicySnapshot, Error> {
    let scope = store.network_id.clone();
    let expected_revision = store.revision().unwrap();
    store.save_network_policy(
        &scope,
        &SaveNetworkPolicyRequest {
            expected_revision,
            policy,
            source_jsonc: None,
        },
    )
}

fn listener(id: usize, owner: &str) -> ManagedListenerSettings {
    ManagedListenerSettings {
        id: format!("material-{id}"),
        node_id: owner.into(),
        enabled: true,
        transport: ConnectionTransport::Vless,
        bind_address: "0.0.0.0".into(),
        bind_port: 20000 + u16::try_from(id).unwrap(),
        advertised_endpoints: vec![ConnectionEndpoint {
            host: "192.0.2.1".into(),
            port: 20000 + u16::try_from(id).unwrap(),
        }],
        vless: Some(ManagedVlessSettings {
            stream: VlessStreamTransport::Tcp,
            server_name: "meshway.test".into(),
            websocket_path: None,
            websocket_host: None,
        }),
    }
}

fn add_listener(
    store: &mut SingleNetworkFixture,
    settings: ManagedListenerSettings,
) -> Result<ManagedListenersSnapshot, Error> {
    let scope = store.network_id.clone();
    let expected_revision = store.revision().unwrap();
    store.save_managed_listener(
        &scope,
        &SaveManagedListenerRequest {
            expected_revision,
            settings,
        },
    )
}

#[test]
fn blocked_management_get_save_preserves_intent_and_unblock_restores_same_candidate() {
    let dir = directory();
    let mut store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    let a = node(&mut store, 9, ConnectionTransport::NativeUdp);
    let b = node(&mut store, 10, ConnectionTransport::NativeUdp);
    let endpoint = ConnectionEndpoint {
        host: "192.0.2.1".into(),
        port: 41000,
    };
    let candidate = ConnectionCandidate {
        id: "retained".into(),
        dialer_node_id: b.0.id.clone(),
        listener_id: "hub".into(),
        endpoint: endpoint.clone(),
        priority: 0,
        credential_ref: None,
    };
    store
        .save_connections(&SaveConnectionsRequest {
            expected_revision: store.revision().unwrap(),
            configuration: ConnectionPlan {
                listeners: vec![ConnectionListener {
                    id: "hub".into(),
                    node_id: a.0.id.clone(),
                    transport: ConnectionTransport::NativeUdp,
                    bind: "0.0.0.0:41000".into(),
                    advertised_endpoints: vec![endpoint],
                    credential_ref: None,
                }],
                candidates: vec![candidate.clone()],
            },
        })
        .unwrap();
    let mut blocked = NetworkPolicy::default_template();
    blocked.connections.push(v3::ConnectionRule {
        id: "block".into(),
        a: format!("node:{}", a.0.id),
        b: format!("node:{}", b.0.id),
        action: v3::ConnectionAction::Block,
    });
    save(&mut store, blocked).unwrap();
    let edit = store.connections().unwrap();
    assert_eq!(edit.configuration.candidates, vec![candidate.clone()]);
    assert!(
        store
            .agent_connections(&b.1)
            .unwrap()
            .payload
            .outgoing
            .is_empty()
    );
    let before = edit.revision;
    store
        .save_connections(&SaveConnectionsRequest {
            expected_revision: before,
            configuration: edit.configuration,
        })
        .unwrap();
    assert_eq!(store.revision().unwrap(), before); // Honest no-op roundtrip.
    save(&mut store, NetworkPolicy::default_template()).unwrap();
    assert_eq!(
        store.connections().unwrap().configuration.candidates,
        vec![candidate]
    );
    assert_eq!(
        store.agent_connections(&b.1).unwrap().payload.outgoing[0].id,
        "retained"
    );
    let current = store.mesh_execution(&b.1, &b.2).unwrap().snapshot.unwrap();
    assert!(current.payload.valid);
    assert!(!current.payload.paths.is_empty());
}

#[test]
fn owner_65th_material_rolls_back_listener_source_and_revision() {
    let dir = directory();
    let mut store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    let a = node(&mut store, 9, ConnectionTransport::Vless);
    save(&mut store, NetworkPolicy::default_template()).unwrap();
    for i in 0..64 {
        add_listener(&mut store, listener(i, &a.0.id)).unwrap();
    }
    let revision = store.revision().unwrap();
    let scope = store.network_id.clone();
    let before = store.mesh_execution(&a.1, &a.2).unwrap().snapshot.unwrap();
    assert_eq!(before.payload.materials.len(), 64);
    assert!(matches!(
        add_listener(&mut store, listener(64, &a.0.id)),
        Err(Error::Capacity(_))
    ));
    assert_eq!(store.revision().unwrap(), revision);
    assert_eq!(store.managed_listeners(&scope).unwrap().listeners.len(), 64);
    let after = store.mesh_execution(&a.1, &a.2).unwrap().snapshot.unwrap();
    assert_eq!(after.digest, before.digest);
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM managed_listeners WHERE listener_id='material-64'",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn legacy_65_materials_cannot_activate_v3_or_download_above_contract() {
    let dir = directory();
    let mut store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    let a = node(&mut store, 9, ConnectionTransport::Vless);
    for i in 0..65 {
        add_listener(&mut store, listener(i, &a.0.id)).unwrap();
    }
    let scope = store.network_id.clone();
    let revision = store.revision().unwrap();
    let legacy = store.policy().unwrap();
    let intent = store.connections().unwrap();
    assert!(matches!(
        save(&mut store, NetworkPolicy::default_template()),
        Err(Error::Capacity(_))
    ));
    assert_eq!(store.revision().unwrap(), revision);
    assert!(!active(&store.connection, &scope).unwrap());
    assert_eq!(store.policy().unwrap().policy, legacy.policy);
    assert_eq!(
        store.connections().unwrap().configuration,
        intent.configuration
    );
    // Defensive download projection has the same count gate even without the
    // mutation gate; no fixture-only activation or malformed database writes.
    let policy = NetworkPolicy::default_template();
    let publication =
        super::super::publication::mesh_v3::Publication::read(&store.connection, &scope, &policy)
            .unwrap();
    assert!(matches!(
        publication.project(&a.0.id, &store.material_key),
        Err(Error::Capacity(_))
    ));
}

#[test]
fn candidate_65th_dialer_material_rolls_back_and_multiple_endpoints_count_once() {
    let dir = directory();
    let mut store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    let a = node(&mut store, 9, ConnectionTransport::Vless);
    let b = node(&mut store, 10, ConnectionTransport::Vless);
    let dialer = node(&mut store, 11, ConnectionTransport::Vless);
    save(&mut store, NetworkPolicy::default_template()).unwrap();
    for i in 0..65 {
        let mut settings = listener(i, if i < 64 { &a.0.id } else { &b.0.id });
        if i == 0 {
            settings.advertised_endpoints.push(ConnectionEndpoint {
                host: "192.0.2.2".into(),
                port: 20000,
            });
        }
        add_listener(&mut store, settings).unwrap();
    }
    let mut intent = store.connections().unwrap().configuration;
    let candidate = |i: usize, id: String| ConnectionCandidate {
        id,
        dialer_node_id: dialer.0.id.clone(),
        listener_id: format!("material-{i}"),
        endpoint: ConnectionEndpoint {
            host: "192.0.2.1".into(),
            port: 20000 + u16::try_from(i).unwrap(),
        },
        priority: 0,
        credential_ref: None,
    };
    intent.candidates = (0..64).map(|i| candidate(i, format!("dial-{i}"))).collect();
    let mut second_endpoint = candidate(0, "second-endpoint-same-material".into());
    second_endpoint.endpoint.host = "192.0.2.2".into();
    intent.candidates.push(second_endpoint);
    store
        .save_connections(&SaveConnectionsRequest {
            expected_revision: store.revision().unwrap(),
            configuration: intent,
        })
        .unwrap();
    let before = store
        .mesh_execution(&dialer.1, &dialer.2)
        .unwrap()
        .snapshot
        .unwrap();
    assert_eq!(before.payload.materials.len(), 64);
    assert!(
        before
            .payload
            .materials
            .iter()
            .all(|m| m.private_key_pem.is_none())
    );
    let edit = store.connections().unwrap();
    let saved = edit.configuration.clone();
    let revision = edit.revision;
    let mut too_many = edit.configuration;
    too_many.candidates.push(candidate(64, "dial-64".into()));
    assert!(matches!(
        store.save_connections(&SaveConnectionsRequest {
            expected_revision: revision,
            configuration: too_many
        }),
        Err(Error::Capacity(_))
    ));
    assert_eq!(store.revision().unwrap(), revision);
    assert_eq!(store.connections().unwrap().configuration, saved);
    assert_eq!(
        store
            .mesh_execution(&dialer.1, &dialer.2)
            .unwrap()
            .snapshot
            .unwrap()
            .digest,
        before.digest
    );
}
