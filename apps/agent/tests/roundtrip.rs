use axum::{Router, routing::post};
use meshway_agent::generated::{
    client::{HttpClient, ReportAgentApiError},
    types::{AgentReport, ReportResponse},
};
use meshway_controller::{AppState, router};

async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, task)
}

fn report(name: &str) -> AgentReport {
    AgentReport {
        node_id: "test-node".to_owned(),
        name: name.to_owned(),
        agent_version: "0.1.0".to_owned(),
    }
}

#[tokio::test]
async fn generated_report_operation_roundtrips_and_decodes_typed_errors() {
    let (base, server) = serve(router(AppState::default())).await;
    let client = HttpClient::new().with_base_url(base);
    let first = client.report_agent(report("nyc")).await.unwrap();
    let updated = client.report_agent(report("new name")).await.unwrap();
    assert_eq!(first.node.id, updated.node.id);
    assert_eq!(updated.node.name, "new name");
    assert!(updated.node.last_seen >= first.node.last_seen);
    let error = client.report_agent(report(" ")).await.unwrap_err();
    let envelope = error.api().unwrap();
    assert_eq!(envelope.status, 400);
    assert!(matches!(
        envelope.typed,
        Some(ReportAgentApiError::Status400(_))
    ));
    server.abort();
}

#[tokio::test]
async fn generated_client_rejects_invalid_response_fields_and_dates() {
    for body in [
        r#"{"node":{"id":"n","name":"nyc","agent_version":"0.1.0","last_seen":"not-a-date"}}"#,
        r#"{"node":{"id":7,"name":"nyc","agent_version":"0.1.0","last_seen":"2026-09-30T00:00:00Z"}}"#,
        r#"{"node":{"id":"n"}}"#,
    ] {
        let app = Router::new().route(
            "/api/v1/agents/report",
            post(move || async move { ([("content-type", "application/json")], body) }),
        );
        let (base, server) = serve(app).await;
        let error = HttpClient::new()
            .with_base_url(base)
            .report_agent(report("nyc"))
            .await
            .unwrap_err();
        assert!(error.api().unwrap().parse_error.is_some());
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_binary_reports_once_with_deterministic_default_identity() {
    let (base, server) = serve(router(AppState::default())).await;
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
        .args(["report", "--controller", &base, "--name", "nyc"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: ReportResponse = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result.node.id, "nyc");
    assert_eq!(result.node.name, "nyc");
    assert_eq!(result.node.agent_version, env!("CARGO_PKG_VERSION"));
    server.abort();
}
