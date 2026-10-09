use super::*;
use meshway_protocol::{configuration::ConfigurationRequest, observations::*};

async fn publications(app: &App) -> i64 {
    app.executor
        .handle()
        .execute(|db| {
            db.query_row("SELECT count(*) FROM execution_publications", [], |r| {
                r.get(0)
            })
            .unwrap()
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn diagnostic_results_and_identical_refreshes_reuse_publication() {
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            ConfigurationRequest::Operations {
                expected_revision: current.revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::ReplaceAccess {
                    access: vec![AccessRule {
                        remark: String::new(),
                        src: vec![format!("node:{a}")],
                        dst: vec![format!("node:{b}")],
                        ip: vec!["icmp:*".into()],
                        bidirectional: false,
                    }],
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    let service = app.execution();
    let source = session(&service, network, a, &ak, &signer_a).await;
    let target = session(&service, network, b, &bk, &signer_b).await;
    let mut instances = Vec::new();
    for token in [source.token.expose(), target.token.expose()] {
        let lease = service
            .acquire(
                token,
                AcquireInstanceRequest {
                    instance_id: iid(),
                    capabilities: caps(),
                },
            )
            .await
            .unwrap();
        instances.push(InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        });
    }
    // Drive maintenance explicitly below, using a virtual renewal instant.
    let observations = app.observation_service(app.execution());
    let subscription_id = ObservationSubscriptionId::new(random_uuid().unwrap()).unwrap();
    let before = publications(&app).await;
    let accepted = observations
        .create_observation(
            crate::state::service::ReadIdentity::UserSession {
                digest: crate::accounts::token_digest(owner.token.expose()).unwrap(),
                network,
            },
            CreateObservation {
                subscription_id,
                source_node_id: a,
                target_node_id: b,
                service: ObservationServiceScope::IcmpEcho,
                selection: ObservationPath::Auto,
                metrics: vec![meshway_model::observations::MetricKind::Rtt],
                interval_ms: 1000,
            },
        )
        .await
        .unwrap();
    assert_eq!(publications(&app).await, before + 1);
    for _ in 0..5 {
        for (token, instance) in [source.token.expose(), target.token.expose()]
            .into_iter()
            .zip(&instances)
        {
            service
                .refresh_configuration(
                    token,
                    NodeConfigurationRequest {
                        instance: instance.clone(),
                        base: None,
                    },
                )
                .await
                .unwrap();
        }
        observations
            .observation_result(
                source.token.expose().to_owned(),
                accepted.id,
                ObservationResult {
                    instance_id: instances[0].instance_id,
                    configuration: accepted.configuration.clone(),
                    result: ObservationUpdate::Phase {
                        phase: ReportedObservationPhase::Preparing,
                    },
                },
            )
            .await
            .unwrap();
        // Exercise the publisher boundary as well: a refresh request with
        // identical authorized inputs must not allocate an empty publication.
        let sealer = app.sealer.clone();
        app.executor
            .handle()
            .execute(move |db| {
                let tx = db.transaction().unwrap();
                store::execution::publish(
                    &tx,
                    &sealer,
                    network,
                    "refresh",
                    store::execution::now().unwrap(),
                )
                .unwrap();
                tx.commit().unwrap();
            })
            .await
            .unwrap();
        let task = observations
            .node_tasks(source.token.expose().to_owned())
            .await
            .unwrap()
            .items
            .remove(0);
        assert_eq!(task.configuration, accepted.configuration);
        assert_eq!(
            publications(&app).await,
            before + 1,
            "unchanged compiled slices must not allocate empty publications"
        );
    }
    // The current immutable wire includes the permit deadline. A real renewal
    // therefore needs one successor; replaying the same confirmed lease does not.
    let now = chrono::Utc::now() + chrono::Duration::seconds(10);
    let expires_at = accepted.subscriber_expires_at.unwrap() + chrono::Duration::seconds(10);
    for _ in 0..2 {
        let execution = app.execution();
        app.executor
            .handle()
            .execute(move |db| {
                let tx = db.transaction().unwrap();
                crate::observations::service::ObservationProvider::synchronize(
                    &execution,
                    &tx,
                    &[crate::observations::service::SubscriptionLease {
                        id: subscription_id,
                        expires_at,
                    }],
                    now,
                )
                .unwrap();
                tx.commit().unwrap();
            })
            .await
            .unwrap();
        assert_eq!(publications(&app).await, before + 2);
    }
    let renewed = service
        .configuration(
            source.token.expose(),
            NodeConfigurationRequest {
                instance: instances[0].clone(),
                base: None,
            },
        )
        .await
        .unwrap()
        .full
        .unwrap();
    assert_ne!(
        renewed.configuration.publication,
        accepted.configuration.execution_publication
    );
    assert_eq!(
        renewed.configuration.diagnostics[&accepted.id].expires_at_ms,
        expires_at.timestamp_millis() as u64
    );
    assert!(
        observations
            .observation_result(
                source.token.expose().to_owned(),
                accepted.id,
                ObservationResult {
                    instance_id: instances[0].instance_id,
                    configuration: accepted.configuration,
                    result: ObservationUpdate::Phase {
                        phase: ReportedObservationPhase::Preparing
                    },
                }
            )
            .await
            .is_err(),
        "old output must not be relabeled with the renewed permit"
    );
    app.shutdown().await.unwrap();
}
