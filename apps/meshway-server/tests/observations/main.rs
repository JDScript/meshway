//! Normal HTTP, PKCE and held-key credentials. Measurements below are explicitly
//! authenticated Node report fixtures, not evidence of packet-level measurement.
mod authorization;
mod lifecycle;
mod node;
mod support;
use meshway_configuration::execution::*;
use meshway_model::{observations::*, *};
use meshway_protocol::{execution::*, observations::*};
use support::*;

fn request(a: &Node, b: &Node) -> CreateObservation {
    CreateObservation {
        subscription_id: id(),
        source_node_id: a.id,
        target_node_id: b.id,
        service: ObservationServiceScope::Tcp {
            port: 443.try_into().unwrap(),
        },
        selection: ObservationPath::Auto,
        metrics: vec![MetricKind::Rtt, MetricKind::Traffic],
        interval_ms: 1000,
    }
}

#[tokio::test]
async fn background_expiry_and_session_revocation_never_renew_diagnostic_authority() {
    // C: OBSERVATION_LEASE_SECONDS (30 s) itself must elapse without a
    // subscriber renew; a shorter wait cannot prove expiry of this authority.
    let server = Server::new().await;
    let user = server.user("expiry@example.com").await;
    let network = server.network(&user).await;
    let a = server.enroll(&user, network.id, "a").await;
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
    let task: ObservationTask =
        json(server.post(&endpoint, user.token.expose(), &first), 201).await;
    server.target(&a).await;
    // Keep only the Node instance alive. Neither this nor elapsed time is a
    // Console subscription renewal; there is no state/observation/SSE polling.
    assert_pre_expiry_budget(&task);
    wait_until(task.expires_at).await;
    let _: InstanceLease = json(
        server.post(
            "/agent-api/v1/instances/renew",
            a.token.expose(),
            &a.instance,
        ),
        200,
    )
    .await;
    assert!(chrono::Utc::now() >= task.expires_at);
    // Expiry is applied by the next transaction that sweeps the runtime, not
    // at the deadline instant; converge on the projection rather than racing it.
    wait_no_diagnostics(&server, &a).await;
    let items: ObservationList =
        json(server.get("/agent-api/v1/tasks", a.token.expose()), 200).await;
    assert!(items.items.is_empty());
    let fresh = request(&a, &b);
    let _: ObservationTask = json(server.post(&endpoint, user.token.expose(), &fresh), 201).await;
    status(
        server.post("/api/v1/auth/logout", user.token.expose(), &()),
        204,
    )
    .await;
    wait_no_diagnostics(&server, &a).await;
    server.stop().await;
}
