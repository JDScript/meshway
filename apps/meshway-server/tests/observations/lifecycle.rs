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
