use super::*;

#[tokio::test]
async fn renewal_checks_current_credentials_and_network_in_its_final_transaction() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    // Each independently enrolled fixture ends permanently. Authentication done
    // before revocation cannot replace the renewal's final-transaction checks.
    for invalidate in [
        "UPDATE node_sessions SET created_at=0,expires_at=1",
        "UPDATE node_keys SET revoked_at=created_at WHERE purpose='control'",
        "UPDATE nodes SET credentials_revoked_at=registered_at",
        "UPDATE networks SET deleted_at=created_at",
    ] {
        let (_dir, app, network, a, b, ak, _, signer_a, _) = fixture().await;
        let service = app.execution();
        let at = session(&service, network, a, &ak, &signer_a).await;
        let ai = acquire(&app, &at).await;
        let create = demand(&ai, b);
        service
            .demand(at.token.expose(), create.clone())
            .await
            .unwrap();
        service.authenticate(at.token.expose()).await.unwrap();
        let before = snapshot(&app, network).await;
        app.executor
            .handle()
            .execute(move |db| {
                db.execute_batch(invalidate).unwrap();
            })
            .await
            .unwrap();
        assert_eq!(
            service
                .renew_demand(at.token.expose(), renewal(&create, ai.expires_at_ms))
                .await,
            Err(Error::InvalidCredentials)
        );
        assert_eq!(snapshot(&app, network).await, before);
        app.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn renewal_enforces_maximum_window_even_with_a_later_instance_deadline() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, _, signer_a, _) = fixture().await;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &signer_a).await;
    let ai = acquire(&app, &at).await;
    let create = demand(&ai, b);
    service
        .demand(at.token.expose(), create.clone())
        .await
        .unwrap();
    let later = store::execution::now().unwrap() + 600_000;
    app.executor.handle().execute(move |db| {
        db.execute("UPDATE node_execution SET instance_expires_at=?1 WHERE network_id=?2 AND node_id=?3", rusqlite::params![later, network.as_uuid().as_bytes().as_slice(), a.as_uuid().as_bytes().as_slice()]).unwrap();
    }).await.unwrap();
    let before = snapshot(&app, network).await;
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&create, later as u64))
            .await,
        Err(Error::InvalidRequest)
    );
    assert_eq!(snapshot(&app, network).await, before);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn another_network_cannot_renew_a_demand_with_its_own_authenticated_instance() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, _, signer_a, _) = fixture().await;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &signer_a).await;
    let ai = acquire(&app, &at).await;
    let create = demand(&ai, b);
    service
        .demand(at.token.expose(), create.clone())
        .await
        .unwrap();
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let other = app
        .networks()
        .create(owner.token.expose(), "other".into(), rid())
        .await
        .unwrap();
    app.configurations()
        .apply(
            owner.token.expose(),
            other.id,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: other.current_revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::ReplaceTagsOwners {
                    tags_owners: std::collections::BTreeMap::from([("tag:test".into(), vec![])]),
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    let signer = ed25519_dalek::SigningKey::from_bytes(&[99; 32]);
    let enrolled = super::super::node::enroll_tag_node(
        &app,
        owner.token.expose(),
        other.id,
        &signer,
        "other",
        [98; 32],
    )
    .await;
    let other_token = session(
        &service,
        other.id,
        enrolled.node_id,
        &enrolled.keys,
        &signer,
    )
    .await;
    let other_lease = acquire(&app, &other_token).await;
    let before = snapshot(&app, network).await;
    let mut request = renewal(&create, other_lease.expires_at_ms);
    request.instance = instance(&other_lease);
    assert_eq!(
        service
            .renew_demand(other_token.token.expose(), request)
            .await,
        Err(Error::BindingConflict)
    );
    assert_eq!(snapshot(&app, network).await, before);
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[test]
fn renewal_route_uses_node_session_and_an_absolute_timestamp_contract() {
    let spec = serde_json::to_value(crate::http::openapi()).unwrap();
    let route = &spec["paths"]["/agent-api/v1/demands/renew"]["post"];
    assert_eq!(route["operationId"], "renewNodeDemand");
    assert_eq!(
        route["security"],
        serde_json::json!([{ "nodeSession": [] }])
    );
    assert_eq!(
        route["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/RenewDemandRequest"
    );
    assert_eq!(
        spec["components"]["schemas"]["RenewDemandRequest"]["properties"]["expiresAt"]["$ref"],
        "#/components/schemas/Timestamp"
    );
    assert_eq!(
        spec["components"]["schemas"]["Timestamp"]["format"],
        "meshway-timestamp-ms"
    );
}
