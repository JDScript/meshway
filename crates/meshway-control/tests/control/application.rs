//! Authenticated Control/Server protocol fixtures. Supplied installation results
//! and probe samples exercise recording, not OS installation or packet I/O.
use super::{
    faults::{Fault, Proxy},
    support::{Owner, Server, request},
    system,
};
use chrono::{DateTime, Utc};
use meshway_configuration::{AccessRule, NetworkConfigurationInput};
use meshway_control::{Control, Error};
use meshway_control_client::{
    ApplyConfigurationPath, GetConfigurationProgressPath, GetConfigurationSourcePath,
    GetConfigurationSourceQuery, GetNodeStatePath, QueryObservationHistoryPath,
};
use meshway_model::{
    ConfigurationRef, EventId, NodeId, Sequence, Version,
    state::{
        Capability, ConfigurationState, NodeState, ProbeKind, ProbeObservation, ProbeOutcome,
        StreamKind, TransportRole,
    },
};
use meshway_protocol::{
    ConfigurationRequest, DemandRequest, InstanceRequest, WireSecret,
    execution::{ApplyFailure, ReceiptFreshness},
    live::{ApplicationPhase, ConfigurationNodeProgress},
    observations::{HistoricalEvent, HistoryItems, HistoryMode, HistoryQuery},
    state::{NodeStateView, VisibleReport},
};
use std::{collections::BTreeMap, time::Duration};

const RECEIPTS: &str = "/configuration/receipts";
const STATE: &str = "/agent-api/v1/state";

async fn start_control(server: &Server, owner: &Owner, origin: &str, name: &str) -> Control {
    let mut control = server
        .control(owner, origin, name, Default::default())
        .await;
    // Revision progress deliberately retains the revision's initial targets.
    // Publish after acquisition so this fixture's ticket is that exact target,
    // rather than an instance-acquisition refresh of the enrollment revision.
    configure(server, owner, |cfg| {
        cfg.dns.domain = format!("{name}.mesh.example");
    })
    .await;
    control.sync_configuration().await.unwrap();
    control.renew_instance().await.unwrap();
    control
}

async fn progress(
    server: &Server,
    owner: &Owner,
    node: NodeId,
    target: &ConfigurationRef,
) -> ConfigurationNodeProgress {
    let mut progress = server
        .client
        .get_configuration_progress(
            &GetConfigurationProgressPath {
                network_id: owner.network,
                revision: target.revision,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    let progress = progress.nodes.remove(&node).unwrap();
    assert_eq!(
        &progress.target, target,
        "read the ticket's exact original revision target"
    );
    progress
}

async fn view(server: &Server, owner: &Owner, node: NodeId) -> NodeStateView {
    server
        .client
        .get_node_state(
            &GetNodeStatePath {
                network_id: owner.network,
                node_id: node,
            },
            owner.token.expose(),
        )
        .await
        .unwrap()
        .nodes
        .remove(&node)
        .unwrap()
}

async fn configure(
    server: &Server,
    owner: &Owner,
    edit: impl FnOnce(&mut NetworkConfigurationInput),
) {
    let source = server
        .client
        .get_configuration_source(
            &GetConfigurationSourcePath {
                network_id: owner.network,
            },
            &GetConfigurationSourceQuery::default(),
            owner.token.expose(),
        )
        .await
        .unwrap();
    let mut input = NetworkConfigurationInput::from(&source.configuration);
    edit(&mut input);
    server
        .client
        .apply_configuration(
            &ApplyConfigurationPath {
                network_id: owner.network,
            },
            &ConfigurationRequest::Document {
                expected_revision: source.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
}

fn facts(control: &Control, applied: bool) -> NodeState {
    NodeState {
        system: system(),
        capabilities: control
            .cached_target()
            .unwrap()
            .configuration
            .self_keys
            .capabilities
            .clone(),
        configuration: ConfigurationState {
            saved: control.status().cached,
            applied: applied.then(|| control.status().cached.unwrap()),
            progress: None,
        },
        listeners: BTreeMap::new(),
        sockets: BTreeMap::new(),
        links: BTreeMap::new(),
        paths: BTreeMap::new(),
        selection: Default::default(),
        probes: Vec::new(),
        problems: Vec::new(),
        exit_capability: None,
    }
}

fn assert_send<T: Send>(_: &T) {}

#[tokio::test]
async fn lost_saved_and_cancelled_completion_keep_exact_identity_and_order_new_attempts() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = start_control(&server, &owner, &proxy.origin, "application-retry").await;
    let node = control.status().binding.unwrap().node_id;
    let target = control.status().cached.unwrap();
    proxy.fault(RECEIPTS, Fault::Lost);
    assert!(control.begin_application().await.is_err());
    let saved = progress(&server, &owner, node, &target).await;
    assert_eq!(saved.phase, ApplicationPhase::Saved);
    assert_eq!(saved.attempt, Some(Version::MIN));
    assert_eq!(saved.sequence, Some(Sequence::MIN));
    tokio::time::sleep(Duration::from_millis(5)).await;
    let first = control.begin_application().await.unwrap();
    let duplicate = progress(&server, &owner, node, &target).await;
    assert_eq!(duplicate.received_at, saved.received_at);
    assert_eq!(first.attempt(), Version::MIN);
    assert_eq!(
        control.begin_application().await.unwrap().attempt(),
        first.attempt()
    );
    assert_eq!(proxy.count(RECEIPTS), 2);

    let observed = Utc::now();
    proxy.fault(RECEIPTS, Fault::Hold);
    {
        let completion = control.finish_application(&first, Err(ApplyFailure::Tun), observed);
        assert_send(&completion);
        tokio::pin!(completion);
        tokio::select! {
            result = &mut completion => panic!("completion delivered before release: {result:?}"),
            result = tokio::time::timeout(Duration::from_secs(2), proxy.committed()) => result.unwrap(),
        }
        // Drop this waiter after the real database owner has committed.
    }
    proxy.release();
    let failed = progress(&server, &owner, node, &target).await;
    assert_eq!(failed.phase, ApplicationPhase::Failed);
    assert_eq!(failed.attempt, Some(first.attempt()));
    assert_eq!(failed.sequence, Some(Sequence::MIN.checked_next().unwrap()));
    for result in [Ok(()), Err(ApplyFailure::Dns)] {
        assert!(matches!(
            control.finish_application(&first, result, Utc::now()).await,
            Err(Error::Pending)
        ));
    }
    assert_eq!(proxy.count(RECEIPTS), 3);
    control
        .finish_application(
            &first.clone(),
            Err(ApplyFailure::Tun),
            observed + chrono::Duration::seconds(1),
        )
        .await
        .unwrap();
    // Server rejects a same attempt/sequence with a changed reportedAt. Success
    // plus the unchanged receivedAt proves this really was the original retry.
    assert_eq!(
        progress(&server, &owner, node, &target).await.received_at,
        failed.received_at
    );

    let second = control.begin_application().await.unwrap();
    assert_eq!(second.attempt(), first.attempt().checked_next().unwrap());
    let saved = progress(&server, &owner, node, &target).await;
    assert_eq!(saved.phase, ApplicationPhase::Saved);
    assert_eq!(saved.attempt, Some(second.attempt()));
    let completed = Utc::now();
    proxy.fault(RECEIPTS, Fault::Lost);
    assert!(
        control
            .finish_application(&second, Ok(()), completed)
            .await
            .is_err()
    );
    let applied = progress(&server, &owner, node, &target).await;
    assert_eq!(applied.phase, ApplicationPhase::Applied);
    assert_eq!(applied.attempt, Some(second.attempt()));
    control
        .finish_application(&second, Ok(()), completed + chrono::Duration::seconds(1))
        .await
        .unwrap();
    assert_eq!(
        progress(&server, &owner, node, &target).await.received_at,
        applied.received_at
    );
    control
        .finish_application(&first, Err(ApplyFailure::Tun), Utc::now())
        .await
        .unwrap();
    let late = progress(&server, &owner, node, &target).await;
    assert_eq!(late.phase, ApplicationPhase::Applied);
    assert_eq!(late.attempt, Some(second.attempt()));
    assert_eq!(late.received_at, applied.received_at);
    assert!(view(&server, &owner, node).await.report.is_none());
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn historical_completion_cannot_promote_disabled_current_target_and_state_is_separate() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let mut control = start_control(&server, &owner, &server.origin, "application-disabled").await;
    let node = control.status().binding.unwrap().node_id;
    let old = control.begin_application().await.unwrap();
    configure(&server, &owner, |cfg| {
        cfg.nodes.get_mut(&node).unwrap().enabled = false
    })
    .await;
    let target = control.sync_configuration().await.unwrap().unwrap();
    assert_ne!(&target, old.target());
    assert!(!control.cached_target().unwrap().configuration.enabled);
    assert!(
        control
            .status()
            .instance
            .unwrap()
            .execution_expires_at_ms
            .is_none()
    );
    assert!(control.execution_snapshot().is_err());
    let current = control.begin_application().await.unwrap();
    control
        .finish_application(&old, Ok(()), Utc::now())
        .await
        .unwrap();
    let historical = progress(&server, &owner, node, old.target()).await;
    assert_eq!(historical.phase, ApplicationPhase::Applied);
    assert_eq!(historical.freshness, ReceiptFreshness::Superseded);
    assert_eq!(
        progress(&server, &owner, node, &target).await.phase,
        ApplicationPhase::Saved
    );
    assert!(view(&server, &owner, node).await.report.is_none());

    // Caller-supplied completion of the disabled installation requires no
    // execution authority. This verifies its protocol record only.
    control
        .finish_application(&current, Ok(()), Utc::now())
        .await
        .unwrap();
    assert_eq!(
        progress(&server, &owner, node, &target).await.phase,
        ApplicationPhase::Applied
    );
    let runtime = facts(&control, true);
    let mut wrong_target = runtime.clone();
    wrong_target.configuration.applied = Some(old.target().clone());
    assert!(matches!(
        control.report_runtime_state(wrong_target, Utc::now()).await,
        Err(Error::Contract)
    ));
    let mut wrong_caps = runtime.clone();
    wrong_caps.capabilities.insert(
        "feature:unacquired".into(),
        Capability::Feature { version: 1 },
    );
    assert!(matches!(
        control.report_runtime_state(wrong_caps, Utc::now()).await,
        Err(Error::Contract)
    ));
    assert!(view(&server, &owner, node).await.report.is_none());
    let observed = Utc::now();
    let accepted = control
        .report_runtime_state(runtime.clone(), observed)
        .await
        .unwrap();
    assert_eq!(accepted.sequence.get(), 1);
    let state = view(&server, &owner, node).await;
    assert_eq!(state.observed_at, Some(observed));
    let Some(VisibleReport::Full { report }) = state.report else {
        panic!("full source report")
    };
    assert_eq!(*report, runtime);
    assert!(control.execution_snapshot().is_err());
    assert!(
        control
            .status()
            .instance
            .unwrap()
            .execution_expires_at_ms
            .is_none()
    );
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn tickets_are_bound_to_the_control_owner_and_instance_generation() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut first = start_control(&server, &owner, &proxy.origin, "application-owner").await;
    let ticket = first.begin_application().await.unwrap();
    let mut other = start_control(&server, &owner, &proxy.origin, "application-foreign").await;
    let count = proxy.count(RECEIPTS);
    assert!(matches!(
        other
            .finish_application(&ticket, Err(ApplyFailure::Tun), Utc::now())
            .await,
        Err(Error::Lifecycle)
    ));
    first.stop().await.unwrap();
    first.authenticate().await.unwrap();
    let instance = first.acquire_instance(Default::default()).await.unwrap();
    first.sync_configuration().await.unwrap();
    assert_ne!(instance.instance_id, ticket.instance().instance_id);
    assert!(instance.generation > ticket.instance().generation);
    assert!(matches!(
        first.finish_application(&ticket, Ok(()), Utc::now()).await,
        Err(Error::Lifecycle)
    ));
    assert_eq!(proxy.count(RECEIPTS), count);
    let recent = first.begin_application().await.unwrap();
    first.shutdown().await.unwrap();
    let mut reopened = Control::open(
        &server.path("application-owner"),
        &proxy.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    reopened.authenticate().await.unwrap();
    reopened.acquire_instance(Default::default()).await.unwrap();
    reopened.sync_configuration().await.unwrap();
    let count = proxy.count(RECEIPTS);
    assert!(matches!(
        reopened
            .finish_application(&recent, Err(ApplyFailure::Storage), Utc::now())
            .await,
        Err(Error::Lifecycle)
    ));
    assert_eq!(proxy.count(RECEIPTS), count);
    let fresh = reopened.begin_application().await.unwrap();
    assert_eq!(fresh.attempt(), Version::MIN);
    reopened
        .finish_application(&fresh, Ok(()), Utc::now())
        .await
        .unwrap();
    other.shutdown().await.unwrap();
    reopened.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

async fn history(
    server: &Server,
    owner: &Owner,
    node: NodeId,
    from: DateTime<Utc>,
) -> Vec<HistoricalEvent> {
    let page = server
        .client
        .query_observation_history(
            &QueryObservationHistoryPath {
                network_id: owner.network,
            },
            &HistoryQuery {
                from,
                to: Utc::now() + chrono::Duration::seconds(1),
                source_node_id: Some(node),
                peer_node_id: None,
                mode: HistoryMode::Raw,
                bucket_seconds: None,
                cursor: None,
                limit: 20,
                kinds: None,
                link_id: None,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    assert!(page.next_cursor.is_none());
    let HistoryItems::Raw { items } = page.result else {
        panic!("raw source history")
    };
    items
}

#[tokio::test]
async fn runtime_and_control_share_sequence_and_pending_retry_never_duplicates_probe_history() {
    use meshway_configuration::execution::{BusinessProtocol, BusinessTarget, DemandDecision};
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let caps = BTreeMap::from([(
        "transport:native_udp".into(),
        Capability::Transport {
            roles: vec![TransportRole::Dial, TransportRole::Listen],
            streams: vec![StreamKind::Udp],
        },
    )]);
    let mut source = Control::open(
        &server.path("runtime-source"),
        &proxy.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    let a = server
        .enroll(&owner, &mut source, "runtime-source")
        .await
        .node_id;
    let mut peer = Control::open(
        &server.path("runtime-peer"),
        &server.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    let b = server
        .enroll(&owner, &mut peer, "runtime-peer")
        .await
        .node_id;
    for control in [&mut source, &mut peer] {
        control.authenticate().await.unwrap();
        control.acquire_instance(caps.clone()).await.unwrap();
    }
    configure(&server, &owner, |cfg| {
        for node in cfg.nodes.values_mut() {
            node.listeners =
                serde_json::from_str(r#"{"udp":{"type":"native_udp","listenPort":23456}}"#)
                    .unwrap();
        }
        cfg.access = vec![AccessRule {
            remark: String::new(),
            src: vec![format!("node:{a}")],
            dst: vec![format!("node:{b}")],
            ip: vec!["udp:42424".into()],
            bidirectional: false,
        }];
    })
    .await;
    let lease = source.status().instance.unwrap();
    let demand = source
        .create_demand(&DemandRequest {
            instance: InstanceRequest {
                instance_id: lease.instance_id,
                generation: lease.generation,
            },
            request_id: request(),
            target: BusinessTarget::Mesh { node: b },
            protocol: BusinessProtocol::Udp,
            port: Some(42424.try_into().unwrap()),
            profile: "mesh".into(),
            duration_seconds: 120,
        })
        .await
        .unwrap();
    assert!(matches!(demand.decision, DemandDecision::Permitted { .. }));
    for control in [&mut source, &mut peer] {
        control.sync_configuration().await.unwrap();
        control.renew_instance().await.unwrap();
    }
    let ticket = source.begin_application().await.unwrap();
    source
        .finish_application(&ticket, Ok(()), Utc::now())
        .await
        .unwrap();
    let first = source.report_control_state(system()).await.unwrap();
    assert_eq!(first.sequence.get(), 1);
    let from = Utc::now() - chrono::Duration::minutes(1);
    assert!(history(&server, &owner, a, from).await.is_empty());
    let observed = Utc::now();
    let mut runtime = facts(&source, true);
    runtime.probes.push(ProbeObservation {
        channels: None,
        measurement_id: EventId::new(*request().as_uuid()).unwrap(),
        kind: ProbeKind::Path,
        link_id: None,
        transport: None,
        path: vec![a, b],
        publication_id: ticket.target().execution_publication,
        started_at: observed - chrono::Duration::milliseconds(1),
        completed_at: Some(observed),
        outcome: ProbeOutcome::Success,
        duration_us: Some(Sequence::new(1000).unwrap()),
    });
    proxy.fault(STATE, Fault::Lost);
    assert!(
        source
            .report_runtime_state(runtime.clone(), observed)
            .await
            .is_err()
    );
    let committed = view(&server, &owner, a).await;
    assert_eq!(committed.report_sequence.unwrap().get(), 2);
    assert_eq!(committed.observed_at, Some(observed));
    let Some(VisibleReport::Full { report }) = &committed.report else {
        panic!("full source report")
    };
    assert_eq!(report.probes, runtime.probes);
    // State is a latest-facts API, not a history ingestion API. Only explicit
    // history/task-result producers append observation history on Server.
    assert!(history(&server, &owner, a, from).await.is_empty());
    let count = proxy.count(STATE);
    assert!(matches!(
        source.report_control_state(system()).await,
        Err(Error::Pending)
    ));
    let mut different = runtime.clone();
    different.probes[0].measurement_id = EventId::new(*request().as_uuid()).unwrap();
    assert!(matches!(
        source.report_runtime_state(different, Utc::now()).await,
        Err(Error::Pending)
    ));
    assert_eq!(proxy.count(STATE), count);

    let accepted = source
        .report_runtime_state(runtime, observed + chrono::Duration::seconds(1))
        .await
        .unwrap();
    assert!(accepted.duplicate);
    assert_eq!(accepted.sequence.get(), 2);
    assert_eq!(accepted.received_at, committed.received_at.unwrap());
    let after = view(&server, &owner, a).await;
    assert_eq!(after.report, committed.report);
    assert_eq!(after.observed_at, committed.observed_at);
    assert_eq!(after.report_sequence, committed.report_sequence);
    assert_eq!(after.received_at, committed.received_at);
    assert!(history(&server, &owner, a, from).await.is_empty());
    let third = source.report_control_state(system()).await.unwrap();
    assert_eq!(third.sequence.get(), 3);
    assert!(!third.duplicate);

    // A plain installed-target fact remains valid historical evidence after
    // replacement. Unlike the probe above, it needs no current path authority.
    let historical = facts(&source, true);
    let observed = Utc::now();
    proxy.fault(STATE, Fault::Lost);
    assert!(
        source
            .report_runtime_state(historical.clone(), observed)
            .await
            .is_err()
    );
    let committed = view(&server, &owner, a).await;
    configure(&server, &owner, |cfg| {
        cfg.nodes.get_mut(&a).unwrap().enabled = false
    })
    .await;
    source.renew_instance().await.unwrap();
    assert_ne!(
        source.status().instance.unwrap().target.as_ref(),
        Some(ticket.target())
    );
    let accepted = source
        .report_runtime_state(historical, observed + chrono::Duration::seconds(1))
        .await
        .unwrap();
    assert!(accepted.duplicate);
    assert_eq!(accepted.sequence.get(), 4);
    assert_eq!(accepted.received_at, committed.received_at.unwrap());
    assert_eq!(view(&server, &owner, a).await.observed_at, Some(observed));
    source.sync_configuration().await.unwrap();
    assert_eq!(
        source
            .report_control_state(system())
            .await
            .unwrap()
            .sequence
            .get(),
        5
    );
    assert!(history(&server, &owner, a, from).await.is_empty());
    source.shutdown().await.unwrap();
    peer.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn invalid_local_time_does_not_freeze_completion_and_server_bounds_late_failures() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let mut control = start_control(&server, &owner, &server.origin, "application-time").await;
    let node = control.status().binding.unwrap().node_id;
    let ticket = control.begin_application().await.unwrap();
    assert!(matches!(
        control
            .finish_application(
                &ticket,
                Err(ApplyFailure::Tun),
                DateTime::from_timestamp(-1, 0).unwrap()
            )
            .await,
        Err(Error::Contract)
    ));
    control
        .finish_application(&ticket, Ok(()), Utc::now())
        .await
        .unwrap();
    let applied = progress(&server, &owner, node, ticket.target()).await;
    assert_eq!(applied.phase, ApplicationPhase::Applied);
    let late = control.begin_application().await.unwrap();
    let saved = progress(&server, &owner, node, late.target()).await;
    assert_eq!(saved.phase, ApplicationPhase::Saved);
    assert!(
        control
            .finish_application(
                &late,
                Err(ApplyFailure::Tun),
                Utc::now() - chrono::Duration::minutes(6)
            )
            .await
            .is_err()
    );
    assert!(control.status().authenticated);
    assert_eq!(
        control.status().instance.unwrap().instance_id,
        late.instance().instance_id
    );
    let after = progress(&server, &owner, node, late.target()).await;
    assert_eq!(after.phase, ApplicationPhase::Saved);
    assert_eq!(after.attempt, saved.attempt);
    assert_eq!(after.received_at, saved.received_at);
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn receipt_rejections_retire_exact_ticket_without_fencing_and_reapply_new_attempt() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = start_control(&server, &owner, &proxy.origin, "receipt-conflicts").await;
    for (code, body_status, refresh) in [
        ("receipt_publication_superseded", 409, true),
        ("receipt_lease_expired", 409, true),
        ("receipt_precondition", 409, true),
        ("receipt_content_conflict", 409, true),
        ("receipt_publication_superseded", 400, false),
        ("receipt_instance_mismatch", 409, false),
        ("receipt_target_mismatch", 409, false),
        ("request_id_conflict", 409, false),
    ] {
        control.authenticate().await.unwrap();
        control.acquire_instance(Default::default()).await.unwrap();
        control.refresh_configuration().await.unwrap();
        let ticket = control.begin_application().await.unwrap();
        let snapshot = control.execution_snapshot().unwrap();
        let status = control.snapshot();
        proxy.fault(RECEIPTS, Fault::ReceiptConflict(code, body_status));
        let error = control
            .finish_application(&ticket, Ok(()), Utc::now())
            .await
            .unwrap_err();
        assert!(!format!("{error:?}").contains("sensitive receipt body"));
        assert!(!error.to_string().contains("sensitive receipt body"));
        assert_eq!(
            matches!(error, Error::ReceiptRejected(_)),
            refresh,
            "{code}/{body_status}"
        );
        assert_eq!(control.status().authenticated, refresh);
        assert_eq!(status.status().instance.is_some(), refresh);
        assert_eq!(snapshot.ensure_current().is_ok(), refresh);
        if refresh {
            assert_eq!(
                control.status().instance.unwrap().instance_id,
                ticket.instance().instance_id
            );
            let count = proxy.count(RECEIPTS);
            assert!(matches!(
                control
                    .finish_application(&ticket, Ok(()), Utc::now())
                    .await,
                Err(Error::Pending)
            ));
            assert_eq!(
                proxy.count(RECEIPTS),
                count,
                "rejected ticket cannot be resubmitted"
            );
            control.refresh_configuration().await.unwrap();
            let next = control.begin_application().await.unwrap();
            assert_eq!(next.attempt(), ticket.attempt().checked_next().unwrap());
            control
                .finish_application(&next, Ok(()), Utc::now())
                .await
                .unwrap();
        } else {
            assert!(control.status().instance.is_none());
        }
    }
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}
