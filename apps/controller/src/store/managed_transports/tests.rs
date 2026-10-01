use super::*;
use crate::{
    NetworkOptions,
    store::tests::{directory, request},
};

fn settings(node_id: &str) -> ManagedListenerSettings {
    ManagedListenerSettings {
        id: "vless".into(),
        node_id: node_id.into(),
        enabled: true,
        transport: ConnectionTransport::Vless,
        bind_address: "0.0.0.0".into(),
        bind_port: 18443,
        advertised_endpoints: vec![ConnectionEndpoint {
            host: "localhost".into(),
            port: 28443,
        }],
        vless: Some(ManagedVlessSettings {
            stream: VlessStreamTransport::Websocket,
            server_name: "meshway.test".into(),
            websocket_path: Some("/meshway".into()),
            websocket_host: Some("meshway.test".into()),
        }),
    }
}
fn enroll(store: &mut Store, network: &NetworkId, name: &str, key: &str) -> (NodeSummary, String) {
    let request = request(name);
    let credential = request.node_credential.clone();
    let token = store.create_token(network, 600).unwrap();
    let node = store.enroll(&token.token, &request).unwrap();
    store
        .publish_connection_identity(
            &credential,
            &PublishConnectionIdentityRequest {
                public_key: key.into(),
                capabilities: vec![ConnectionCapability {
                    transport: ConnectionTransport::Vless,
                    can_dial: true,
                    can_accept: true,
                    semantics: ConnectionSemantics::Stream,
                }],
            },
        )
        .unwrap();
    store
        .publish_managed_capabilities(
            &credential,
            &PublishManagedTransportCapabilitiesRequest {
                vless_streams: vec![VlessStreamTransport::Tcp, VlessStreamTransport::Websocket],
            },
        )
        .unwrap();
    (node, credential)
}

#[test]
fn materials_are_encrypted_scope_minimized_revision_coherent_and_stable_after_restart() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let network = store.initial_network_id().unwrap();
    let (a, ca) = enroll(
        &mut store,
        &network,
        "listener",
        &format!("09{}", "00".repeat(31)),
    );
    let (b, cb) = enroll(
        &mut store,
        &network,
        "dialer",
        &format!("0a{}", "00".repeat(31)),
    );
    let (_, cc) = enroll(
        &mut store,
        &network,
        "unrelated",
        &format!("0b{}", "00".repeat(31)),
    );
    let revision = store.revision(&network).unwrap();
    let saved = store
        .save_managed_listener(
            &network,
            &SaveManagedListenerRequest {
                expected_revision: revision,
                settings: settings(&a.id),
            },
        )
        .unwrap();
    assert_eq!(saved.revision, revision + 1);
    assert_eq!(saved.listeners[0].material_version, 1);
    let public = serde_json::to_string(&saved).unwrap();
    assert!(!public.contains("private_key"));
    assert!(!public.contains("uuid"));
    assert!(!public.contains("BEGIN CERTIFICATE"));
    let mut plan = store.connections(&network).unwrap().configuration;
    plan.candidates.push(ConnectionCandidate {
        id: "dial".into(),
        dialer_node_id: b.id.clone(),
        listener_id: "vless".into(),
        endpoint: ConnectionEndpoint {
            host: "localhost".into(),
            port: 28443,
        },
        priority: 10,
        credential_ref: Some("managed.vless".into()),
    });
    store
        .save_connections(
            &network,
            &SaveConnectionsRequest {
                expected_revision: saved.revision,
                configuration: plan,
            },
        )
        .unwrap();
    let owner = store.agent_managed_dataplane(&ca).unwrap();
    let dialer = store.agent_managed_dataplane(&cb).unwrap();
    let unrelated = store.agent_managed_dataplane(&cc).unwrap();
    assert_eq!(owner.materials.len(), 1);
    assert_eq!(dialer.materials.len(), 1);
    assert!(unrelated.materials.is_empty());
    assert!(owner.materials[0].private_key_pem.is_some());
    assert!(dialer.materials[0].private_key_pem.is_none());
    assert_eq!(owner.materials[0].uuid, dialer.materials[0].uuid);
    assert_eq!(
        owner.digest,
        managed_digest(&owner.network_id, &owner.dataplane, &owner.materials).unwrap()
    );
    assert_eq!(
        owner.dataplane.payload.revision,
        owner.dataplane.payload.connections.revision
    );
    assert!(
        !serde_json::to_string(&store.connections(&network).unwrap())
            .unwrap()
            .contains(&owner.materials[0].uuid)
    );
    let ciphertext: Vec<u8> = store
        .connection
        .query_row(
            "SELECT material_ciphertext FROM managed_listeners WHERE network_id=?1",
            [network.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !ciphertext
            .windows(owner.materials[0].uuid.len())
            .any(|w| w == owner.materials[0].uuid.as_bytes())
    );
    let other = store
        .create_network(&CreateNetworkRequest {
            name: "isolated".into(),
        })
        .unwrap();
    let other_id = store.network_id(&other.network_id).unwrap();
    assert!(matches!(
        store.save_managed_listener(
            &other_id,
            &SaveManagedListenerRequest {
                expected_revision: other.revision,
                settings: settings(&a.id)
            }
        ),
        Err(Error::NotFound)
    ));
    assert!(
        store
            .managed_listeners(&other_id)
            .unwrap()
            .listeners
            .is_empty()
    );
    drop(store);
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let restored = store.agent_managed_dataplane(&ca).unwrap();
    assert_eq!(owner.digest, restored.digest);
    assert_eq!(owner.materials[0].uuid, restored.materials[0].uuid);
    let mut changed = settings(&a.id);
    changed.bind_port = 18444;
    changed.vless.as_mut().unwrap().websocket_path = Some("/next".into());
    let next = store
        .save_managed_listener(
            &network,
            &SaveManagedListenerRequest {
                expected_revision: owner.dataplane.payload.revision,
                settings: changed.clone(),
            },
        )
        .unwrap();
    assert_eq!(next.listeners[0].material_version, 2);
    let updated = store.agent_managed_dataplane(&ca).unwrap();
    assert_eq!(updated.materials[0].uuid, owner.materials[0].uuid);
    assert_eq!(
        updated.materials[0].certificate_pem,
        owner.materials[0].certificate_pem
    );
    assert_ne!(updated.digest, owner.digest);
    assert_eq!(
        store
            .save_managed_listener(
                &network,
                &SaveManagedListenerRequest {
                    expected_revision: next.revision,
                    settings: changed.clone()
                }
            )
            .unwrap()
            .revision,
        next.revision
    );
    assert!(matches!(
        store.save_managed_listener(
            &network,
            &SaveManagedListenerRequest {
                expected_revision: next.revision - 1,
                settings: changed
            }
        ),
        Err(Error::Conflict)
    ));
    store.revoke(&network, &a.id).unwrap();
    assert!(matches!(
        store.agent_managed_dataplane(&ca),
        Err(Error::Forbidden)
    ));
    assert!(
        store
            .agent_managed_dataplane(&cb)
            .unwrap()
            .materials
            .is_empty()
    );
    drop(store);
    std::fs::remove_file(dir.path().join("transport-material.key")).unwrap();
    assert!(Store::open(dir.path(), &NetworkOptions::default()).is_err());
}

#[test]
fn initial_intent_consumption_is_atomic_identity_bound_and_cannot_restore_revocation() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let network = store.initial_network_id().unwrap();
    let token = store
        .create_managed_token(
            &network,
            &CreateManagedEnrollmentTokenRequest {
                expires_in_seconds: 600,
                initial_listener: settings(""),
            },
        )
        .unwrap();
    let revision = store.revision(&network).unwrap();
    let request = request("initial");
    let node = store.enroll(&token.token, &request).unwrap();
    assert_eq!(store.revision(&network).unwrap(), revision + 1);
    let listeners = store.managed_listeners(&network).unwrap();
    assert_eq!(listeners.listeners[0].settings.node_id, node.id);
    assert!(!listeners.listeners[0].compiled_capability_available);
    assert_eq!(store.enroll(&token.token, &request).unwrap().id, node.id);
    assert_eq!(store.revision(&network).unwrap(), revision + 1);
    let mut different = super::super::tests::request("initial");
    different.node_credential = local::credential().unwrap();
    assert!(matches!(
        store.enroll(&token.token, &different),
        Err(Error::Unauthorized)
    ));
    let another = store
        .create_managed_token(
            &network,
            &CreateManagedEnrollmentTokenRequest {
                expires_in_seconds: 600,
                initial_listener: settings(""),
            },
        )
        .unwrap();
    let attempted = super::super::tests::request("collision");
    assert!(matches!(
        store.enroll(&another.token, &attempted),
        Err(Error::Conflict)
    ));
    assert_eq!(store.nodes(&network).unwrap().len(), 1);
    store.revoke(&network, &node.id).unwrap();
    assert!(matches!(
        store.enroll(&token.token, &request),
        Err(Error::Forbidden)
    ));
    assert_eq!(store.nodes(&network).unwrap().len(), 1);
    let bound: String = store
        .connection
        .query_row(
            "SELECT used_node_id FROM enrollment_tokens WHERE id=?1",
            [token.token_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bound, node.id);
}

#[test]
fn migration_preserves_existing_rows_revision_and_rolls_back_all_ddl() {
    let dir = directory();
    let store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let network = store.initial_network_id().unwrap();
    let revision = store.revision(&network).unwrap();
    let db = &store.connection;
    db.execute_batch("DROP TABLE managed_enrollment_intents; DROP TABLE managed_transport_capabilities; DROP TABLE managed_listeners; DROP INDEX enrollment_tokens_network_id; PRAGMA user_version=9;").unwrap();
    drop(store);
    let store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(store.revision(&network).unwrap(), revision);
    assert!(
        store
            .managed_listeners(&network)
            .unwrap()
            .listeners
            .is_empty()
    );
    drop(store);
    let mut db = Connection::open(dir.path().join("controller.sqlite3")).unwrap();
    db.execute_batch("DROP TABLE managed_enrollment_intents; DROP TABLE managed_transport_capabilities; DROP TABLE managed_listeners; DROP INDEX enrollment_tokens_network_id; CREATE TABLE managed_enrollment_intents(blocker TEXT); PRAGMA user_version=9;").unwrap();
    assert!(upgrade(&mut db).is_err());
    assert_eq!(
        db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        9
    );
    assert!(
        !db.prepare("SELECT 1 FROM sqlite_master WHERE name='managed_listeners'")
            .unwrap()
            .exists([])
            .unwrap()
    );
}

#[test]
fn closed_settings_reject_unknown_transport_tls_ws_fields_and_bad_binding() {
    let mut value = serde_json::to_value(settings("node")).unwrap();
    value["vless"]["unexpected"] = true.into();
    assert!(serde_json::from_value::<ManagedListenerSettings>(value).is_err());
    let mut value = serde_json::to_value(settings("node")).unwrap();
    value["transport"] = "vless_ws_unimplemented".into();
    assert!(serde_json::from_value::<ManagedListenerSettings>(value).is_err());
    let mut item = settings("node");
    item.vless.as_mut().unwrap().stream = VlessStreamTransport::Tcp;
    assert!(validate(&item).is_err());
    item = settings("node");
    item.advertised_endpoints[0].host = "0.0.0.0".into();
    assert!(validate(&item).is_err());
}

#[test]
fn offline_import_preserves_legacy_uuid_certificate_key_and_refuses_non_equivalent_listener() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let network = store.initial_network_id().unwrap();
    let (node, credential) = enroll(
        &mut store,
        &network,
        "legacy",
        &format!("09{}", "00".repeat(31)),
    );
    let mut item = settings(&node.id);
    let vless = item.vless.as_mut().unwrap();
    vless.stream = VlessStreamTransport::Tcp;
    vless.websocket_path = None;
    vless.websocket_host = None;
    let legacy = ConnectionListener {
        id: item.id.clone(),
        node_id: node.id.clone(),
        transport: item.transport,
        bind: "0.0.0.0:18443".into(),
        advertised_endpoints: item.advertised_endpoints.clone(),
        credential_ref: Some("old-local-server".into()),
    };
    let saved = store
        .save_connections(
            &network,
            &SaveConnectionsRequest {
                expected_revision: store.revision(&network).unwrap(),
                configuration: ConnectionPlan {
                    listeners: vec![legacy],
                    candidates: vec![],
                },
            },
        )
        .unwrap();
    let material = generate(&item.id, 1, item.vless.as_ref().unwrap()).unwrap();
    let uuid = material.uuid.clone();
    let cert = material.certificate_pem.clone();
    let key = material.private_key_pem.clone();
    let profiles = dir.path().join("legacy-profiles.json");
    local::atomic_write(&dir.path().join("legacy-cert.pem"), cert.as_bytes(), false).unwrap();
    local::atomic_write(
        &dir.path().join("legacy-key.pem"),
        key.as_ref().unwrap().as_bytes(),
        false,
    )
    .unwrap();
    let file = serde_json::json!({"version":1,"profiles":[{"role":"server","id":"old-local-server","uuid":uuid,"certificate":"legacy-cert.pem","private_key":"legacy-key.pem"}]});
    local::atomic_write(&profiles, &serde_json::to_vec(&file).unwrap(), false).unwrap();
    store
        .verify_legacy_material(&network, &item, &material, &profiles)
        .unwrap();
    let mut replacement = material.clone();
    replacement.uuid = "00000000-0000-4000-a000-000000000000".into();
    assert!(
        store
            .verify_legacy_material(&network, &item, &replacement, &profiles)
            .is_err()
    );
    let mut incompatible = item.clone();
    incompatible.bind_port += 1;
    assert!(matches!(
        store.import_managed_listener(
            &network,
            &SaveManagedListenerRequest {
                expected_revision: saved.revision,
                settings: incompatible
            },
            material.clone()
        ),
        Err(Error::Conflict)
    ));
    assert_eq!(store.revision(&network).unwrap(), saved.revision);
    let mut invalid = material.clone();
    invalid.certificate_sha256 = "0".repeat(64);
    assert!(
        store
            .import_managed_listener(
                &network,
                &SaveManagedListenerRequest {
                    expected_revision: saved.revision,
                    settings: item.clone()
                },
                invalid
            )
            .is_err()
    );
    store
        .import_managed_listener(
            &network,
            &SaveManagedListenerRequest {
                expected_revision: saved.revision,
                settings: item,
            },
            material,
        )
        .unwrap();
    let imported = store.agent_managed_connections(&credential).unwrap();
    assert_eq!(imported.materials[0].uuid, uuid);
    assert_eq!(imported.materials[0].certificate_pem, cert);
    assert_eq!(imported.materials[0].private_key_pem, key);
    assert_eq!(
        connections::source(&store.connection, &network)
            .unwrap()
            .listeners
            .len(),
        0
    );
    assert_eq!(
        imported.connections.payload.listeners[0]
            .credential_ref
            .as_deref(),
        Some("managed.vless")
    );
}
