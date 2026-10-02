use super::*;
use crate::NetworkOptions;
use crate::store::migrations::latency::validate_schema;
use std::os::unix::fs::PermissionsExt;

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    network: NetworkId,
    source: String,
    peer: String,
    token: String,
    peer_token: String,
    sample: LatencySample,
}

fn enroll(store: &mut Store, network: &NetworkId, seed: u8) -> (String, String) {
    let enrollment = store.create_token(network, 3600).unwrap();
    let token = local::credential().unwrap();
    let node = store
        .enroll(
            &enrollment.token,
            &EnrollAgentRequest {
                node_credential: token.clone(),
                name: format!("node-{seed}"),
                agent_version: "test".into(),
            },
        )
        .unwrap();
    store
        .publish_connection_identity(
            &token,
            &PublishConnectionIdentityRequest {
                public_key: format!("{seed:02x}").repeat(32),
                capabilities: vec![ConnectionCapability {
                    transport: ConnectionTransport::NativeTcp,
                    can_dial: true,
                    can_accept: true,
                    semantics: ConnectionSemantics::Stream,
                }],
            },
        )
        .unwrap();
    (node.id, token)
}

fn applied(store: &mut Store, token: &str, epoch: u32) {
    let snapshot = store.agent_connections(token).unwrap();
    store
        .report_connection_runtime(
            token,
            &ReportConnectionRuntimeRequest {
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
                    .map(|l| ConnectionListenerObservation {
                        listener_id: l.id.clone(),
                        state: ConnectionListenerState::Bound,
                        reason: ConnectionRuntimeReason::None,
                    })
                    .collect(),
                peers: vec![],
            },
        )
        .unwrap();
}

fn fixture() -> Fixture {
    let dir = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let network = store.initial_network_id().unwrap();
    let (source, token) = enroll(&mut store, &network, 11);
    let (peer, peer_token) = enroll(&mut store, &network, 12);
    store
        .save_connections(
            &network,
            &SaveConnectionsRequest {
                expected_revision: store.revision(&network).unwrap(),
                configuration: ConnectionPlan {
                    listeners: vec![ConnectionListener {
                        id: "listen".into(),
                        node_id: source.clone(),
                        transport: ConnectionTransport::NativeTcp,
                        bind: "127.0.0.1:19001".into(),
                        advertised_endpoints: vec![ConnectionEndpoint {
                            host: "127.0.0.1".into(),
                            port: 19001,
                        }],
                        credential_ref: None,
                    }],
                    candidates: vec![ConnectionCandidate {
                        id: "dial".into(),
                        dialer_node_id: peer.clone(),
                        listener_id: "listen".into(),
                        endpoint: ConnectionEndpoint {
                            host: "127.0.0.1".into(),
                            port: 19001,
                        },
                        priority: 0,
                        credential_ref: None,
                    }],
                },
            },
        )
        .unwrap();
    store
        .begin_connection_runtime(
            &token,
            &BeginConnectionRuntimeRequest {
                expected_epoch: 0,
                instance_id: "a1".repeat(32),
            },
        )
        .unwrap();
    applied(&mut store, &token, 1);
    let snapshot = store.agent_connections(&token).unwrap();
    let now = Utc::now();
    let sample = LatencySample {
        measurement_id: local::credential().unwrap(),
        sampler_generation: local::credential().unwrap(),
        peer_node_id: peer.clone(),
        direction: LatencyDirection::SourceToPeer,
        runtime_epoch: 1,
        revision: snapshot.payload.revision,
        digest: snapshot.digest,
        measured_at: now,
        completed_at: now + chrono::Duration::milliseconds(2),
        outcome: LatencyOutcome::Success,
        failure_reasons: vec![],
        rtt_us: Some(1000),
        channel_associations: vec![LatencyChannelAssociation {
            transport: ConnectionTransport::NativeTcp,
            initiator_node_id: peer.clone(),
            listener_id: "listen".into(),
            candidate_id: None,
        }],
    };
    Fixture {
        dir,
        store,
        network,
        source,
        peer,
        token,
        peer_token,
        sample,
    }
}

fn query(f: &Fixture) -> QueryLatencyHistoryRequest {
    QueryLatencyHistoryRequest {
        source_node_id: f.source.clone(),
        peer_node_id: f.peer.clone(),
        from: Utc::now() - chrono::Duration::hours(1),
        until: Utc::now() + chrono::Duration::seconds(3),
        limit: 100,
        after: None,
    }
}

fn send(
    f: &mut Fixture,
    samples: Vec<LatencySample>,
) -> Result<ReportLatencySamplesResponse, Error> {
    f.store
        .report_latency_samples(&f.token, &ReportLatencySamplesRequest { samples })
}

#[test]
fn immutable_idempotency_atomic_batches_no_heartbeat_samples_and_restart() {
    let mut f = fixture();
    let q = query(&f);
    assert!(
        f.store
            .latency_history(&f.network, &q)
            .unwrap()
            .records
            .is_empty()
    );
    let sample = f.sample.clone();
    assert_eq!(
        send(&mut f, vec![sample.clone(), sample.clone()])
            .unwrap()
            .inserted,
        1
    );
    let first = f
        .store
        .latency_history(&f.network, &q)
        .unwrap()
        .records
        .remove(0);
    assert_eq!(send(&mut f, vec![sample.clone()]).unwrap().duplicates, 1);
    let second = f
        .store
        .latency_history(&f.network, &q)
        .unwrap()
        .records
        .remove(0);
    assert_eq!(first.received_at, second.received_at);
    let mut changed = sample.clone();
    changed.rtt_us = Some(1200);
    let mut fresh = sample.clone();
    fresh.measurement_id = local::credential().unwrap();
    assert!(matches!(
        send(&mut f, vec![fresh, changed]),
        Err(Error::Conflict)
    ));
    assert_eq!(
        f.store
            .latency_history(&f.network, &q)
            .unwrap()
            .records
            .len(),
        1
    );
    f.store.heartbeat(&f.token, "test").unwrap();
    assert_eq!(
        f.store.latency_history(&f.network, &q).unwrap().records[0].received_at,
        first.received_at
    );
    drop(f.store);
    let mut reopened = Store::open(f.dir.path(), &NetworkOptions::default()).unwrap();
    let history = reopened.latency_history(&f.network, &q).unwrap();
    assert_eq!(history.records.len(), 1);
    assert_eq!(history.records[0].sample, sample);
    assert!(history.records[0].historical);
}

#[test]
fn clock_scope_epoch_withdrawal_and_revocation_fail_closed() {
    let mut f = fixture();
    for delta in [-121, 6] {
        let mut sample = f.sample.clone();
        sample.measured_at = Utc::now() + chrono::Duration::seconds(delta);
        sample.completed_at = sample.measured_at + chrono::Duration::milliseconds(2);
        assert!(matches!(
            send(&mut f, vec![sample]),
            Err(Error::Unprocessable(_))
        ));
    }
    let mut sample = f.sample.clone();
    sample.measured_at = Utc::now() - chrono::Duration::seconds(10);
    sample.completed_at = sample.measured_at + chrono::Duration::milliseconds(2);
    assert!(matches!(
        send(&mut f, vec![sample]),
        Err(Error::Unprocessable(_))
    ));
    let mut sample = f.sample.clone();
    sample.completed_at = sample.measured_at - chrono::Duration::seconds(1);
    assert!(matches!(
        send(&mut f, vec![sample]),
        Err(Error::Unprocessable(_))
    ));
    let mut sample = f.sample.clone();
    sample.channel_associations[0].initiator_node_id = "unrelated".into();
    assert!(matches!(
        send(&mut f, vec![sample]),
        Err(Error::Unprocessable(_))
    ));
    let other = f
        .store
        .create_network(&CreateNetworkRequest {
            name: "other".into(),
        })
        .unwrap();
    let other = f.store.network_id(&other.network_id).unwrap();
    let (cross, _) = enroll(&mut f.store, &other, 13);
    let mut sample = f.sample.clone();
    sample.peer_node_id = cross;
    assert!(matches!(
        send(&mut f, vec![sample]),
        Err(Error::Unprocessable(_))
    ));
    let sample = f.sample.clone();
    send(&mut f, vec![sample.clone()]).unwrap();
    f.store
        .begin_connection_runtime(
            &f.token,
            &BeginConnectionRuntimeRequest {
                expected_epoch: 1,
                instance_id: "a2".repeat(32),
            },
        )
        .unwrap();
    assert!(matches!(
        send(&mut f, vec![sample.clone()]),
        Err(Error::Conflict)
    ));
    applied(&mut f.store, &f.token, 2);
    assert!(matches!(send(&mut f, vec![sample]), Err(Error::Conflict)));
    let q = query(&f);
    assert!(f.store.latency_history(&f.network, &q).unwrap().records[0].historical);
    f.store.revoke(&f.network, &f.source).unwrap();
    let sample = f.sample.clone();
    assert!(matches!(send(&mut f, vec![sample]), Err(Error::Forbidden)));
    assert_eq!(
        f.store
            .latency_history(&f.network, &q)
            .unwrap()
            .records
            .len(),
        1
    );
}

#[test]
fn timeout_null_gaps_pagination_direction_and_query_isolation() {
    let mut f = fixture();
    let mut timeout = f.sample.clone();
    timeout.measurement_id = local::credential().unwrap();
    timeout.outcome = LatencyOutcome::Timeout;
    timeout.rtt_us = None;
    assert!(matches!(
        send(&mut f, vec![timeout.clone()]),
        Err(Error::Unprocessable(_))
    ));
    timeout.channel_associations.clear();
    let sample = f.sample.clone();
    send(&mut f, vec![sample, timeout]).unwrap();
    let mut q = query(&f);
    q.limit = 1;
    let page = f.store.latency_history(&f.network, &q).unwrap();
    assert_eq!(page.records.len(), 1);
    q.after = page.next;
    let page = f.store.latency_history(&f.network, &q).unwrap();
    assert_eq!(page.records[0].sample.outcome, LatencyOutcome::Timeout);
    assert!(page.records[0].sample.rtt_us.is_none());
    assert!(page.next.is_none());
    q.after = None;
    std::mem::swap(&mut q.source_node_id, &mut q.peer_node_id);
    assert!(
        f.store
            .latency_history(&f.network, &q)
            .unwrap()
            .records
            .is_empty()
    );
    q.limit = 1001;
    assert!(matches!(
        f.store.latency_history(&f.network, &q),
        Err(Error::Unprocessable(_))
    ));
    q.limit = 1;
    q.from = q.until - chrono::Duration::hours(25);
    assert!(matches!(
        f.store.latency_history(&f.network, &q),
        Err(Error::Unprocessable(_))
    ));
    let other = f
        .store
        .create_network(&CreateNetworkRequest {
            name: "isolated".into(),
        })
        .unwrap();
    let other = f.store.network_id(&other.network_id).unwrap();
    let q = query(&f);
    assert!(matches!(
        f.store.latency_history(&other, &q),
        Err(Error::NotFound)
    ));
    let error = f
        .store
        .authorized_latency_history(&f.peer_token, f.network.as_str(), &q)
        .unwrap_err();
    assert_eq!(
        axum::response::IntoResponse::into_response(error).status(),
        axum::http::StatusCode::UNAUTHORIZED
    );
}

#[test]
fn cleanup_is_bounded_and_expired_events_are_never_returned() {
    let mut f = fixture();
    let sample = f.sample.clone();
    send(&mut f, vec![sample]).unwrap();
    let old = Utc::now() - chrono::Duration::days(8);
    for n in 0..1100 {
        f.store.connection.execute("INSERT INTO latency_samples(network_id,source_node_id,peer_node_id,measurement_id,runtime_epoch,measured_ms,received_ms,body) VALUES (?1,?2,?3,?4,1,?5,?5,X'00')", params![f.network.as_str(),f.source,f.peer,format!("expired-{n}"),old.timestamp_millis()]).unwrap();
    }
    assert_eq!(
        cleanup(&f.store.connection, Utc::now()).unwrap(),
        CLEANUP_BUDGET as usize
    );
    let q = query(&f);
    assert_eq!(
        f.store
            .latency_history(&f.network, &q)
            .unwrap()
            .records
            .len(),
        1
    );
    assert_eq!(cleanup(&f.store.connection, Utc::now()).unwrap(), 0);
}

#[test]
fn stale_and_withdrawn_runtime_cannot_extend_sampling_lease() {
    let mut f = fixture();
    f.store
        .connection
        .execute(
            "UPDATE connection_runtime_reports SET reported_at=?1",
            [(Utc::now() - chrono::Duration::seconds(61)).to_rfc3339()],
        )
        .unwrap();
    let sample = f.sample.clone();
    assert!(matches!(send(&mut f, vec![sample]), Err(Error::Conflict)));
    let snapshot = f.store.agent_connections(&f.token).unwrap();
    f.store
        .report_connection_runtime(
            &f.token,
            &ReportConnectionRuntimeRequest {
                epoch: 1,
                sequence: 2,
                revision: snapshot.payload.revision,
                digest: snapshot.digest,
                state: ConnectionRuntimeState::Withdrawn,
                reason: ConnectionRuntimeReason::ShuttingDown,
                peers: vec![],
                listeners: vec![],
            },
        )
        .unwrap();
    let sample = f.sample.clone();
    assert!(matches!(send(&mut f, vec![sample]), Err(Error::Conflict)));
    assert!(
        f.store
            .latency_history(&f.network, &query(&f))
            .unwrap()
            .records
            .is_empty()
    );
}

#[test]
fn report_budget_and_failure_outcomes_are_explicit() {
    let mut f = fixture();
    let mut failed = f.sample.clone();
    failed.outcome = LatencyOutcome::Failed;
    failed.rtt_us = None;
    failed.channel_associations.clear();
    assert!(matches!(
        send(&mut f, vec![failed.clone()]),
        Err(Error::Unprocessable(_))
    ));
    failed.failure_reasons = vec![LatencyFailureReason::ChannelUnavailable];
    send(&mut f, vec![failed.clone()]).unwrap();
    for count in [64, 64, 64, 64, 64, 64, 64, 63] {
        let mut events = Vec::new();
        for _ in 0..count {
            let mut sample = f.sample.clone();
            sample.measurement_id = local::credential().unwrap();
            events.push(sample);
        }
        send(&mut f, events).unwrap();
    }
    assert_eq!(send(&mut f, vec![failed]).unwrap().duplicates, 1);
    let mut extra = f.sample.clone();
    extra.measurement_id = local::credential().unwrap();
    assert!(matches!(send(&mut f, vec![extra]), Err(Error::Capacity(_))));
    let q = query(&f);
    assert_eq!(
        f.store
            .latency_history(&f.network, &q)
            .unwrap()
            .records
            .len(),
        100
    );
}

#[test]
fn out_of_order_events_remain_distinct_and_loss_of_adjacency_blocks_new_writes() {
    let mut f = fixture();
    let earlier = f.sample.clone();
    let mut later = earlier.clone();
    later.measurement_id = local::credential().unwrap();
    later.measured_at += chrono::Duration::milliseconds(100);
    later.completed_at += chrono::Duration::milliseconds(100);
    send(&mut f, vec![later.clone()]).unwrap();
    send(&mut f, vec![earlier.clone()]).unwrap();
    let q = query(&f);
    let h = f.store.latency_history(&f.network, &q).unwrap();
    assert_eq!(h.records[0].sample, later);
    assert_eq!(h.records[1].sample, earlier);
    f.store
        .save_connections(
            &f.network,
            &SaveConnectionsRequest {
                expected_revision: f.store.revision(&f.network).unwrap(),
                configuration: ConnectionPlan::default(),
            },
        )
        .unwrap();
    let mut extra = f.sample.clone();
    extra.measurement_id = local::credential().unwrap();
    assert!(matches!(send(&mut f, vec![extra]), Err(Error::Conflict)));
    assert_eq!(
        f.store
            .latency_history(&f.network, &q)
            .unwrap()
            .records
            .len(),
        2
    );
}

fn seed_full(f: &mut Fixture) -> String {
    let mut sample = f.sample.clone();
    sample.measured_at = Utc::now() - chrono::Duration::hours(1);
    sample.completed_at = sample.measured_at + chrono::Duration::milliseconds(2);
    let tx = f.store.connection.transaction().unwrap();
    {
        let mut statement = tx.prepare("INSERT INTO latency_samples(network_id,source_node_id,peer_node_id,measurement_id,runtime_epoch,measured_ms,received_ms,body) VALUES (?1,?2,?3,?4,1,?5,?6,?7)").unwrap();
        for i in 0..NODE_ROW_BUDGET {
            sample.measurement_id = format!("{i:064x}");
            sample.measured_at -= chrono::Duration::milliseconds(1);
            sample.completed_at -= chrono::Duration::milliseconds(1);
            statement
                .execute(params![
                    f.network.as_str(),
                    f.source,
                    f.peer,
                    sample.measurement_id,
                    sample.measured_at.timestamp_millis(),
                    (Utc::now() - chrono::Duration::minutes(3)).timestamp_millis(),
                    canonical_json(&sample).unwrap()
                ])
                .unwrap();
        }
    }
    tx.commit().unwrap();
    sample.measurement_id
}

#[test]
fn rolling_quota_admits_latest_at_cap_and_rolls_eviction_back_with_bad_batch() {
    let mut f = fixture();
    let oldest = seed_full(&mut f);
    let mut sample = f.sample.clone();
    for _ in 0..3 {
        sample.measurement_id = local::credential().unwrap();
        assert_eq!(send(&mut f, vec![sample.clone()]).unwrap().inserted, 1);
    }
    let count: u32 = f
        .store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM latency_samples WHERE source_node_id=?1",
            [&f.source],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, NODE_ROW_BUDGET);
    assert!(
        !f.store
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM latency_samples WHERE measurement_id=?1)",
                [oldest],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
    let before: i64 = f
        .store
        .connection
        .query_row("SELECT MAX(row_id) FROM latency_samples", [], |r| r.get(0))
        .unwrap();
    assert!(before > i64::from(NODE_ROW_BUDGET));
    let mut fresh = sample.clone();
    fresh.measurement_id = local::credential().unwrap();
    let mut invalid = fresh.clone();
    invalid.measurement_id = local::credential().unwrap();
    invalid.peer_node_id = "f".repeat(64);
    assert!(send(&mut f, vec![fresh.clone(), invalid]).is_err());
    assert_eq!(
        f.store
            .connection
            .query_row("SELECT MAX(row_id) FROM latency_samples", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(send(&mut f, vec![sample.clone()]).unwrap().duplicates, 1);
    sample.rtt_us = Some(1200);
    assert!(matches!(send(&mut f, vec![sample]), Err(Error::Conflict)));
    assert_eq!(send(&mut f, vec![fresh]).unwrap().inserted, 1);
    let mut q = query(&f);
    q.from = Utc::now() - chrono::Duration::hours(2);
    q.limit = 1;
    let page = f.store.latency_history(&f.network, &q).unwrap();
    assert!(page.capacity_trimmed && page.capacity_trimmed_before.is_some());
    q.after = page.next;
    let mut extra = f.sample.clone();
    extra.measurement_id = local::credential().unwrap();
    send(&mut f, vec![extra]).unwrap();
    let page2 = f.store.latency_history(&f.network, &q).unwrap();
    assert!(page2.capacity_trimmed);
    assert!(page2.capacity_trimmed_before >= page.capacity_trimmed_before);
}

#[test]
fn retry_identity_and_insert_rate_survive_eviction_and_expire_explicitly() {
    let mut f = fixture();
    let sample = f.sample.clone();
    send(&mut f, vec![sample.clone()]).unwrap();
    // Exercise the post-eviction path: row removal cannot remove admission ID,
    // immutable hash, original receipt or the rolling-minute insert counter.
    f.store
        .connection
        .execute("DELETE FROM latency_samples", [])
        .unwrap();
    assert_eq!(send(&mut f, vec![sample.clone()]).unwrap().duplicates, 1);
    let mut changed = sample.clone();
    changed.completed_at += chrono::Duration::milliseconds(1);
    assert!(matches!(send(&mut f, vec![changed]), Err(Error::Conflict)));
    for n in 0..511 {
        let id = format!("{n:064x}");
        f.store.connection.execute("INSERT INTO latency_admissions(network_id,source_node_id,measurement_id,body_digest,received_ms,expires_ms) VALUES (?1,?2,?3,?4,?5,?6)",params![f.network.as_str(),f.source,id,vec![0u8;32],Utc::now().timestamp_millis(),(Utc::now()+chrono::Duration::seconds(121)).timestamp_millis()]).unwrap();
    }
    let mut fresh = sample.clone();
    fresh.measurement_id = local::credential().unwrap();
    assert!(matches!(send(&mut f, vec![fresh]), Err(Error::Capacity(_))));
    // Beyond the retry/rate window, an evicted ID has no perpetual tombstone.
    // A retimed new event can reuse it; retained rows still enforce identity.
    f.store
        .connection
        .execute(
            "UPDATE latency_admissions SET expires_ms=?1,received_ms=?2",
            params![
                (Utc::now() - chrono::Duration::seconds(1)).timestamp_millis(),
                (Utc::now() - chrono::Duration::seconds(61)).timestamp_millis()
            ],
        )
        .unwrap();
    let mut reused = sample;
    reused.measured_at = Utc::now();
    reused.completed_at = reused.measured_at + chrono::Duration::milliseconds(2);
    assert_eq!(send(&mut f, vec![reused]).unwrap().inserted, 1);
}

#[test]
fn incompatible_schema_is_detected_without_side_effects() {
    let f = fixture();
    validate_schema(&f.store.connection).unwrap();
    f.store.connection.execute_batch("ALTER TABLE latency_samples RENAME TO original_samples; CREATE TABLE latency_samples AS SELECT * FROM original_samples WHERE 0; DROP TABLE original_samples;").unwrap();
    let before: String = f
        .store
        .connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='latency_samples'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(validate_schema(&f.store.connection).is_err());
    let after: String = f
        .store
        .connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='latency_samples'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
    drop(f.store);
    assert!(Store::open(f.dir.path(), &NetworkOptions::default()).is_err());
}

#[test]
fn history_trigger_set_is_exact_pure_and_does_not_reject_base_triggers() {
    let f = fixture();
    f.store
        .connection
        .execute_batch(
            "CREATE TRIGGER legitimate_base_fixture AFTER INSERT ON nodes BEGIN SELECT 1; END;",
        )
        .unwrap();
    validate_schema(&f.store.connection).unwrap();
    for table in [
        "latency_samples",
        "latency_admissions",
        "latency_coverage",
        "latency_runtime_epochs",
    ] {
        for body in ["SELECT RAISE(IGNORE);", "SELECT 1;"] {
            f.store.connection.execute_batch(&format!("CREATE TRIGGER same_name_history_guard BEFORE INSERT ON {table} BEGIN {body} END;")).unwrap();
            let before: String = f
                .store
                .connection
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name='same_name_history_guard'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(
                validate_schema(&f.store.connection).is_err(),
                "unexpected {table} trigger must be rejected"
            );
            let after: String = f
                .store
                .connection
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name='same_name_history_guard'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                before, after,
                "validation must not remove/repair the trigger"
            );
            f.store
                .connection
                .execute_batch("DROP TRIGGER same_name_history_guard;")
                .unwrap();
            validate_schema(&f.store.connection).unwrap();
        }
        f.store.connection.execute_batch(&format!("CREATE TEMP TRIGGER temporary_history_guard BEFORE INSERT ON main.{table} BEGIN SELECT RAISE(IGNORE); END;")).unwrap();
        assert!(validate_schema(&f.store.connection).is_err());
        f.store
            .connection
            .execute_batch("DROP TRIGGER temporary_history_guard;")
            .unwrap();
        validate_schema(&f.store.connection).unwrap();
    }
    // SQLite permits trigger/index names to collide. Separate object collections
    // must retain the trigger rather than letting the expected index hide it.
    f.store.connection.execute_batch("CREATE TRIGGER latency_source_oldest BEFORE INSERT ON latency_samples BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(validate_schema(&f.store.connection).is_err());
    assert_eq!(f.store.connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='latency_source_oldest' AND type IN ('index','trigger')",[],|r|r.get::<_,u32>(0)).unwrap(),2);
    f.store
        .connection
        .execute_batch("DROP TRIGGER latency_source_oldest;")
        .unwrap();
    validate_schema(&f.store.connection).unwrap();
}

#[test]
fn ignored_history_insert_cannot_claim_receipt_or_half_commit_batch() {
    for table in ["latency_samples", "latency_admissions"] {
        let mut f = fixture();
        let first = f.sample.clone();
        let mut ignored = first.clone();
        ignored.measurement_id = local::credential().unwrap();
        f.store.connection.execute_batch(&format!("CREATE TRIGGER ignore_after_startup BEFORE INSERT ON {table} WHEN NEW.measurement_id='{}' BEGIN SELECT RAISE(IGNORE); END;", ignored.measurement_id)).unwrap();
        assert!(
            send(&mut f, vec![first.clone(), ignored.clone()]).is_err(),
            "ignored {table} insertion must fail the batch"
        );
        for history_table in ["latency_samples", "latency_admissions", "latency_coverage"] {
            assert_eq!(
                f.store
                    .connection
                    .query_row(&format!("SELECT COUNT(*) FROM {history_table}"), [], |r| {
                        r.get::<_, u32>(0)
                    })
                    .unwrap(),
                0
            );
        }
        f.store
            .connection
            .execute_batch("DROP TRIGGER ignore_after_startup;")
            .unwrap();
        assert_eq!(send(&mut f, vec![first, ignored]).unwrap().inserted, 2);
    }
}

#[test]
fn ignored_capacity_watermark_rolls_back_eviction_and_admission() {
    let mut f = fixture();
    let oldest = seed_full(&mut f);
    for target in [
        "BEFORE INSERT ON latency_coverage".to_owned(),
        format!("BEFORE INSERT ON latency_admissions WHEN NEW.measurement_id='{oldest}'"),
        format!("BEFORE DELETE ON latency_samples WHEN OLD.measurement_id='{oldest}'"),
    ] {
        f.store
            .connection
            .execute_batch(&format!(
                "CREATE TRIGGER ignore_after_startup {target} BEGIN SELECT RAISE(IGNORE); END;"
            ))
            .unwrap();
        let sample = f.sample.clone();
        assert!(send(&mut f, vec![sample]).is_err());
        assert_eq!(
            f.store
                .connection
                .query_row("SELECT COUNT(*) FROM latency_samples", [], |r| r
                    .get::<_, u32>(0))
                .unwrap(),
            NODE_ROW_BUDGET
        );
        assert!(
            f.store
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM latency_samples WHERE measurement_id=?1)",
                    [&oldest],
                    |r| r.get::<_, bool>(0)
                )
                .unwrap()
        );
        assert_eq!(
            f.store
                .connection
                .query_row("SELECT COUNT(*) FROM latency_admissions", [], |r| r
                    .get::<_, u32>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            f.store
                .connection
                .query_row("SELECT COUNT(*) FROM latency_coverage", [], |r| r
                    .get::<_, u32>(0))
                .unwrap(),
            0
        );
        f.store
            .connection
            .execute_batch("DROP TRIGGER ignore_after_startup;")
            .unwrap();
    }
}

#[test]
fn ignored_epoch_insert_cannot_half_commit_a_new_runtime_lease() {
    let mut f = fixture();
    f.store.connection.execute_batch("CREATE TRIGGER ignore_epoch_after_startup BEFORE INSERT ON latency_runtime_epochs BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let before = f
        .store
        .get_connection_runtime_epoch(&f.token)
        .unwrap()
        .epoch;
    assert!(
        f.store
            .begin_connection_runtime(
                &f.token,
                &BeginConnectionRuntimeRequest {
                    expected_epoch: before,
                    instance_id: local::credential().unwrap()
                }
            )
            .is_err()
    );
    assert_eq!(
        f.store
            .get_connection_runtime_epoch(&f.token)
            .unwrap()
            .epoch,
        before
    );
    let sample = f.sample.clone();
    assert_eq!(
        send(&mut f, vec![sample]).unwrap().inserted,
        1,
        "original applied epoch remains usable"
    );
}

#[test]
fn rolling_is_source_scoped_and_eviction_failure_rolls_back_admission_and_coverage() {
    let mut f = fixture();
    seed_full(&mut f);
    let other = f
        .store
        .create_network(&CreateNetworkRequest {
            name: "other-quota".into(),
        })
        .unwrap();
    let network = f.store.network_id(&other.network_id).unwrap();
    let (source, _) = enroll(&mut f.store, &network, 25);
    let (peer, _) = enroll(&mut f.store, &network, 26);
    f.store.connection.execute("INSERT INTO latency_samples(network_id,source_node_id,peer_node_id,measurement_id,runtime_epoch,measured_ms,received_ms,body) VALUES (?1,?2,?3,?4,1,?5,?5,?6)",params![network.as_str(),source,peer,local::credential().unwrap(),Utc::now().timestamp_millis(),canonical_json(&f.sample).unwrap()]).unwrap();
    f.store.connection.execute_batch("CREATE TRIGGER fail_eviction BEFORE DELETE ON latency_samples BEGIN SELECT RAISE(ABORT,'fixture eviction failure'); END;").unwrap();
    let sample = f.sample.clone();
    assert!(send(&mut f, vec![sample.clone()]).is_err());
    assert_eq!(
        f.store
            .connection
            .query_row("SELECT COUNT(*) FROM latency_admissions", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.store
            .connection
            .query_row("SELECT COUNT(*) FROM latency_coverage", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        0
    );
    f.store
        .connection
        .execute_batch("DROP TRIGGER fail_eviction")
        .unwrap();
    assert_eq!(send(&mut f, vec![sample]).unwrap().inserted, 1);
    assert_eq!(
        f.store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM latency_samples WHERE network_id=?1",
                [network.as_str()],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        f.store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM latency_samples WHERE source_node_id=?1",
                [&f.source],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        NODE_ROW_BUDGET
    );
}

#[test]
fn capacity_covers_all_peers_of_source_without_claiming_contiguous_pair_coverage() {
    let mut f = fixture();
    let (extra, _) = enroll(&mut f.store, &f.network, 27);
    let mut plan = f.store.connections(&f.network).unwrap().configuration;
    plan.candidates.push(ConnectionCandidate {
        id: "dial-extra".into(),
        dialer_node_id: extra.clone(),
        listener_id: "listen".into(),
        endpoint: ConnectionEndpoint {
            host: "127.0.0.1".into(),
            port: 19001,
        },
        priority: 0,
        credential_ref: None,
    });
    f.store
        .save_connections(
            &f.network,
            &SaveConnectionsRequest {
                expected_revision: f.store.revision(&f.network).unwrap(),
                configuration: plan,
            },
        )
        .unwrap();
    f.store
        .connection
        .execute(
            "DELETE FROM connection_runtime_reports WHERE node_id=?1",
            [&f.source],
        )
        .unwrap();
    applied(&mut f.store, &f.token, 1);
    let snapshot = f.store.agent_connections(&f.token).unwrap();
    f.sample.revision = snapshot.payload.revision;
    f.sample.digest = snapshot.digest;
    seed_full(&mut f);
    let mut second = f.sample.clone();
    second.peer_node_id = extra.clone();
    second.channel_associations[0].initiator_node_id = extra.clone();
    second.measurement_id = local::credential().unwrap();
    let mut first = f.sample.clone();
    first.measurement_id = local::credential().unwrap();
    let response = send(&mut f, vec![first, second]).unwrap();
    assert_eq!(response.inserted, 2);
    assert_eq!(
        f.store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM latency_samples WHERE source_node_id=?1",
                [&f.source],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        NODE_ROW_BUDGET
    );
    let mut q = query(&f);
    q.peer_node_id = extra;
    q.from = Utc::now() - chrono::Duration::hours(2);
    let response = f.store.latency_history(&f.network, &q).unwrap();
    assert_eq!(response.records.len(), 1);
    assert!(response.capacity_trimmed); // deliberately conservative across peers
}
