use super::*;

#[tokio::test]
async fn node_absolute_renewal_does_not_slide_on_retry_and_restart_or_revocation_does_not_restore_it()
 {
    // C: the Node subscription's renew_after (10 s) must pass in real time;
    // replaying an absolute renewal must not slide the confirmed deadline.
    let server = Server::new().await;
    let user = server.user("node-lease@example.com").await;
    let network = server.network(&user).await;
    let mut a = server.enroll(&user, network.id, "a").await;
    let b = server.enroll(&user, network.id, "b").await;
    server
        .configure(&user, network.id, |cfg| {
            for node in cfg.nodes.values_mut() {
                node.listeners =
                    serde_json::from_str(r#"{"udp":{"type":"native_udp","listenPort":23456}}"#)
                        .unwrap();
            }
            cfg.access = vec![meshway_configuration::AccessRule {
                remark: String::new(),
                src: vec![format!("node:{}", a.id)],
                dst: vec![format!("node:{}", b.id)],
                ip: vec!["icmp:*".into()],
                bidirectional: false,
            }];
        })
        .await;
    let first = CreateNodeObservation {
        service: ObservationServiceScope::IcmpEcho,
        ..demand(&a, "b")
    };
    let created: NodeObservationResponse = json(
        server.post("/agent-api/v1/observations", a.token.expose(), &first),
        201,
    )
    .await;
    status(
        server.post(
            "/agent-api/v1/observations",
            a.token.expose(),
            &demand(&a, "b"),
        ),
        403,
    )
    .await;
    let invalid = RenewNodeObservation {
        instance: a.instance.clone(),
        // A 31 s client deadline becomes legal after 1 s of HTTP scheduling.
        // Exercise the invalid upper bound without racing that transport delay.
        expires_at_ms: (chrono::Utc::now() + chrono::Duration::days(1)).timestamp_millis() as u64,
    };
    status(
        server.post(
            &format!("{}/renew", selected(&first)),
            a.token.expose(),
            &invalid,
        ),
        400,
    )
    .await;
    wait_until(created.task.renew_after).await;
    server.target(&a).await;
    let read: NodeObservationResponse = json(
        server
            .get(&selected(&first), a.token.expose())
            .query(&query(&a)),
        200,
    )
    .await;
    assert_eq!(
        read.task.subscriber_expires_at,
        created.task.subscriber_expires_at
    );
    let renewal = RenewNodeObservation {
        instance: a.instance.clone(),
        expires_at_ms: chrono::Utc::now().timestamp_millis() as u64 + 29_000,
    };
    let renewed: NodeObservationResponse = json(
        server.post(
            &format!("{}/renew", selected(&first)),
            a.token.expose(),
            &renewal,
        ),
        200,
    )
    .await;
    assert_pre_expiry_budget(&created.task);
    assert_eq!(
        renewed
            .task
            .subscriber_expires_at
            .unwrap()
            .timestamp_millis() as u64,
        renewal.expires_at_ms
    );
    let replay: NodeObservationResponse = json(
        server.post(
            &format!("{}/renew", selected(&first)),
            a.token.expose(),
            &renewal,
        ),
        200,
    )
    .await;
    assert_eq!(
        replay.task.subscriber_expires_at,
        renewed.task.subscriber_expires_at
    );
    assert_eq!(replay.task.renew_after, renewed.task.renew_after);
    let server = server.restart().await;
    status(
        server
            .get(&selected(&first), a.token.expose())
            .query(&query(&a)),
        401,
    )
    .await;
    server.resume(&mut a, network.id).await;
    let tasks: ObservationList =
        json(server.get("/agent-api/v1/tasks", a.token.expose()), 200).await;
    assert!(tasks.items.is_empty());
    status(
        server.post(
            "/agent-api/v1/observations",
            a.token.expose(),
            &CreateNodeObservation {
                instance: a.instance.clone(),
                ..first.clone()
            },
        ),
        409,
    )
    .await;
    status(
        server.post("/agent-api/v1/observations", a.token.expose(), &first),
        401,
    )
    .await;
    let fresh = CreateNodeObservation {
        instance: a.instance.clone(),
        subscription_id: id(),
        ..first.clone()
    };
    let _: NodeObservationResponse = json(
        server.post("/agent-api/v1/observations", a.token.expose(), &fresh),
        201,
    )
    .await;
    status(
        server.post(
            &format!("/api/v1/networks/{}/nodes/{}/keys/revoke", network.id, a.id),
            user.token.expose(),
            &meshway_protocol::RevokeNodeKeyRequest {
                key_id: a.keys.control.id,
                expected_generation: a.keys.control.generation,
            },
        ),
        204,
    )
    .await;
    status(
        server
            .get(&selected(&fresh), a.token.expose())
            .query(&query(&a)),
        401,
    )
    .await;
    server.stop().await;
}
