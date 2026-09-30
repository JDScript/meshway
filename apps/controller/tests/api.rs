use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use meshway_controller::{AppState, openapi_json, router};
use meshway_protocol::{AgentReport, ErrorResponse, NodeList, Overview, ReportResponse};
use tower::ServiceExt;

async fn get<T: serde::de::DeserializeOwned>(app: &Router, path: &str) -> T {
    let response = app
        .clone()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap()
}

async fn report(app: &Router, body: String, content_type: &str) -> (StatusCode, Vec<u8>) {
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/agents/report")
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    (
        response.status(),
        to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap()
            .to_vec(),
    )
}

#[tokio::test]
async fn reports_are_real_sorted_and_upserted_without_invented_initial_nodes() {
    let app = router(AppState::default());
    assert_eq!(
        get::<Overview>(&app, "/api/v1/overview").await.node_count,
        0
    );
    assert!(
        get::<NodeList>(&app, "/api/v1/nodes")
            .await
            .nodes
            .is_empty()
    );
    let before = chrono::Utc::now();
    for (id, name) in [("z", "Zürich"), ("a", "nyc"), ("z", "updated")] {
        let body = serde_json::to_string(&AgentReport {
            node_id: id.into(),
            name: name.into(),
            agent_version: "0.1.0".into(),
        })
        .unwrap();
        let (status, bytes) = report(&app, body, "application/json").await;
        assert_eq!(status, StatusCode::OK);
        let stored: ReportResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(stored.node.id, id);
        assert_eq!(stored.node.name, name);
        assert!(stored.node.last_seen >= before && stored.node.last_seen <= chrono::Utc::now());
    }
    let nodes = get::<NodeList>(&app, "/api/v1/nodes").await.nodes;
    assert_eq!(
        nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "z"]
    );
    assert_eq!(nodes[1].name, "updated");
    assert_eq!(
        get::<Overview>(&app, "/api/v1/overview").await.node_count,
        2
    );
    assert!(
        get::<NodeList>(&router(AppState::default()), "/api/v1/nodes")
            .await
            .nodes
            .is_empty()
    );
}

#[tokio::test]
async fn invalid_or_oversized_reports_are_rejected_without_mutating_state() {
    let app = router(AppState::default());
    for body in [
        r#"{"node_id":"","name":"nyc","agent_version":"0.1.0"}"#.to_owned(),
        r#"{"node_id":"n","name":" \t","agent_version":"0.1.0"}"#.to_owned(),
        r#"{"node_id":"n","name":"nyc","agent_version":""}"#.to_owned(),
        r#"{"node_id":"n","name":"nyc","agent_version":"0.1.0","secret":"unexpected"}"#.to_owned(),
        r#"{"node_id":"n","name":"nyc"}"#.to_owned(),
        r#"{"node_id":3,"name":"nyc","agent_version":"0.1.0"}"#.to_owned(),
        "{".to_owned(),
        serde_json::to_string(&AgentReport {
            node_id: "x".repeat(129),
            name: "nyc".into(),
            agent_version: "0.1.0".into(),
        })
        .unwrap(),
        serde_json::to_string(&AgentReport {
            node_id: "n".into(),
            name: "x".repeat(129),
            agent_version: "0.1.0".into(),
        })
        .unwrap(),
        serde_json::to_string(&AgentReport {
            node_id: "n".into(),
            name: "nyc".into(),
            agent_version: "x".repeat(65),
        })
        .unwrap(),
    ] {
        let (status, bytes) = report(&app, body, "application/json").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            !serde_json::from_slice::<ErrorResponse>(&bytes)
                .unwrap()
                .message
                .is_empty()
        );
    }
    let (status, _) = report(&app, " ".repeat(8193), "application/json").await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let (status, _) = report(&app, "{}".into(), "text/plain").await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(
        get::<Overview>(&app, "/api/v1/overview").await.node_count,
        0
    );
}

#[test]
fn committed_openapi_matches_live_routes_and_current_version() {
    assert_eq!(
        openapi_json().unwrap(),
        include_str!("../../../packages/protocol/openapi.json"),
        "Run pnpm --filter @meshway/control-client generate"
    );
}
