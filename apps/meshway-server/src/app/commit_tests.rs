//! Commit-to-journal checks use the ordinary App and real admission inventory.
use super::*;
use crate::{
    accounts::{Secret, random_uuid},
    state::stream::CommittedChange,
};
use meshway_model::*;
use meshway_protocol::{
    WireSecret, configuration::ConfigurationRequest, events::ConfigurationNotice,
};
use std::{collections::BTreeMap, future::Future, task::Poll};

fn request() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}
fn sequence(app: &App) -> u64 {
    app.state_runtime.lock().unwrap().hub.events.sequence()
}
fn changes(app: &App, after: u64, network: NetworkId) -> Vec<CommittedChange> {
    app.state_runtime
        .lock()
        .unwrap()
        .hub
        .events
        .after(after, network)
        .unwrap()
        .into_iter()
        .map(|entry| entry.change)
        .collect()
}
fn notices(app: &App, after: u64, network: NetworkId) -> Vec<ConfigurationNotice> {
    changes(app, after, network)
        .into_iter()
        .filter_map(|change| match change {
            CommittedChange::Configuration { notice } => Some(notice),
            _ => None,
        })
        .collect()
}
async fn targets(
    app: &App,
    network: NetworkId,
    revision: Revision,
) -> BTreeMap<NodeId, ConfigurationRef> {
    app.executor.handle().execute(move |db| {
        let value: String = db.query_row("SELECT execution_targets FROM configuration_versions WHERE network_id=?1 AND revision=?2", rusqlite::params![network.as_uuid().as_bytes().as_slice(), revision.get() as i64], |row| row.get(0)).unwrap();
        serde_json::from_str(&value).unwrap()
    }).await.unwrap()
}

#[tokio::test]
async fn account_revocation_cannot_be_blocked_by_the_replay_window_size() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let owner = app
        .accounts()
        .register(
            "many-networks@example.test".into(),
            Secret::new("valid many networks password".into()),
            request(),
        )
        .await
        .unwrap();
    let count = crate::state::stream::MAX_REPLAY_EVENTS + 1;
    let mut last = None;
    for index in 0..count {
        last = Some(
            app.networks()
                .create(owner.token.expose(), format!("revoke-{index}"), request())
                .await
                .unwrap()
                .id,
        );
    }
    let before = sequence(&app);
    app.accounts()
        .revoke_sessions(
            owner.token.expose(),
            crate::accounts::RevokeSessions::All,
            request(),
        )
        .await
        .unwrap();
    assert!(matches!(
        app.accounts().authenticate(owner.token.expose()).await,
        Err(crate::error::Error::InvalidSession)
    ));
    {
        let runtime = app.state_runtime.lock().unwrap();
        let events = &runtime.hub.events;
        assert_eq!(events.sequence(), before + count as u64);
        assert!(
            events.after(before, last.unwrap()).is_err(),
            "old cursors reset when the bounded replay window is exceeded"
        );
    }
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn accepted_configuration_waiter_cancellation_noop_and_rollback_preserve_journal() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, _, _, _, _) = super::node_tests::fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            request(),
        )
        .await
        .unwrap();
    let other = app
        .networks()
        .create(owner.token.expose(), "other".into(), request())
        .await
        .unwrap();
    assert!(
        targets(&app, other.id, other.current_revision)
            .await
            .is_empty()
    );
    let service = app.configurations();
    let current = service
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let before = sequence(&app);
    let edit_id = request();
    let edit = || ConfigurationRequest::Document {
        expected_revision: current.revision,
        request_id: edit_id,
        source_jsonc: WireSecret::new(format!(
            "// private source marker\n{}",
            current.document.source_jsonc.expose()
        )),
    };
    app.executor.handle().execute(|db| db.execute_batch("CREATE TRIGGER fail_notice BEFORE INSERT ON configuration_commits BEGIN SELECT RAISE(ABORT,'failure after participant'); END;")).await.unwrap().unwrap();
    assert!(
        service
            .apply(owner.token.expose(), network, edit(), request())
            .await
            .is_err()
    );
    assert_eq!(sequence(&app), before);
    assert_eq!(
        service
            .read(owner.token.expose(), network, None)
            .await
            .unwrap()
            .revision,
        current.revision
    );
    app.executor
        .handle()
        .execute(|db| db.execute_batch("DROP TRIGGER fail_notice"))
        .await
        .unwrap()
        .unwrap();

    let (entered, entered_rx) = tokio::sync::oneshot::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let handle = app.executor.handle();
    let blocker = tokio::spawn(async move {
        handle
            .execute(move |_| {
                entered.send(()).unwrap();
                release_rx.recv().unwrap();
            })
            .await
            .unwrap()
    });
    entered_rx.await.unwrap();
    // This is the exact future awaited by the HTTP apply handler. Polling once
    // accepts its synchronous SQLite job, then cancelling only drops the waiter.
    let mut waiter = Box::pin(service.apply(owner.token.expose(), network, edit(), request()));
    std::future::poll_fn(|cx| {
        assert!(waiter.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(sequence(&app), before);
    drop(waiter);
    release.send(()).unwrap();
    blocker.await.unwrap();
    let committed = service
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let notice = notices(&app, before, network);
    assert_eq!(notice.len(), 1);
    let notice = &notice[0];
    assert_eq!(notice.previous_revision, current.revision);
    assert_eq!(notice.revision, committed.revision);
    assert_eq!(notice.request_id, Some(edit_id));
    assert!(
        matches!(notice.actor, meshway_protocol::events::EventActor::User { user_id } if user_id == owner.session.user.id)
    );
    assert!(notice.source_changed && notice.changes_complete);
    assert!(notice.changed_pointers.is_empty());
    assert!(
        !serde_json::to_string(notice)
            .unwrap()
            .contains("private source marker")
    );
    assert!(changes(&app, before, other.id).is_empty());
    let fixed = targets(&app, network, committed.revision).await;
    assert_eq!(
        fixed.keys().copied().collect::<Vec<_>>(),
        [a, b]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    );
    assert!(
        fixed
            .values()
            .all(|reference| reference.revision == committed.revision)
    );
    let after = sequence(&app);
    service
        .apply(owner.token.expose(), network, edit(), request())
        .await
        .unwrap();
    service
        .apply(
            owner.token.expose(),
            network,
            ConfigurationRequest::Document {
                expected_revision: committed.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(committed.document.source_jsonc.expose().to_owned()),
            },
            request(),
        )
        .await
        .unwrap();
    assert_eq!(sequence(&app), after);
    app.executor.handle().execute(move |db| {
        assert!(db.execute("UPDATE configuration_versions SET execution_targets='{}' WHERE network_id=?1 AND revision=?2", rusqlite::params![network.as_uuid().as_bytes().as_slice(), committed.revision.get() as i64]).is_err());
    }).await.unwrap();
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn email_sync_commits_all_network_notices_or_none_and_keeps_fixed_targets() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let owner = app
        .accounts()
        .register(
            "before@example.test".into(),
            Secret::new("valid password".into()),
            request(),
        )
        .await
        .unwrap();
    let mut networks = Vec::new();
    for index in 0..2 {
        let network = app
            .networks()
            .create(owner.token.expose(), format!("network {index}"), request())
            .await
            .unwrap();
        let signer = ed25519_dalek::SigningKey::from_bytes(&[index + 51; 32]);
        let node = super::node_tests::enroll_user_node(
            &app,
            owner.token.expose(),
            network.id,
            &signer,
            [index + 61; 32],
        )
        .await;
        networks.push((network.id, node.node_id));
    }
    let before = sequence(&app);
    app.executor.handle().execute(|db| db.execute_batch("CREATE TRIGGER fail_email BEFORE INSERT ON activity_events WHEN NEW.action='account.email' BEGIN SELECT RAISE(ABORT,'outer rollback'); END;")).await.unwrap().unwrap();
    assert!(
        app.accounts()
            .change_email(
                owner.token.expose(),
                Secret::new("valid password".into()),
                "after@example.test".into(),
                request(),
                app.configurations()
            )
            .await
            .is_err()
    );
    assert_eq!(sequence(&app), before);
    assert_eq!(
        app.accounts()
            .authenticate(owner.token.expose())
            .await
            .unwrap()
            .user
            .email,
        "before@example.test"
    );
    app.executor
        .handle()
        .execute(|db| db.execute_batch("DROP TRIGGER fail_email"))
        .await
        .unwrap()
        .unwrap();
    let change_id = request();
    app.accounts()
        .change_email(
            owner.token.expose(),
            Secret::new("valid password".into()),
            "after@example.test".into(),
            change_id,
            app.configurations(),
        )
        .await
        .unwrap();
    for (network, node) in networks {
        let notice = notices(&app, before, network);
        assert_eq!(notice.len(), 1);
        assert!(matches!(
            notice[0].cause,
            meshway_protocol::history::ConfigurationChangeCause::EmailSync
        ));
        assert_eq!(notice[0].request_id, Some(change_id));
        assert!(
            notice[0]
                .changed_pointers
                .iter()
                .any(|pointer| pointer.starts_with(&format!("/nodes/{node}")))
        );
        assert_eq!(
            targets(&app, network, notice[0].revision)
                .await
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![node]
        );
        let snapshot = app
            .configurations()
            .read(owner.token.expose(), network, None)
            .await
            .unwrap();
        assert!(
            matches!(&snapshot.document.configuration.nodes[&node].identity, meshway_configuration::NodeIdentity::User { user } if user == "after@example.test")
        );
    }
    let before = sequence(&app);
    app.accounts()
        .change_email(
            owner.token.expose(),
            Secret::new("valid password".into()),
            "after@example.test".into(),
            request(),
            app.configurations(),
        )
        .await
        .unwrap();
    assert_eq!(sequence(&app), before);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn apply_targets_do_not_follow_instance_refresh_demands_or_retired_nodes() {
    use meshway_protocol::execution::*;
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, asigner, bsigner) = super::node_tests::fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            request(),
        )
        .await
        .unwrap();
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let fixed = targets(&app, network, current.revision).await;
    let execution = app.execution();
    let mut admitted = Vec::new();
    for (node, keys, signer) in [(a, ak, asigner), (b, bk, bsigner)] {
        let session = super::node_tests::session(&execution, network, node, &keys, &signer).await;
        let acquire = AcquireInstanceRequest {
            instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
            capabilities: super::node_tests::caps(),
        };
        let before = sequence(&app);
        let lease = execution
            .acquire(session.token.expose(), acquire.clone())
            .await
            .unwrap();
        assert_eq!(
            changes(&app, before, network)
                .iter()
                .filter(|c| matches!(c, CommittedChange::Progress { .. }))
                .count(),
            1
        );
        let before = sequence(&app);
        assert_eq!(
            execution
                .acquire(session.token.expose(), acquire)
                .await
                .unwrap(),
            lease
        );
        assert_eq!(sequence(&app), before);
        admitted.push((session, lease));
    }
    let (session, lease) = &admitted[0];
    let instance = InstanceRequest {
        instance_id: lease.instance_id,
        generation: lease.generation,
    };
    let demand = DemandRequest {
        request_id: request(),
        instance: instance.clone(),
        target: meshway_configuration::execution::BusinessTarget::Mesh { node: b },
        protocol: meshway_configuration::execution::BusinessProtocol::Tcp,
        port: Some(80.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds: 60,
    };
    execution
        .demand(session.token.expose(), demand.clone())
        .await
        .unwrap();
    let before = sequence(&app);
    execution
        .demand(session.token.expose(), demand)
        .await
        .unwrap();
    assert_eq!(sequence(&app), before);
    assert_eq!(targets(&app, network, current.revision).await, fixed);
    let applied = app
        .configurations()
        .apply(
            owner.token.expose(),
            network,
            ConfigurationRequest::Operations {
                expected_revision: current.revision,
                request_id: request(),
                ops: vec![meshway_configuration::ConfigurationOperation::DeleteNode { node_id: b }],
            },
            request(),
        )
        .await
        .unwrap();
    assert!(
        !targets(&app, network, applied.revision)
            .await
            .contains_key(&b)
    );
    assert_eq!(targets(&app, network, current.revision).await, fixed);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn account_revocations_remove_real_observation_subscriptions_at_commit() {
    use meshway_configuration::execution::{BusinessProtocol, BusinessTarget};
    use meshway_protocol::{execution::*, observations::*};
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, asigner, bsigner) = super::node_tests::fixture().await;
    let execution = app.execution();
    let at = super::node_tests::session(&execution, network, a, &ak, &asigner).await;
    let bt = super::node_tests::session(&execution, network, b, &bk, &bsigner).await;
    let mut ai = None;
    for (index, session) in [&at, &bt].into_iter().enumerate() {
        let lease = execution
            .acquire(
                session.token.expose(),
                AcquireInstanceRequest {
                    instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                    capabilities: super::node_tests::caps(),
                },
            )
            .await
            .unwrap();
        if index == 0 {
            ai = Some(InstanceRequest {
                instance_id: lease.instance_id,
                generation: lease.generation,
            });
        }
    }
    execution
        .demand(
            at.token.expose(),
            DemandRequest {
                instance: ai.clone().unwrap(),
                request_id: request(),
                target: BusinessTarget::Mesh { node: b },
                protocol: BusinessProtocol::Tcp,
                port: Some(80.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 300,
            },
        )
        .await
        .unwrap();
    let observations = app.observation_service(execution.clone());
    let mut password = "a sufficiently long password";
    for operation in 0..4 {
        let session = app
            .accounts()
            .login(
                "owner@example.com".into(),
                Secret::new(password.into()),
                request(),
            )
            .await
            .unwrap();
        let identity = crate::state::service::ReadIdentity::UserSession {
            digest: crate::accounts::token_digest(session.token.expose()).unwrap(),
            network,
        };
        observations
            .create_observation(
                identity,
                CreateObservation {
                    service: meshway_configuration::execution::ObservationServiceScope::Tcp {
                        port: meshway_transport_spec::Port::new(443).unwrap(),
                    },
                    subscription_id: ObservationSubscriptionId::new(random_uuid().unwrap())
                        .unwrap(),
                    source_node_id: a,
                    target_node_id: b,
                    selection: ObservationPath::Auto,
                    metrics: vec![meshway_model::observations::MetricKind::Rtt],
                    interval_ms: 1000,
                },
            )
            .await
            .unwrap();
        let provider = execution.clone();
        execution
            .renew(at.token.expose(), ai.clone().unwrap())
            .await
            .unwrap();
        execution
            .configuration(
                at.token.expose(),
                NodeConfigurationRequest {
                    instance: ai.clone().unwrap(),
                    base: None,
                },
            )
            .await
            .unwrap();
        let node_token = at.token.expose().to_owned();
        let admission = app
            .executor
            .handle()
            .execute(move |db| {
                let tx = db.transaction().unwrap();
                provider
                    .report_admission_in_transaction(&tx, &node_token, chrono::Utc::now())
                    .unwrap()
            })
            .await
            .unwrap();
        assert_eq!(
            app.state_runtime
                .lock()
                .unwrap()
                .observations
                .node_tasks(&admission, chrono::Utc::now())
                .len(),
            1
        );
        let before = sequence(&app);
        match operation {
            0 => app
                .accounts()
                .logout(session.token.expose(), request())
                .await
                .unwrap(),
            1 => app
                .accounts()
                .revoke_sessions(
                    session.token.expose(),
                    crate::accounts::RevokeSessions::One(session.session.session_id),
                    request(),
                )
                .await
                .unwrap(),
            2 => {
                app.accounts()
                    .change_password(
                        session.token.expose(),
                        Secret::new(password.into()),
                        Secret::new("replacement password".into()),
                        request(),
                    )
                    .await
                    .unwrap();
                password = "replacement password";
            }
            _ => {
                let reset = app
                    .accounts()
                    .issue_password_reset("owner@example.com".into(), 600, request())
                    .await
                    .unwrap();
                app.accounts()
                    .redeem_password_reset(
                        reset.token.expose(),
                        Secret::new("reset replacement password".into()),
                        request(),
                    )
                    .await
                    .unwrap();
            }
        }
        // Read the runtime owner directly: a subsequent provider request must
        // not be needed to notice revocation and cancel the diagnostic demand.
        assert!(
            app.state_runtime
                .lock()
                .unwrap()
                .observations
                .node_tasks(&admission, chrono::Utc::now())
                .is_empty()
        );
        assert!(
            changes(&app, before, network)
                .iter()
                .any(|change| matches!(change, CommittedChange::Authorization { .. }))
        );
        assert!(
            app.accounts()
                .authenticate(session.token.expose())
                .await
                .is_err()
        );
    }
    app.shutdown().await.unwrap();
}
