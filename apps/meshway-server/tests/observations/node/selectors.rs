use super::*;

#[tokio::test]
async fn node_get_decodes_required_uri_scalars_without_relaxing_json_or_instance_authorization() {
    let server = Server::new().await;
    let user = server.user("node-uri-query@example.com").await;
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
    let request = demand(&a, "b");
    let created: NodeObservationResponse = json(
        server.post("/agent-api/v1/observations", a.token.expose(), &request),
        201,
    )
    .await;
    let path = selected(&request);
    let read: NodeObservationResponse =
        json(server.get(&path, a.token.expose()).query(&query(&a)), 200).await;
    assert_eq!(read.task.id, created.task.id);
    assert_eq!(
        read.task.subscriber_expires_at,
        created.task.subscriber_expires_at
    );
    let instance = a.instance.instance_id;
    let generation = a.instance.generation;
    for suffix in [
        String::new(),
        format!("?instanceId={instance}"),
        format!("?generation={generation}"),
        format!("?instanceId=&generation={generation}"),
        format!("?instanceId=not-a-uuid&generation={generation}"),
        format!("?instanceId=00000000-0000-0000-0000-000000000000&generation={generation}"),
        format!("?instanceId={instance}&generation={generation}&generation={generation}"),
        format!("?instanceId={instance}&instanceId={instance}&generation={generation}"),
        format!("?instanceId={instance}&generation={generation}&unknown=1"),
    ] {
        status(
            server.get(&format!("{path}{suffix}"), a.token.expose()),
            400,
        )
        .await;
    }
    for malformed in [
        "",
        "zero",
        "0",
        "-1",
        "%2B1",
        "%201",
        "1.0",
        "1e0",
        "9007199254740992",
        "18446744073709551616",
    ] {
        status(
            server.get(
                &format!("{path}?instanceId={instance}&generation={malformed}"),
                a.token.expose(),
            ),
            400,
        )
        .await;
    }
    // Valid URI scalars reach the existing instance authorization check.
    for foreign_generation in [generation.checked_next().unwrap(), Version::MAX] {
        status(
            server
                .get(&path, a.token.expose())
                .query(&NodeObservationQuery {
                    instance_id: instance,
                    generation: foreign_generation,
                }),
            401,
        )
        .await;
    }
    status(
        server
            .get(&path, a.token.expose())
            .query(&NodeObservationQuery {
                instance_id: b.instance.instance_id,
                generation,
            }),
        401,
    )
    .await;
    status(server.get(&path, b.token.expose()).query(&query(&b)), 403).await;
    // JSON remains strict: a string generation is not a Version number.
    status(
        server.post(
            &format!("{path}/cancel"),
            a.token.expose(),
            &serde_json::json!({ "instanceId": instance, "generation": generation.to_string() }),
        ),
        400,
    )
    .await;
    let after: NodeObservationResponse =
        json(server.get(&path, a.token.expose()).query(&query(&a)), 200).await;
    assert_eq!(after.task.id, created.task.id);
    assert_eq!(
        after.task.subscriber_expires_at,
        created.task.subscriber_expires_at
    );
    server.stop().await;
}

