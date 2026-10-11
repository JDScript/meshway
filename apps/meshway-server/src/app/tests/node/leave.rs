//! A Node deletes itself over HTTP with its own session: the same permanent
//! deletion a manager performs, never another Node's.
use super::*;

async fn post(
    client: &reqwest::Client,
    base: &str,
    path: &str,
    bearer: &str,
    body: serde_json::Value,
) -> reqwest::Response {
    client
        .post(format!("{base}{path}"))
        .bearer_auth(bearer)
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn version(app: &App, network: NetworkId, revision: Revision) -> (Vec<u8>, String) {
    app.executor
        .handle()
        .execute(move |db| {
            db.query_row(
                "SELECT created_by,cause FROM configuration_versions WHERE network_id=?1 AND revision=?2",
                rusqlite::params![network.as_uuid().as_bytes().as_slice(), revision.get() as i64],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn a_node_deletes_only_itself_and_its_identity_stays_revoked() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let execution = app.execution();
    let token_a = session(&execution, network, a, &ak, &signer_a).await;
    let token_b = session(&execution, network, b, &bk, &signer_b).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = crate::http::router(&app);
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured(meshway_transport_stream::tls::platform_client_config().unwrap())
        .build()
        .unwrap();
    let bearer_a = token_a.token.expose().to_owned();

    // A cannot delete B: the request names the session's own Node only.
    let other = post(
        &client,
        &base,
        "/agent-api/v1/leave",
        &bearer_a,
        serde_json::json!({ "requestId": rid(), "nodeId": b }),
    )
    .await;
    assert_eq!(other.status(), 403);
    let problem: meshway_protocol::ErrorResponse = other.json().await.unwrap();
    assert!(matches!(
        problem.problem,
        meshway_protocol::Problem::PermissionDenied { .. }
    ));
    let before = app.state_runtime.lock().unwrap().hub.events.sequence();

    let left = post(
        &client,
        &base,
        "/agent-api/v1/leave",
        &bearer_a,
        serde_json::json!({ "requestId": rid(), "nodeId": a }),
    )
    .await;
    assert_eq!(left.status(), 200);
    let left: NodeLeaveResponse = left.json().await.unwrap();
    assert_eq!((left.network_id, left.node_id), (network, a));

    // One configuration version without A, by the user who admitted A, with
    // the Node as the notice's actor.
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let snapshot = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert_eq!(snapshot.revision, left.revision);
    assert!(!snapshot.document.configuration.nodes.contains_key(&a));
    assert!(snapshot.document.configuration.nodes.contains_key(&b));
    let (author, cause) = version(&app, network, left.revision).await;
    assert_eq!(author, owner.session.user.id.as_uuid().as_bytes().to_vec());
    assert_eq!(cause, "nodeDeleted");
    let notices: Vec<_> = app
        .state_runtime
        .lock()
        .unwrap()
        .hub
        .events
        .after(before, network)
        .unwrap()
        .into_iter()
        .filter_map(|entry| match entry.change {
            crate::effect::Effect::Configuration { notice } => Some(notice),
            _ => None,
        })
        .collect();
    assert_eq!(notices.len(), 1);
    assert!(matches!(
        notices[0].actor,
        meshway_protocol::events::EventActor::Node { node_id } if node_id == a
    ));

    // The inventory no longer lists A; B keeps working.
    let inventory = execution
        .inventory(owner.token.expose(), network)
        .await
        .unwrap();
    assert!(inventory.items.iter().all(|node| node.id != a));
    let synced = post(
        &client,
        &base,
        "/agent-api/v1/sync",
        token_b.token.expose(),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(synced.status(), 200);

    // A's sessions ended with its credentials: its old bearer neither syncs
    // nor leaves again, and its key can never sign in again.
    for path in ["/agent-api/v1/sync", "/agent-api/v1/leave"] {
        let body = if path.ends_with("leave") {
            serde_json::json!({ "requestId": rid(), "nodeId": a })
        } else {
            serde_json::json!({})
        };
        assert_eq!(
            post(&client, &base, path, &bearer_a, body).await.status(),
            401,
            "{path}"
        );
    }
    let intent = NodeSessionIntent { request_id: rid() };
    let challenge = execution
        .challenge(execution.signed_challenge_fixture(
            NodeChallengeRequest {
                network_id: network,
                node_id: a,
                key_id: ak.control.id,
                operation: ProofOperation::Session,
                request_digest: proof::request_digest(&intent).unwrap(),
            },
            &signer_a,
        ))
        .await;
    assert!(matches!(challenge, Err(Error::NodeIdentityRejected)));
    server.abort();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_user_device_leaves_under_its_own_user() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, _, _, _, _, _, _) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let signer = SigningKey::from_bytes(&[9; 32]);
    let device = enroll_user_node(&app, owner.token.expose(), network, &signer, [19; 32]).await;
    let execution = app.execution();
    let token = session(&execution, network, device.node_id, &device.keys, &signer).await;
    let left = execution
        .leave(
            app.configurations().committer(),
            token.token.expose(),
            NodeLeaveRequest {
                request_id: rid(),
                node_id: device.node_id,
            },
        )
        .await
        .unwrap();
    let (author, cause) = version(&app, network, left.revision).await;
    assert_eq!(author, owner.session.user.id.as_uuid().as_bytes().to_vec());
    assert_eq!(cause, "nodeDeleted");
    let revoked: bool = app
        .executor
        .handle()
        .execute(move |db| {
            db.query_row(
                "SELECT credentials_revoked_at IS NOT NULL FROM nodes WHERE id=?1",
                [device.node_id.as_uuid().as_bytes().as_slice()],
                |row| row.get(0),
            )
            .unwrap()
        })
        .await
        .unwrap();
    assert!(revoked);
    app.shutdown().await.unwrap();
}
