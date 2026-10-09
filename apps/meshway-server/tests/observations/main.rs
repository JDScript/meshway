//! Normal HTTP, PKCE and held-key credentials. Measurements below are explicitly
//! authenticated Node report fixtures, not evidence of packet-level measurement.
mod authorization;
mod lifecycle;
mod node;
mod support;
use meshway_configuration::execution::*;
use meshway_model::{observations::*, state::*, *};
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
fn slice(target: NodeConfigurationResponse) -> NodeConfiguration {
    target.full.expect("full target required").configuration
}

/// A real subscriber renews once `renew_after` has passed. The subscriber
/// lease is 30 s and a loaded CI host can spend longer than that inside one
/// flow; the flow asserts cancellation isolation, not lease expiry. Renewal
/// republishes the diagnostic demand: callers renew before fetching a report
/// configuration, or after the last result using that configuration.
async fn keep_alive(
    server: &Server,
    user: &meshway_protocol::SessionIssuedResponse,
    endpoint: &str,
    task: ObservationTask,
) -> ObservationTask {
    if chrono::Utc::now() < task.renew_after {
        return task;
    }
    let subscription = task.subscription_id.unwrap();
    eprintln!(
        "subscriber renewal {subscription}: now={}, renew_after={}, subscriber_expires_at={:?}, phase={:?}",
        chrono::Utc::now(),
        task.renew_after,
        task.subscriber_expires_at,
        task.phase,
    );
    let renewed: ObservationList = json(
        server.post(
            &format!("{endpoint}/{subscription}/renew"),
            user.token.expose(),
            &(),
        ),
        200,
    )
    .await;
    renewed
        .items
        .into_iter()
        .find(|item| item.subscription_id == Some(subscription))
        .expect("renewed subscription must remain present")
}

fn sample_result(
    running: &ObservationTask,
    a: &Node,
    path: Vec<NodeId>,
    segment: Sequence,
    now: chrono::DateTime<chrono::Utc>,
    event: EventId,
) -> ObservationResult {
    ObservationResult {
        instance_id: a.instance.instance_id,
        configuration: running.configuration.clone(),
        result: ObservationUpdate::Sample {
            path: path.clone(),
            path_segment: segment,
            sample: Box::new(ObservationSample {
                measurement_id: event,
                version: Version::MIN,
                window_start: now - chrono::Duration::milliseconds(1),
                window_end: now,
                partial: false,
                probe: Some(ProbeObservation {
                    channels: None,
                    measurement_id: event,
                    kind: ProbeKind::Path,
                    link_id: None,
                    transport: None,
                    path,
                    publication_id: running.configuration.execution_publication,
                    started_at: now - chrono::Duration::milliseconds(1),
                    completed_at: Some(now),
                    outcome: ProbeOutcome::Success,
                    duration_us: Some(Sequence::new(1000).unwrap()),
                }),
                traffic: None,
                probe_counts: None,
            }),
        },
    }
}

async fn fresh_sample(
    server: &Server,
    user: &meshway_protocol::SessionIssuedResponse,
    endpoint: &str,
    nodes: [&Node; 3],
    mut tabs: [&mut ObservationTask; 2],
    segment: &mut Sequence,
) -> ObservationResult {
    // Each assertion phase receives a NEW measurement after real Console/Node
    // renewal. Never relabel an earlier measurement with a successor binding.
    for node in nodes.into_iter().rev() {
        for tab in &mut tabs {
            **tab = keep_alive(server, user, endpoint, (**tab).clone()).await;
        }
        server.target(node).await;
    }
    let tasks: ObservationList = json(
        server.get("/agent-api/v1/tasks", nodes[0].token.expose()),
        200,
    )
    .await;
    assert_eq!(tasks.items.len(), 1);
    // The producer owns its monotonic segment across publication resets, even
    // when the Server's Preparing projection no longer exposes the old segment.
    *segment = segment.checked_next().unwrap();
    sample_result(
        &tasks.items[0],
        nodes[0],
        nodes.map(|node| node.id).to_vec(),
        *segment,
        chrono::Utc::now(),
        id(),
    )
}

#[tokio::test]
async fn idle_observation_compiles_narrow_path_shares_tabs_and_accepts_real_source_events() {
    let server = Server::new().await;
    let user = server.user("owner@example.com").await;
    let network = server.network(&user).await;
    let a = server.enroll(&user, network.id, "a").await;
    let b = server.enroll(&user, network.id, "b").await;
    let c = server.enroll(&user, network.id, "c").await;
    server
        .configure(&user, network.id, |cfg| {
            for node in cfg.nodes.values_mut() {
                node.listeners = serde_json::from_value(
                    serde_json::json!({"udp":{"type":"native_udp","listenPort":23456}}),
                )
                .unwrap();
            }
            cfg.access = vec![meshway_configuration::AccessRule {
                remark: String::new(),
                src: vec![format!("node:{}", a.id)],
                dst: vec![format!("node:{}", b.id)],
                ip: vec!["tcp:443".into()],
                bidirectional: false,
            }];
            cfg.routing.mesh = vec![meshway_configuration::MeshRoute {
                remark: String::new(),
                src: vec![format!("node:{}", a.id)],
                dst: vec![format!("node:{}", b.id)],
                via: vec![format!("node:{}", c.id)],
            }];
        })
        .await;
    let endpoint = format!("/api/v1/networks/{}/observations", network.id);
    // Denied authorization setup does not need the shared subscriptions.
    // Finish it before starting their OBSERVATION_LEASE_SECONDS clocks.
    let mut forbidden = request(&a, &b);
    forbidden.service = ObservationServiceScope::Tcp {
        port: 80.try_into().unwrap(),
    };
    status(server.post(&endpoint, user.token.expose(), &forbidden), 403).await;
    forbidden = request(&a, &b);
    forbidden.selection = ObservationPath::Explicit {
        path: vec![a.id, b.id],
    };
    status(server.post(&endpoint, user.token.expose(), &forbidden), 403).await;
    let first = request(&a, &b);
    let mut task: ObservationTask =
        json(server.post(&endpoint, user.token.expose(), &first), 201).await;
    assert_eq!(task.phase, ObservationPhase::Preparing);
    assert!(task.path.is_none());
    assert!(task.latest.is_none());
    assert_eq!(task.unsupported_metrics, vec![MetricKind::Traffic]);
    // Idempotency is checked next to creation, before any legitimate renewal.
    let retry: ObservationTask =
        json(server.post(&endpoint, user.token.expose(), &first), 201).await;
    assert_eq!(retry.id, task.id);
    assert_eq!(retry.expires_at, task.expires_at);
    assert_eq!(retry.configuration, task.configuration);
    task = keep_alive(&server, &user, &endpoint, task).await;
    let source = slice(server.target(&a).await);
    assert!(source.permits.is_empty());
    assert!(source.decisions.is_empty());
    assert_eq!(
        source.diagnostics[&task.id].paths[0].nodes,
        vec![a.id, c.id, b.id]
    );
    assert_eq!(
        slice(server.target(&c).await).diagnostics[&task.id].paths[0].nodes,
        vec![a.id, c.id, b.id]
    );
    task = keep_alive(&server, &user, &endpoint, task).await;
    let second = request(&a, &b);
    let mut shared: ObservationTask =
        json(server.post(&endpoint, user.token.expose(), &second), 201).await;
    assert_eq!(shared.id, task.id);
    assert_ne!(shared.subscription_id, task.subscription_id);
    let listing: ObservationList = json(server.get(&endpoint, user.token.expose()), 200).await;
    assert_eq!(listing.items.len(), 2);
    task = keep_alive(&server, &user, &endpoint, task).await;
    let mut explicit = request(&a, &b);
    explicit.selection = ObservationPath::Explicit {
        path: vec![a.id, c.id, b.id],
    };
    let exact: ObservationTask =
        json(server.post(&endpoint, user.token.expose(), &explicit), 201).await;
    status(
        server.post(
            &format!("{endpoint}/{}/cancel", exact.subscription_id.unwrap()),
            user.token.expose(),
            &(),
        ),
        204,
    )
    .await;
    let mut events = Events::open(&server, network.id, user.token.expose()).await;
    loop {
        if let meshway_protocol::StreamEvent::ObservationStatus { observation } =
            events.next().await.0
            && observation.subscription_id == Some(first.subscription_id)
        {
            break;
        }
    }
    // SSE setup also belongs before the last renewal/configuration fetch.
    let mut segment = Sequence::MIN;
    let result = fresh_sample(
        &server,
        &user,
        &endpoint,
        [&a, &c, &b],
        [&mut task, &mut shared],
        &mut segment,
    )
    .await;
    let ObservationUpdate::Sample { sample, .. } = &result.result else {
        unreachable!()
    };
    let (now, event) = (sample.window_end, sample.measurement_id);
    let result_path = format!("/agent-api/v1/tasks/{}/results", task.id);
    status(server.post(&result_path, b.token.expose(), &result), 401).await;
    let accepted: ObservationTask =
        json(server.post(&result_path, a.token.expose(), &result), 200).await;
    assert_eq!(accepted.phase, ObservationPhase::Running);
    let _: ObservationTask = json(server.post(&result_path, a.token.expose(), &result), 200).await;
    let mut delivered = std::collections::BTreeSet::new();
    while delivered.len() < 2 {
        if let meshway_protocol::StreamEvent::ObservationSample { observation } =
            events.next().await.0
        {
            assert_eq!(observation.id, task.id);
            assert_eq!(observation.latest.unwrap().measurement_id, event);
            delivered.insert(observation.subscription_id.unwrap());
        }
    }
    assert_eq!(
        delivered,
        [first.subscription_id, second.subscription_id]
            .into_iter()
            .collect()
    );
    drop(events);
    assert!(accepted.latest.as_ref().unwrap().traffic.is_none());
    let mut wrong_instance = fresh_sample(
        &server,
        &user,
        &endpoint,
        [&a, &c, &b],
        [&mut task, &mut shared],
        &mut segment,
    )
    .await;
    wrong_instance.instance_id = id();
    status(
        server.post(&result_path, a.token.expose(), &wrong_instance),
        401,
    )
    .await;
    let mut bypass = fresh_sample(
        &server,
        &user,
        &endpoint,
        [&a, &c, &b],
        [&mut task, &mut shared],
        &mut segment,
    )
    .await;
    if let ObservationUpdate::Sample { path, .. } = &mut bypass.result {
        *path = vec![a.id, b.id];
    }
    status(server.post(&result_path, a.token.expose(), &bypass), 400).await;
    let fresh = fresh_sample(
        &server,
        &user,
        &endpoint,
        [&a, &c, &b],
        [&mut task, &mut shared],
        &mut segment,
    )
    .await;
    let _: ObservationTask = json(server.post(&result_path, a.token.expose(), &fresh), 200).await;
    let mut rewritten = fresh.clone();
    if let ObservationUpdate::Sample { sample, .. } = &mut rewritten.result {
        sample.version = Version::new(2).unwrap();
    }
    status(server.post(&result_path, a.token.expose(), &rewritten), 409).await;
    let mut before_task = fresh;
    if let ObservationUpdate::Sample { sample, .. } = &mut before_task.result {
        sample.window_start = now - chrono::Duration::minutes(1);
    }
    status(
        server.post(&result_path, a.token.expose(), &before_task),
        400,
    )
    .await;
    // All old-configuration reports are finished. Renew both tabs before
    // history I/O, then renew the survivor before testing cancellation isolation.
    let _ = keep_alive(&server, &user, &endpoint, task).await;
    shared = keep_alive(&server, &user, &endpoint, shared).await;
    let history: HistoryPage = json(
        server.post(
            &format!("/api/v1/networks/{}/history/query", network.id),
            user.token.expose(),
            &HistoryQuery {
                from: now - chrono::Duration::minutes(1),
                to: chrono::Utc::now(),
                source_node_id: Some(a.id),
                peer_node_id: None,
                mode: HistoryMode::Raw,
                bucket_seconds: None,
                cursor: None,
                limit: 100,
                kinds: None,
                link_id: None,
            },
        ),
        200,
    )
    .await;
    let HistoryItems::Raw { items } = history.result else {
        panic!()
    };
    // Two genuine samples were accepted, with distinct measurement IDs. None
    // of the duplicate or invalid reports may manufacture a history row.
    assert_eq!(items.len(), 2);
    shared = keep_alive(&server, &user, &endpoint, shared).await;
    status(
        server.post(
            &format!("{endpoint}/{}/cancel", first.subscription_id),
            user.token.expose(),
            &(),
        ),
        204,
    )
    .await;
    keep_alive(&server, &user, &endpoint, shared).await;
    server.target(&a).await;
    let tasks: ObservationList =
        json(server.get("/agent-api/v1/tasks", a.token.expose()), 200).await;
    assert_eq!(tasks.items.len(), 1);
    status(
        server.post(
            &format!("{endpoint}/{}/cancel", second.subscription_id),
            user.token.expose(),
            &(),
        ),
        204,
    )
    .await;
    assert!(slice(server.target(&a).await).diagnostics.is_empty());
    let tasks: ObservationList =
        json(server.get("/agent-api/v1/tasks", a.token.expose()), 200).await;
    assert!(tasks.items.is_empty());
    status(server.post(&result_path, a.token.expose(), &result), 401).await;
    server
        .configure(&user, network.id, |cfg| {
            cfg.topology.push(meshway_configuration::TopologyRule {
                transports: meshway_configuration::Input::Omitted,
                remark: String::new(),
                action: meshway_configuration::TopologyAction::Block,
                a: format!("node:{}", c.id),
                b: format!("node:{}", b.id),
            });
        })
        .await;
    status(
        server.post(&endpoint, user.token.expose(), &request(&a, &b)),
        403,
    )
    .await;
    server.stop().await;
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
