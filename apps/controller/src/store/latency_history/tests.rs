use super::*;
use crate::NetworkOptions;
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
    assert!(matches!(
        f.store
            .authorize_latency_history_read(&f.peer_token, f.network.as_str()),
        Err(Error::Unauthorized)
    ));
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
