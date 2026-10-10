use super::*;
use crate::observations::service::ObservationOperation;
use crate::state::service::ReadIdentity;
use meshway_configuration::execution::ObservationServiceScope;
use meshway_model::observations::MetricKind;
use meshway_protocol::node_observations::{CreateNodeObservation, NodeObservationTarget};
use meshway_protocol::observations::{CreateObservation, ObservationPath, ObservationPhase};

fn sid() -> ObservationSubscriptionId {
    ObservationSubscriptionId::new(random_uuid().unwrap()).unwrap()
}

fn request(lab: &Lab, service: ObservationServiceScope) -> CreateObservation {
    CreateObservation {
        subscription_id: sid(),
        source_node_id: lab.nodes[0],
        target_node_id: lab.nodes[1],
        selection: ObservationPath::Auto,
        service,
        metrics: vec![MetricKind::Rtt],
        interval_ms: 1000,
    }
}

fn owner(lab: &Lab) -> ReadIdentity {
    ReadIdentity::user_session(&lab.owner, lab.network).unwrap()
}

#[tokio::test]
async fn console_observation_reads_the_netmap_and_renewal_publishes_nothing() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(false).await;
    let [a, b, _] = lab.nodes;
    let before = lab.netmap(a).await.reference();
    let service = lab.app.observation_service(lab.app.netmap_diagnostics());
    let netmaps_before = lab.references().await;
    let created = service
        .create_observation(
            owner(&lab),
            request(&lab, ObservationServiceScope::IcmpEcho),
        )
        .await
        .unwrap();
    assert_eq!(created.phase, ObservationPhase::Preparing);
    assert!(created.unsupported_metrics.is_empty());
    assert_eq!(created.netmap, lab.applied[0]);
    let subscription = created.subscription_id.unwrap();
    // Renewal is the subscription's own lease: before its threshold it is
    // refused by the subscription; neither outcome touches any netmap.
    assert!(matches!(
        service
            .observations(owner(&lab), Some(subscription), ObservationOperation::Renew)
            .await,
        Err(Error::ObservationLimit)
    ));
    let read = service
        .observations(owner(&lab), Some(subscription), ObservationOperation::Read)
        .await
        .unwrap();
    assert_eq!(read.items.len(), 1);
    assert_eq!(read.items[0].id, created.id);
    // Observations never change any netmap.
    assert_eq!(lab.references().await, netmaps_before);
    assert_eq!(lab.netmap(a).await.reference(), before);
    // The decision is the source's netmap decision: the only path is a→b.
    let decision = crate::execution::netmap_diagnostic(
        &lab.netmap(a).await.netmap,
        b,
        &ObservationServiceScope::IcmpEcho,
        &ObservationPath::Auto,
    )
    .unwrap();
    assert_eq!(decision.paths, vec![vec![a, b]]);
    assert!(decision.observable);
    lab.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn denied_and_unsupported_stay_distinct() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(false).await;
    let [a, b, c] = lab.nodes;
    let service = lab.app.observation_service(lab.app.netmap_diagnostics());
    let tcp = |port: u16| ObservationServiceScope::Tcp {
        port: meshway_transport::spec::Port::new(port).unwrap(),
    };
    for (scope, target) in [(tcp(80), b), (ObservationServiceScope::IcmpEcho, c)] {
        let denied = CreateObservation {
            target_node_id: target,
            ..request(&lab, scope)
        };
        assert!(matches!(
            service.create_observation(owner(&lab), denied).await,
            Err(Error::PermissionDenied)
        ));
    }
    // b re-acquires without path observation: the pair stays permitted, the
    // path is no longer observable, and the task is Unsupported, not denied.
    let mut capabilities = caps();
    capabilities.remove("feature:path-observation");
    lab.app
        .execution()
        .acquire(
            &lab.tokens[1],
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities,
            },
        )
        .await
        .unwrap();
    let decision = crate::execution::netmap_diagnostic(
        &lab.netmap(a).await.netmap,
        b,
        &tcp(443),
        &ObservationPath::Auto,
    )
    .unwrap();
    assert!(!decision.observable);
    assert_eq!(decision.paths, vec![vec![a, b]]);
    let unsupported = service
        .create_observation(owner(&lab), request(&lab, tcp(443)))
        .await
        .unwrap();
    assert_eq!(unsupported.phase, ObservationPhase::Unsupported);
    assert_eq!(unsupported.unsupported_metrics, vec![MetricKind::Rtt]);
    lab.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn node_ping_subscription_resolves_and_decides_on_the_source_netmap() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(false).await;
    let b = lab.nodes[1];
    let service = lab.app.observation_service(lab.app.netmap_diagnostics());
    let ping = |target| CreateNodeObservation {
        instance: lab.instances[0].clone(),
        subscription_id: sid(),
        target,
        service: ObservationServiceScope::IcmpEcho,
        selection: ObservationPath::Auto,
        metrics: vec![MetricKind::Rtt],
        interval_ms: 1000,
    };
    let netmaps_before = lab.references().await;
    let created = service
        .create_node_observation(
            lab.tokens[0].clone(),
            ping(NodeObservationTarget::Name { name: "b".into() }),
        )
        .await
        .unwrap();
    assert_eq!(created.target.node_id, b);
    let subscription = created.task.subscription_id.unwrap();
    let expiry = created.task.subscriber_expires_at.unwrap() + chrono::Duration::milliseconds(1);
    // Renewal before its threshold is refused by the subscription itself.
    let early = service
        .node_observation(
            lab.tokens[0].clone(),
            lab.instances[0].clone(),
            subscription,
            Some(expiry.timestamp_millis() as u64),
        )
        .await;
    assert!(
        matches!(early, Err(Error::ObservationLimit | Error::InvalidRequest)),
        "{:?}",
        early.map(|r| r.task.id)
    );
    let read = service
        .node_observation(
            lab.tokens[0].clone(),
            lab.instances[0].clone(),
            subscription,
            None,
        )
        .await
        .unwrap();
    assert_eq!(read.task.id, created.task.id);
    // c is not in a's netmap: it cannot even be named.
    assert!(matches!(
        service
            .create_node_observation(
                lab.tokens[0].clone(),
                ping(NodeObservationTarget::Name { name: "c".into() }),
            )
            .await,
        Err(Error::NotFound)
    ));
    // Observations never change any netmap.
    assert_eq!(lab.references().await, netmaps_before);
    lab.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn account_revocations_end_their_observation_subscriptions_at_commit() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(false).await;
    let service = lab.app.observation_service(lab.app.netmap_diagnostics());
    let execution = lab.app.execution();
    let token = lab.tokens[0].clone();
    let admission = lab
        .app
        .executor
        .handle()
        .execute(move |db| {
            let tx = db.transaction()?;
            execution.report_admission_in_transaction(&tx, &token, chrono::Utc::now())
        })
        .await
        .unwrap()
        .unwrap();
    let tasks = |app: &App| {
        app.state_runtime
            .lock()
            .unwrap()
            .observations
            .node_tasks(&admission, chrono::Utc::now())
            .len()
    };
    let mut password = "a sufficiently long password";
    for operation in 0..4 {
        let session = lab
            .app
            .accounts()
            .login(
                "owner@example.com".into(),
                Secret::new(password.into()),
                rid(),
            )
            .await
            .unwrap();
        let identity = ReadIdentity::user_session(session.token.expose(), lab.network).unwrap();
        service
            .create_observation(identity, request(&lab, ObservationServiceScope::IcmpEcho))
            .await
            .unwrap();
        assert_eq!(tasks(&lab.app), 1, "{operation}");
        let accounts = lab.app.accounts();
        match operation {
            0 => accounts.logout(session.token.expose(), rid()).await.unwrap(),
            1 => accounts
                .revoke_sessions(
                    session.token.expose(),
                    crate::accounts::RevokeSessions::One(session.session.session_id),
                    rid(),
                )
                .await
                .unwrap(),
            2 => {
                accounts
                    .change_password(
                        session.token.expose(),
                        Secret::new(password.into()),
                        Secret::new("replacement password".into()),
                        rid(),
                    )
                    .await
                    .unwrap();
                password = "replacement password";
            }
            _ => {
                let reset = accounts
                    .issue_password_reset("owner@example.com".into(), 600, rid())
                    .await
                    .unwrap();
                accounts
                    .redeem_password_reset(
                        reset.token.expose(),
                        Secret::new("reset replacement password".into()),
                        rid(),
                    )
                    .await
                    .unwrap();
            }
        }
        // The runtime owner itself: no later request is needed to notice.
        assert_eq!(tasks(&lab.app), 0, "{operation}");
        assert!(accounts.authenticate(session.token.expose()).await.is_err());
    }
    lab.app.shutdown().await.unwrap();
}
