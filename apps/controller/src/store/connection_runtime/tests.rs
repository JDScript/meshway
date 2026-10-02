use super::*;
use crate::NetworkOptions;
use crate::store::test_store::TestStore as Store;

fn directory() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

fn enrolled(store: &mut Store, seed: u8) -> (String, String) {
    let token = store.create_token(3600).unwrap();
    let credential = local::credential().unwrap();
    let node = store
        .enroll(
            &token.token,
            &EnrollAgentRequest {
                node_credential: credential.clone(),
                name: format!("node-{seed}"),
                agent_version: "test".into(),
            },
        )
        .unwrap();
    store
        .publish_connection_identity(
            &credential,
            &PublishConnectionIdentityRequest {
                public_key: format!("{seed:02x}{}", "00".repeat(31)),
                capabilities: vec![ConnectionCapability {
                    transport: ConnectionTransport::NativeTcp,
                    can_dial: true,
                    can_accept: true,
                    semantics: ConnectionSemantics::Stream,
                }],
            },
        )
        .unwrap();
    (node.id, credential)
}

fn begin(
    store: &mut Store,
    token: &str,
    instance: u8,
    expected_epoch: u32,
) -> BeginConnectionRuntimeResponse {
    store
        .begin_connection_runtime(
            token,
            &BeginConnectionRuntimeRequest {
                instance_id: format!("{instance:02x}").repeat(32),
                expected_epoch,
            },
        )
        .unwrap()
}

fn applied(store: &Store, token: &str, epoch: u32) -> ReportConnectionRuntimeRequest {
    let snapshot = store.agent_connections(token).unwrap();
    ReportConnectionRuntimeRequest {
        epoch,
        sequence: 1,
        revision: snapshot.payload.revision,
        digest: snapshot.digest,
        state: ConnectionRuntimeState::Applied,
        reason: ConnectionRuntimeReason::None,
        listeners: snapshot
            .payload
            .listeners
            .iter()
            .map(|listener| ConnectionListenerObservation {
                listener_id: listener.id.clone(),
                state: ConnectionListenerState::Bound,
                reason: ConnectionRuntimeReason::None,
            })
            .collect(),
        peers: Vec::new(),
    }
}

fn connected_plan(store: &mut Store, acceptor: &str, dialer: &str) {
    store
        .save_connections(&SaveConnectionsRequest {
            expected_revision: store.revision().unwrap(),
            configuration: ConnectionPlan {
                listeners: vec![ConnectionListener {
                    id: "listen".into(),
                    node_id: acceptor.into(),
                    transport: ConnectionTransport::NativeTcp,
                    bind: "127.0.0.1:19000".into(),
                    advertised_endpoints: vec![ConnectionEndpoint {
                        host: "127.0.0.1".into(),
                        port: 19000,
                    }],
                    credential_ref: None,
                }],
                candidates: vec![ConnectionCandidate {
                    id: "dial".into(),
                    dialer_node_id: dialer.into(),
                    listener_id: "listen".into(),
                    endpoint: ConnectionEndpoint {
                        host: "127.0.0.1".into(),
                        port: 19000,
                    },
                    priority: 0,
                    credential_ref: None,
                }],
            },
        })
        .unwrap();
}

fn peer(peer: &str, initiator: &str) -> ConnectionNeighborObservation {
    ConnectionNeighborObservation {
        peer_node_id: peer.into(),
        transport: ConnectionTransport::NativeTcp,
        initiator_node_id: initiator.into(),
        listener_id: Some("listen".into()),
        candidate_id: Some("dial".into()),
        state: ConnectionNeighborState::Authenticated,
        reason: ConnectionRuntimeReason::None,
        probe_fresh: true,
        probe_rtt_ms: Some(7),
    }
}

#[test]
fn leases_retries_sequences_and_ttl_do_not_change_configuration_or_receipts() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (id, token) = enrolled(&mut store, 9);
    let config = store.agent_connections(&token).unwrap();
    store
        .acknowledge_connections(
            &token,
            &AcknowledgeAgentConnectionsRequest {
                revision: config.payload.revision,
                digest: config.digest.clone(),
            },
        )
        .unwrap();
    let inventory = serde_json::to_value(store.connections().unwrap()).unwrap();
    assert!(store.connection_runtimes().unwrap().nodes.is_empty());
    assert_eq!(begin(&mut store, &token, 1, 0).epoch, 1);
    let initial = store.connection_runtimes().unwrap().nodes.remove(0);
    assert_eq!(initial.state, ConnectionRuntimeState::AwaitingReport);
    assert!(initial.reported_at.is_none() && initial.stale && !initial.current);
    let mut report = applied(&store, &token, 1);
    let first = store.report_connection_runtime(&token, &report).unwrap();
    let fresh = store.connection_runtimes().unwrap().nodes.remove(0);
    assert!(fresh.current && !fresh.stale);
    assert_eq!(fresh.state, ConnectionRuntimeState::Applied);
    assert!(fresh.peers.is_empty()); // Empty plans are genuinely applied without peers.
    assert_eq!(begin(&mut store, &token, 1, 0).epoch, 1);
    assert_eq!(
        store
            .report_connection_runtime(&token, &report)
            .unwrap()
            .reported_at,
        first.reported_at
    );
    let old_time = Utc::now() - chrono::Duration::seconds(61);
    store
        .connection
        .execute(
            "UPDATE connection_runtime_reports SET reported_at = ?1",
            [old_time.to_rfc3339()],
        )
        .unwrap();
    let retry = store.report_connection_runtime(&token, &report).unwrap();
    assert_eq!(retry.reported_at, old_time);
    let old = store.connection_runtimes().unwrap().nodes.remove(0);
    assert!(old.current && old.stale);
    report.sequence = 3;
    store.report_connection_runtime(&token, &report).unwrap();
    let latest = serde_json::to_value(store.connection_runtimes().unwrap()).unwrap();
    report.state = ConnectionRuntimeState::Withdrawn;
    assert!(matches!(
        store.report_connection_runtime(&token, &report),
        Err(Error::Conflict)
    ));
    report.sequence = 2;
    assert!(matches!(
        store.report_connection_runtime(&token, &report),
        Err(Error::Conflict)
    ));
    assert_eq!(
        serde_json::to_value(store.connection_runtimes().unwrap()).unwrap(),
        latest
    );
    assert_eq!(
        serde_json::to_value(store.connections().unwrap()).unwrap(),
        inventory
    );
    assert_eq!(
        store.agent_connections(&token).unwrap().digest,
        config.digest
    );
    assert_eq!(begin(&mut store, &token, 2, 1).epoch, 2);
    let reset = store.connection_runtimes().unwrap().nodes.remove(0);
    assert_eq!(reset.sequence, 0);
    assert!(reset.reported_at.is_none() && reset.peers.is_empty());
    report.sequence = 4;
    assert!(matches!(
        store.report_connection_runtime(&token, &report),
        Err(Error::Conflict)
    ));
    drop(store);
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(begin(&mut store, &token, 2, 1).epoch, 2);
    store
        .connection
        .execute(
            "UPDATE connection_runtime_leases SET epoch = 4294967295 WHERE node_id = ?1",
            [&id],
        )
        .unwrap();
    let report = applied(&store, &token, u32::MAX);
    store.report_connection_runtime(&token, &report).unwrap();
    let last = serde_json::to_value(store.connection_runtimes().unwrap()).unwrap();
    assert!(matches!(
        store.begin_connection_runtime(
            &token,
            &BeginConnectionRuntimeRequest {
                instance_id: "03".repeat(32),
                expected_epoch: u32::MAX,
            }
        ),
        Err(Error::Capacity(_))
    ));
    assert!(matches!(
        store.begin_connection_runtime(
            &token,
            &BeginConnectionRuntimeRequest {
                instance_id: "03".repeat(32),
                expected_epoch: 2,
            }
        ),
        Err(Error::Conflict)
    ));
    assert_eq!(begin(&mut store, &token, 2, 1).epoch, u32::MAX);
    assert_eq!(
        store.get_connection_runtime_epoch(&token).unwrap().epoch,
        u32::MAX
    );
    assert_eq!(
        serde_json::to_value(store.connection_runtimes().unwrap()).unwrap(),
        last
    );
}

#[test]
fn report_scope_states_and_probe_claims_are_validated_against_node_projection() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (acceptor_id, acceptor) = enrolled(&mut store, 9);
    let (dialer_id, dialer) = enrolled(&mut store, 10);
    connected_plan(&mut store, &acceptor_id, &dialer_id);
    begin(&mut store, &acceptor, 1, 0);
    begin(&mut store, &dialer, 2, 0);
    let mut incoming = applied(&store, &acceptor, 1);
    incoming.peers.push(peer(&dialer_id, &dialer_id));
    let mut outgoing = applied(&store, &dialer, 1);
    outgoing.peers.push(peer(&acceptor_id, &dialer_id));
    // No ACK is needed for observation and reporting cannot manufacture one.
    for (token, valid) in [(&acceptor, &incoming), (&dialer, &outgoing)] {
        store.report_connection_runtime(token, valid).unwrap();
    }
    assert!(
        store
            .connections()
            .unwrap()
            .nodes
            .iter()
            .all(|node| node.acknowledged_at.is_none())
    );
    let mut invalid = Vec::new();
    for field in [
        "peer_node_id",
        "initiator_node_id",
        "listener_id",
        "candidate_id",
        "transport",
        "probe_fresh",
        "state",
    ] {
        let mut value = serde_json::to_value(&outgoing).unwrap();
        value["sequence"] = 2.into();
        value["peers"][0][field] = match field {
            "transport" => "native_udp".into(),
            "probe_fresh" => false.into(),
            "state" => "waiting".into(),
            _ => "unconfigured".into(),
        };
        invalid.push(serde_json::from_value::<ReportConnectionRuntimeRequest>(value).unwrap());
    }
    let mut duplicate = outgoing.clone();
    duplicate.sequence = 2;
    duplicate.peers.push(duplicate.peers[0].clone());
    invalid.push(duplicate);
    let mut no_candidate = outgoing.clone();
    no_candidate.sequence = 2;
    no_candidate.peers[0].candidate_id = None;
    invalid.push(no_candidate);
    for state in [
        ConnectionRuntimeState::Disabled,
        ConnectionRuntimeState::Withdrawn,
        ConnectionRuntimeState::Error,
        ConnectionRuntimeState::AwaitingReport,
    ] {
        let mut report = outgoing.clone();
        report.sequence = 2;
        report.state = state;
        invalid.push(report);
    }
    let mut excess = outgoing.clone();
    excess.sequence = 2;
    excess.peers = vec![excess.peers[0].clone(); 257];
    invalid.push(excess);
    for report in invalid {
        assert!(
            matches!(
                store.report_connection_runtime(&dialer, &report),
                Err(Error::Unprocessable(_))
            ),
            "accepted {report:?}"
        );
    }
    for edit in 0..5 {
        let mut report = incoming.clone();
        report.sequence = 2;
        match edit {
            0 => report.listeners.clear(),
            1 => report.listeners[0].listener_id = "unknown".into(),
            2 => report.listeners[0].state = ConnectionListenerState::Error,
            3 => report.listeners.push(report.listeners[0].clone()),
            _ => report.reason = ConnectionRuntimeReason::BindFailed,
        }
        assert!(matches!(
            store.report_connection_runtime(&acceptor, &report),
            Err(Error::Unprocessable(_))
        ));
    }
    incoming.sequence = 2;
    incoming.peers[0].candidate_id = None; // Authenticated incoming channels need not know dialer's candidate ID.
    incoming.peers[0].probe_rtt_ms = None;
    store
        .report_connection_runtime(&acceptor, &incoming)
        .unwrap();
    outgoing.sequence = 2;
    outgoing.peers[0].state = ConnectionNeighborState::Waiting;
    outgoing.peers[0].probe_fresh = false;
    outgoing.peers[0].probe_rtt_ms = None;
    store.report_connection_runtime(&dialer, &outgoing).unwrap();
    assert_eq!(
        store
            .connection_runtimes()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.node_id == dialer_id)
            .unwrap()
            .state,
        ConnectionRuntimeState::Applied
    );
    // Partial reconciliation can report an actual listener failure without claiming applied.
    incoming.sequence = 3;
    incoming.state = ConnectionRuntimeState::Applying;
    incoming.listeners[0].state = ConnectionListenerState::Error;
    incoming.listeners[0].reason = ConnectionRuntimeReason::BindFailed;
    incoming.peers.clear();
    store
        .report_connection_runtime(&acceptor, &incoming)
        .unwrap();
}

#[test]
fn peer_failure_reasons_preserve_applied_state_and_lease_expiry_withdraws_resources() {
    use ConnectionNeighborState as PeerState;
    use ConnectionRuntimeReason as Reason;

    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (acceptor_id, _) = enrolled(&mut store, 9);
    let (dialer_id, dialer) = enrolled(&mut store, 10);
    connected_plan(&mut store, &acceptor_id, &dialer_id);
    begin(&mut store, &dialer, 1, 0);
    let mut report = applied(&store, &dialer, 1);
    report.peers.push(peer(&acceptor_id, &dialer_id));
    let before = serde_json::to_value(store.connections().unwrap()).unwrap();

    for reason in [
        Reason::ProfileUnavailable,
        Reason::ResolveFailed,
        Reason::TransportFailed,
        Reason::AuthenticationFailed,
        Reason::ProbeFailed,
        Reason::InternalError,
    ] {
        report.peers[0] = peer(&acceptor_id, &dialer_id);
        report.peers[0].reason = reason;
        assert!(matches!(
            store.report_connection_runtime(&dialer, &report),
            Err(Error::Unprocessable(_))
        )); // A fresh authenticated probe cannot coexist with a failure reason.
        report.peers[0].probe_fresh = false;
        report.peers[0].probe_rtt_ms = None;
        for state in [PeerState::Waiting, PeerState::Unreachable] {
            report.peers[0].state = state;
            store.report_connection_runtime(&dialer, &report).unwrap();
            let observed = store.connection_runtimes().unwrap().nodes.remove(0);
            assert_eq!(observed.state, ConnectionRuntimeState::Applied);
            assert_eq!(observed.reason, Reason::None);
            assert_eq!(observed.peers[0].reason, reason);
            assert!(observed.current && !observed.stale);
            report.sequence += 1;
        }
    }
    for state in [PeerState::Waiting, PeerState::Unreachable] {
        report.peers[0].state = state;
        for reason in [
            Reason::Disabled,
            Reason::InvalidConfiguration,
            Reason::BindFailed,
            Reason::LeaseSuperseded,
            Reason::LeaseExpired,
            Reason::ShuttingDown,
        ] {
            report.peers[0].reason = reason;
            assert!(matches!(
                store.report_connection_runtime(&dialer, &report),
                Err(Error::Unprocessable(_))
            ));
        }
    }
    report.peers[0].reason = Reason::None;
    assert!(matches!(
        store.report_connection_runtime(&dialer, &report),
        Err(Error::Unprocessable(_))
    )); // Unreachable requires a failure; waiting with no attempt may use none.
    report.peers[0].state = PeerState::Waiting;
    store.report_connection_runtime(&dialer, &report).unwrap();
    report.sequence += 1;
    report.state = ConnectionRuntimeState::Withdrawn;
    report.reason = Reason::LeaseExpired;
    assert!(matches!(
        store.report_connection_runtime(&dialer, &report),
        Err(Error::Unprocessable(_))
    ));
    report.peers.clear();
    store.report_connection_runtime(&dialer, &report).unwrap();
    let observed = store.connection_runtimes().unwrap().nodes.remove(0);
    assert_eq!(observed.reason, Reason::LeaseExpired);
    assert_eq!(observed.state, ConnectionRuntimeState::Withdrawn);
    assert!(observed.peers.is_empty());
    assert_eq!(
        serde_json::to_value(store.connections().unwrap()).unwrap(),
        before
    );
}

#[test]
fn configuration_mismatch_withdrawal_and_revocation_never_leave_current_authenticated_claims() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (acceptor_id, acceptor) = enrolled(&mut store, 9);
    let (dialer_id, dialer) = enrolled(&mut store, 10);
    connected_plan(&mut store, &acceptor_id, &dialer_id);
    begin(&mut store, &acceptor, 1, 0);
    begin(&mut store, &dialer, 2, 0);
    let mut report = applied(&store, &dialer, 1);
    report.peers.push(peer(&acceptor_id, &dialer_id));
    store.report_connection_runtime(&dialer, &report).unwrap();
    let mut bad = report.clone();
    bad.sequence += 1;
    bad.digest = "00".repeat(32);
    assert!(matches!(
        store.report_connection_runtime(&dialer, &bad),
        Err(Error::Unprocessable(_))
    ));
    bad.digest = report.digest.clone();
    bad.revision += 1;
    assert!(matches!(
        store.report_connection_runtime(&dialer, &bad),
        Err(Error::Unprocessable(_))
    ));
    report.sequence = 2;
    report.state = ConnectionRuntimeState::Withdrawn;
    report.reason = ConnectionRuntimeReason::ShuttingDown;
    report.peers.clear();
    store.report_connection_runtime(&dialer, &report).unwrap();
    let withdrawn = store
        .connection_runtimes()
        .unwrap()
        .nodes
        .into_iter()
        .find(|node| node.node_id == dialer_id)
        .unwrap();
    assert!(withdrawn.current && !withdrawn.stale && withdrawn.peers.is_empty());
    report.sequence = 3;
    report.state = ConnectionRuntimeState::Applied;
    report.reason = ConnectionRuntimeReason::None;
    report.peers.push(peer(&acceptor_id, &dialer_id));
    store.report_connection_runtime(&dialer, &report).unwrap();
    store.revoke(&acceptor_id).unwrap();
    assert!(matches!(
        store.report_connection_runtime(&dialer, &report),
        Err(Error::Unprocessable(_))
    ));
    let historical = store
        .connection_runtimes()
        .unwrap()
        .nodes
        .into_iter()
        .find(|node| node.node_id == dialer_id)
        .unwrap();
    assert!(!historical.current && historical.stale);
    let config = store.agent_connections(&dialer).unwrap();
    assert!(!config.payload.valid);
    report.revision = config.payload.revision;
    report.digest = config.digest;
    report.sequence = 4;
    report.peers.clear();
    assert!(matches!(
        store.report_connection_runtime(&dialer, &report),
        Err(Error::Unprocessable(_))
    ));
    report.state = ConnectionRuntimeState::Withdrawn;
    report.reason = ConnectionRuntimeReason::InvalidConfiguration;
    store.report_connection_runtime(&dialer, &report).unwrap();
    store.revoke(&dialer_id).unwrap();
    let revoked = store
        .connection_runtimes()
        .unwrap()
        .nodes
        .into_iter()
        .find(|node| node.node_id == dialer_id)
        .unwrap();
    assert_eq!(revoked.state, ConnectionRuntimeState::Withdrawn);
    assert!(
        revoked.peers.is_empty()
            && revoked.reported_at.is_none()
            && !revoked.current
            && revoked.stale
    );
    assert!(matches!(
        store.report_connection_runtime(&dialer, &report),
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        store.begin_connection_runtime(
            &dialer,
            &BeginConnectionRuntimeRequest {
                instance_id: "01".repeat(32),
                expected_epoch: 0,
            }
        ),
        Err(Error::Forbidden)
    ));
}

fn existing_rows(connection: &Connection) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    let tables: Vec<String> = connection.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'connection_runtime_%' AND name NOT IN ('network_identity','networks','latency_samples','latency_runtime_epochs') ORDER BY name")
        .unwrap().query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    tables
        .into_iter()
        .map(|table| {
            let mut query = connection
                .prepare(&format!(
                    "SELECT {} FROM {table} ORDER BY 1",
                    crate::store::test_store::original_columns(connection, &table)
                ))
                .unwrap();
            let columns = query.column_count();
            let rows = query
                .query_map([], |row| (0..columns).map(|index| row.get(index)).collect())
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (table, rows)
        })
        .collect()
}

#[test]
fn v3_to_v4_preserves_every_existing_sql_value_and_schema1_payload_bytes() {
    let dir = directory();
    let mut store = Store::open_v8(dir.path(), &NetworkOptions::default()).unwrap();
    let (id, token) = enrolled(&mut store, 9);
    let (peer_id, peer_token) = enrolled(&mut store, 10);
    connected_plan(&mut store, &id, &peer_id);
    let before = store.agent_connections(&token).unwrap();
    store
        .acknowledge_connections(
            &token,
            &AcknowledgeAgentConnectionsRequest {
                revision: before.payload.revision,
                digest: before.digest.clone(),
            },
        )
        .unwrap();
    let ip = store.configuration(&token).unwrap();
    store
        .acknowledge(
            &token,
            &AcknowledgeConfigurationRequest {
                revision: ip.payload.revision,
                digest: ip.digest,
            },
        )
        .unwrap();
    store.create_token(3600).unwrap();
    store.revoke(&peer_id).unwrap();
    let before = serde_json::to_vec(&store.agent_connections(&token).unwrap()).unwrap();
    let rows = existing_rows(&store.connection);
    let admin_before = std::fs::read(dir.path().join("admin.token")).unwrap();
    store.downgrade_v8();
    store.connection.execute_batch("ALTER TABLE enrollment_tokens DROP COLUMN device_binding; DROP TABLE network_identity; DROP TABLE mesh_access_source; DROP TABLE mesh_access_legacy_v5; DROP TABLE node_tags; DROP TABLE tag_definitions; DROP TABLE connection_runtime_reports; DROP TABLE connection_runtime_leases; ALTER TABLE configuration DROP COLUMN source_jsonc; PRAGMA user_version=3;").unwrap();
    drop(store);
    let mut reopened = Store::open_v8(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(existing_rows(&reopened.connection), rows);
    assert_eq!(
        serde_json::to_vec(&reopened.agent_connections(&token).unwrap()).unwrap(),
        before
    );
    assert_eq!(
        std::fs::read(dir.path().join("admin.token")).unwrap(),
        admin_before
    );
    assert_eq!(
        reopened
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        10
    );
    assert!(reopened.connection_runtimes().unwrap().nodes.is_empty());
    assert!(matches!(
        reopened.heartbeat(&peer_token, "test"),
        Err(Error::Forbidden)
    ));
    assert_eq!(begin(&mut reopened, &token, 1, 0).epoch, 1);
    assert_eq!(existing_rows(&reopened.connection), rows);
}
