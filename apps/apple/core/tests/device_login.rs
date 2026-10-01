use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use meshway_apple_core::{
    SignInError, begin_authorization, exchange_device, get_device_configuration, get_device_dns,
    heartbeat, new_node_credential,
};
use meshway_controller::{AppState, admin_token, router};
use meshway_protocol::{
    AuthorizeDeviceRequest, AuthorizeDeviceResponse, DEVICE_CALLBACK_URI, NetworkList,
    RevokeNodeRequest, s256,
};
use tower::ServiceExt;

async fn initial_network(app: &axum::Router, admin: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/networks")
                .header("authorization", format!("Bearer {admin}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let networks: NetworkList =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(
        networks.networks.len(),
        1,
        "fixture requires a unique initial network"
    );
    networks.networks[0].network_id.clone()
}

#[tokio::test]
async fn real_controller_exchange_recovery_heartbeat_and_revocation() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let app = router(AppState::open(directory.path()).unwrap());
    let admin = admin_token(directory.path()).unwrap();
    let network_id = initial_network(&app, &admin).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(axum::serve(listener, app.clone()).into_future());
    let attempt = begin_authorization(server_url.clone(), "Mac".into(), "v1".into()).unwrap();
    let authorization = AuthorizeDeviceRequest {
        redirect_uri: DEVICE_CALLBACK_URI.into(),
        code_challenge: s256(&attempt.verifier),
        name: "Mac".into(),
        agent_version: "v1".into(),
    };
    let response = app
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/v1/networks/{network_id}/device-authorizations"
            ))
            .header("authorization", format!("Bearer {admin}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&authorization).unwrap()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response: AuthorizeDeviceResponse =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    let credential = new_node_credential().unwrap();
    let exchange = || {
        exchange_device(
            server_url.clone(),
            response.code.clone(),
            attempt.verifier.clone(),
            credential.clone(),
            "Mac".into(),
            "v1".into(),
        )
    };
    let node = exchange().await.unwrap();
    let inventory =
        get_device_configuration(server_url.clone(), credential.clone(), node.id.clone())
            .await
            .unwrap();
    assert_eq!(inventory.own_device.id, node.id);
    assert_eq!(inventory.own_device.mesh_ipv4, node.mesh_ipv4);
    assert!(inventory.peers.is_empty());
    assert!(matches!(
        get_device_configuration(
            server_url.clone(),
            credential.clone(),
            new_node_credential().unwrap()
        )
        .await,
        Err(SignInError::Server)
    ));
    let dns = get_device_dns(server_url.clone(), credential.clone(), node.id.clone())
        .await
        .unwrap();
    assert!(dns.revision > 0);
    assert_eq!(exchange().await.unwrap().id, node.id);
    assert!(matches!(
        heartbeat(server_url.clone(), new_node_credential().unwrap()).await,
        Err(SignInError::CredentialRejected)
    ));
    assert_eq!(
        heartbeat(server_url.clone(), credential.clone())
            .await
            .unwrap()
            .id,
        node.id
    );
    let revoked = app
        .oneshot(
            Request::post(format!("/api/v1/networks/{network_id}/nodes/revoke"))
                .header("authorization", format!("Bearer {admin}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&RevokeNodeRequest {
                        node_id: node.id.clone(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked.status(), 200);
    assert!(matches!(
        get_device_configuration(server_url.clone(), credential.clone(), node.id.clone()).await,
        Err(SignInError::Revoked)
    ));
    assert!(matches!(
        get_device_dns(server_url.clone(), credential.clone(), node.id).await,
        Err(SignInError::Revoked)
    ));
    assert!(matches!(
        heartbeat(server_url.clone(), credential.clone()).await,
        Err(SignInError::Revoked)
    ));
    assert!(matches!(exchange().await, Err(SignInError::Revoked)));
    server.abort();
}
