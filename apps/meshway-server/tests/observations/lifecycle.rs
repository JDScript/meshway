use super::*;

#[tokio::test]
async fn unavailable_node_instance_never_invalidates_observer_user_session() {
    let server = Server::new().await;
    let user = server.user("node-unavailable@example.com").await;
    let network = server.network(&user).await;
    let a = server.enroll(&user, network.id, "a").await;
    let b = server.enroll(&user, network.id, "b").await;
    server
        .configure(&user, network.id, |cfg| {
            for node in cfg.nodes.values_mut() {
                node.listeners =
                    serde_json::from_str(r#"{"udp":{"type":"native_udp","listenPort":23456}}"#)
                        .unwrap();
            }
        })
        .await;
    let endpoint = format!("/api/v1/networks/{}/observations", network.id);
    let task: ObservationTask = json(
        server.post(&endpoint, user.token.expose(), &request(&a, &b)),
        201,
    )
    .await;
    status(
        server.post(
            "/agent-api/v1/instances/release",
            a.token.expose(),
            &a.instance,
        ),
        204,
    )
    .await;
    status(
        server.post(&endpoint, user.token.expose(), &request(&a, &b)),
        409,
    )
    .await;
    let selected = format!("{endpoint}/{}", task.subscription_id.unwrap());
    status(server.get(&selected, user.token.expose()), 404).await;
    status(
        server.post(&format!("{selected}/renew"), user.token.expose(), &()),
        404,
    )
    .await;
    let list: ObservationList = json(server.get(&endpoint, user.token.expose()), 200).await;
    assert!(list.items.is_empty());
    server.stop().await;
}

#[tokio::test]
async fn renew_is_per_subscription_and_restart_or_node_key_revocation_cannot_restore_tasks() {
    let server = Server::new().await;
    let user = server.user("renew@example.com").await;
    let network = server.network(&user).await;
    let mut a = server.enroll(&user, network.id, "a").await;
    let b = server.enroll(&user, network.id, "b").await;
    server
        .configure(&user, network.id, |cfg| {
            for node in cfg.nodes.values_mut() {
                node.listeners = serde_json::from_value(
                    serde_json::json!({"udp":{"type":"native_udp","listenPort":23456}}),
                )
                .unwrap();
            }
        })
        .await;
    let endpoint = format!("/api/v1/networks/{}/observations", network.id);
    let first = request(&a, &b);
    let second = request(&a, &b);
    let task: ObservationTask =
        json(server.post(&endpoint, user.token.expose(), &first), 201).await;
    let other: ObservationTask =
        json(server.post(&endpoint, user.token.expose(), &second), 201).await;
    status(
        server.post(
            &format!("{endpoint}/{}/renew", first.subscription_id),
            user.token.expose(),
            &(),
        ),
        429,
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_secs(11)).await;
    let renewed: ObservationList = json(
        server.post(
            &format!("{endpoint}/{}/renew", first.subscription_id),
            user.token.expose(),
            &(),
        ),
        200,
    )
    .await;
    assert_eq!(renewed.items.len(), 1);
    let renewed = &renewed.items[0];
    assert!(renewed.subscriber_expires_at > task.subscriber_expires_at);
    assert_ne!(renewed.configuration, task.configuration);
    let untouched: ObservationList = json(
        server.get(
            &format!("{endpoint}/{}", second.subscription_id),
            user.token.expose(),
        ),
        200,
    )
    .await;
    assert_eq!(
        untouched.items[0].subscriber_expires_at,
        other.subscriber_expires_at
    );
    let retry: ObservationTask =
        json(server.post(&endpoint, user.token.expose(), &first), 201).await;
    assert_eq!(retry.subscriber_expires_at, renewed.subscriber_expires_at);
    assert_eq!(retry.configuration, renewed.configuration);
    let server = server.restart().await;
    status(server.get("/agent-api/v1/tasks", a.token.expose()), 401).await;
    let tasks: ObservationList = json(server.get(&endpoint, user.token.expose()), 200).await;
    assert!(tasks.items.is_empty());
    server.resume(&mut a, network.id).await;
    assert!(slice(server.target(&a).await).diagnostics.is_empty());
    let _: ObservationTask = json(
        server.post(&endpoint, user.token.expose(), &request(&a, &b)),
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
    status(server.get("/agent-api/v1/tasks", a.token.expose()), 401).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let tasks: ObservationList = json(server.get(&endpoint, user.token.expose()), 200).await;
    assert!(tasks.items.is_empty());
    server.stop().await;
}
