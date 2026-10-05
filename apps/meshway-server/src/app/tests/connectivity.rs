//! Registered identities and real compiled publications. Reported documentation
//! addresses exercise the approved self-report contract, not packet reachability.
use super::{
    App,
    node::{caps, fixture, session},
};
use crate::{accounts::random_uuid, state::service::ReadIdentity};
use meshway_configuration::execution::{
    BusinessProtocol, BusinessTarget, Carrier, CompiledNode, DemandDecision,
};
use meshway_model::{
    ConfigurationRef, InstanceId, NetworkId, NodeId, RequestId, Sequence, StateEpoch, Version,
    state::*,
};
use meshway_protocol::{
    connectivity::*,
    execution::*,
    state::{StateReport, VisibleReport},
};
use std::collections::BTreeMap;

fn rid() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}

struct Node {
    id: NodeId,
    token: String,
    instance: InstanceRequest,
    target: CompiledNode,
    report: StateReport,
}
impl Node {
    fn configuration(&self) -> ConfigurationRef {
        ConfigurationRef {
            revision: self.target.configuration.revision,
            execution_publication: self.target.configuration.publication,
            digest: self.target.digest,
        }
    }
    fn query(&self) -> QueryUdpTraversalRequest {
        QueryUdpTraversalRequest {
            instance: self.instance.clone(),
            configuration: self.configuration(),
        }
    }
    fn candidate(&self) -> CandidateReference {
        let socket = &self.report.state.sockets["socket"];
        CandidateReference::from_observation(
            SocketReference {
                node_id: self.id,
                instance: self.instance.clone(),
                configuration: self.configuration(),
                socket_id: "socket".into(),
                socket_generation: socket.generation,
                resource: socket.resource.clone(),
            },
            socket.network,
            socket.bound_address,
            &socket.endpoints[0],
        )
        .unwrap()
    }
    fn cas(&self, offer: &UdpTraversalOffer) -> UdpTraversalAttemptRequest {
        UdpTraversalAttemptRequest {
            instance: self.instance.clone(),
            configuration: self.configuration(),
            state_epoch: offer.state_epoch,
            version: offer.version,
            digest: offer.digest,
        }
    }
    async fn publish(&mut self, app: &App) {
        self.report.sequence = self.report.sequence.checked_next().unwrap();
        app.node_state_service()
            .unwrap()
            .accept_report(self.token.clone(), self.report.clone())
            .await
            .unwrap();
    }
}

fn report(target: &CompiledNode, instance: &InstanceRequest, address: &str) -> StateReport {
    let now = chrono::Utc::now();
    let reference = ConfigurationRef {
        revision: target.configuration.revision,
        execution_publication: target.configuration.publication,
        digest: target.digest,
    };
    StateReport {
        instance_id: instance.instance_id,
        sequence: Sequence::MIN,
        observed_at: now,
        state: NodeState {
            system: SystemState {
                hostname: "reported-fact".into(),
                os: "linux".into(),
                arch: "aarch64".into(),
                agent_version: "test".into(),
            },
            capabilities: target.configuration.self_keys.capabilities.clone(),
            configuration: ConfigurationState {
                saved: Some(reference.clone()),
                applied: Some(reference.clone()),
                progress: None,
            },
            listeners: BTreeMap::from([(
                "udp".into(),
                ListenerState {
                    phase: ListenerPhase::Listening,
                    socket_ids: vec!["socket".into()],
                    loaded_certificate_sha256: None,
                    problem: None,
                    configuration: Some(reference.clone()),
                },
            )]),
            sockets: BTreeMap::from([(
                "socket".into(),
                SocketState {
                    resource: ResourceRef::Listener { key: "udp".into() },
                    configuration: Some(reference),
                    generation: Sequence::new(1).unwrap(),
                    network: SocketNetwork::Udp,
                    bound_address: "0.0.0.0:23456".parse().unwrap(),
                    endpoints: vec![EndpointCandidate {
                        address: address.into(),
                        origin: EndpointSource::Stun {
                            observer: "203.0.113.53:3478".into(),
                        },
                        observed_at: now,
                        expires_at: now + chrono::Duration::seconds(30),
                    }],
                },
            )]),
            links: BTreeMap::new(),
            paths: BTreeMap::new(),
            selection: SelectionState {
                applied_profile: Some(target.configuration.dns.profile.clone()),
            },
            probes: vec![],
            problems: vec![],
        },
    }
}

async fn sync_execution(app: &App, token: &str, instance: &InstanceRequest) -> CompiledNode {
    let response = app
        .execution()
        .refresh_configuration(
            token,
            NodeConfigurationRequest {
                instance: instance.clone(),
                base: None,
            },
        )
        .await
        .unwrap();
    let target = response.full.unwrap();
    for (sequence, phase) in [(1, ReceiptPhase::Saved), (2, ReceiptPhase::Applied)] {
        app.execution()
            .receipt(
                token,
                ReceiptRequest {
                    instance: instance.clone(),
                    target: response.lease.target.clone().unwrap(),
                    phase,
                    attempt: Version::MIN,
                    sequence: Sequence::new(sequence).unwrap(),
                    reported_at_ms: chrono::Utc::now().timestamp_millis() as u64,
                    failure: None,
                },
            )
            .await
            .unwrap();
    }
    target
}

async fn sync(
    app: &App,
    id: NodeId,
    token: String,
    instance: InstanceRequest,
    address: &str,
) -> Node {
    let target = sync_execution(app, &token, &instance).await;
    let previous = app
        .node_state_service()
        .unwrap()
        .query(
            ReadIdentity::Node {
                authorization: token.clone(),
            },
            None,
        )
        .await
        .unwrap();
    let mut report = report(&target, &instance, address);
    report.sequence = previous
        .nodes
        .get(&id)
        .and_then(|node| node.report_sequence)
        .unwrap_or(Sequence::MIN);
    let mut node = Node {
        id,
        token,
        instance,
        target,
        report,
    };
    node.publish(app).await;
    node
}

async fn setup(business: bool) -> (tempfile::TempDir, App, NetworkId, Node, Node, RequestId) {
    let (dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let at = session(&app.execution(), network, a, &ak, &signer_a).await;
    let bt = session(&app.execution(), network, b, &bk, &signer_b).await;
    let mut instances = Vec::new();
    for token in [at.token.expose(), bt.token.expose()] {
        let lease = app
            .execution()
            .acquire(
                token,
                AcquireInstanceRequest {
                    instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                    capabilities: caps(),
                },
            )
            .await
            .unwrap();
        instances.push(InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        });
    }
    let demand = rid();
    if business {
        let response = app
            .execution()
            .demand(
                at.token.expose(),
                DemandRequest {
                    instance: instances[0].clone(),
                    request_id: demand,
                    target: BusinessTarget::Mesh { node: b },
                    protocol: BusinessProtocol::Udp,
                    port: Some(53.try_into().unwrap()),
                    profile: "mesh".into(),
                    duration_seconds: 60,
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            response.decision,
            DemandDecision::Permitted { .. }
        ));
    }
    let a = sync(
        &app,
        a,
        at.token.expose().into(),
        instances[0].clone(),
        "198.51.100.1:23456",
    )
    .await;
    let b = sync(
        &app,
        b,
        bt.token.expose().into(),
        instances[1].clone(),
        "198.51.100.2:23456",
    )
    .await;
    (dir, app, network, a, b, demand)
}

async fn request(app: &App, a: &Node, b: &Node) -> PrepareUdpTraversalRequest {
    let QueryUdpTraversalResponse::Current { state_epoch, .. } =
        app.connectivity().query(&a.token, a.query()).await.unwrap()
    else {
        panic!("current metadata");
    };
    request_in_epoch(a, b, state_epoch)
}

fn request_in_epoch(a: &Node, b: &Node, state_epoch: StateEpoch) -> PrepareUdpTraversalRequest {
    PrepareUdpTraversalRequest {
        instance: a.instance.clone(),
        configuration: a.configuration(),
        state_epoch,
        request_id: rid(),
        expires_at_ms: chrono::Utc::now().timestamp_millis() as u64 + 9_000,
        peer_id: b.id,
        carrier: Carrier {
            dialer: a.id,
            listener: b.id,
            listener_key: "udp".into(),
            protocol: meshway_transport_spec::TransportProtocol::NativeUdp,
        },
        local_candidate: a.candidate(),
        peer_candidate: b.candidate(),
    }
}

async fn offer(app: &App, a: &Node, request: PrepareUdpTraversalRequest) -> UdpTraversalOffer {
    let result = app.connectivity().prepare(&a.token, request).await.unwrap();
    let PrepareUdpTraversalResponse::Offer { offer } = result else {
        panic!("expected offer, got {result:?}");
    };
    assert_eq!(offer.compute_digest().unwrap(), offer.digest);
    offer
}

#[tokio::test]
async fn bilateral_readiness_replay_identity_and_cancel_are_real_current_authority() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, mut a, b, _) = setup(true).await;
    assert!(app.stun_ready());
    let QueryUdpTraversalResponse::Current {
        stun_servers,
        attempts,
        ..
    } = app.connectivity().query(&a.token, a.query()).await.unwrap()
    else {
        panic!();
    };
    assert!(stun_servers.as_slice().is_empty());
    assert!(attempts.as_slice().is_empty());
    let view = app
        .node_state_service()
        .unwrap()
        .query(
            ReadIdentity::Node {
                authorization: a.token.clone(),
            },
            None,
        )
        .await
        .unwrap();
    let Some(VisibleReport::Peer { report }) = &view.nodes[&b.id].report else {
        panic!();
    };
    assert_eq!(report.endpoints[0].candidate_reference, b.candidate());
    assert!(
        !serde_json::to_string(report)
            .unwrap()
            .contains("203.0.113.53")
    );
    let request = request(&app, &a, &b).await;
    let first = offer(&app, &a, request.clone()).await;
    assert_eq!(offer(&app, &a, request.clone()).await, first);
    let mut changed = request.clone();
    changed.expires_at_ms -= 1;
    assert_eq!(
        app.connectivity().prepare(&a.token, changed).await.unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::RequestChanged
        }
    );
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Waiting { .. }
    ));
    let QueryUdpTraversalResponse::Current { attempts, .. } =
        app.connectivity().query(&b.token, b.query()).await.unwrap()
    else {
        panic!();
    };
    assert_eq!(attempts.as_slice()[0].phase, UdpTraversalPhase::Waiting);
    // A new telemetry sequence, with the original candidate, is not a new mapping.
    a.report.state.system.hostname = "telemetry-update".into();
    a.publish(&app).await;
    assert!(matches!(
        app.connectivity()
            .accept(&b.token, first.attempt_id, b.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Active { .. }
    ));
    assert_eq!(offer(&app, &a, request.clone()).await, first);
    let QueryUdpTraversalResponse::Current { attempts, .. } =
        app.connectivity().query(&a.token, a.query()).await.unwrap()
    else {
        panic!();
    };
    assert_eq!(attempts.as_slice()[0].phase, UdpTraversalPhase::Active);
    for _ in 0..2 {
        assert_eq!(
            app.connectivity()
                .cancel(&b.token, first.attempt_id, b.cas(&first))
                .await
                .unwrap(),
            CancelUdpTraversalResponse::Cancelled
        );
    }
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable { .. }
    ));
    assert!(matches!(
        app.connectivity().prepare(&a.token, request).await.unwrap(),
        PrepareUdpTraversalResponse::Unavailable { .. }
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn candidate_cas_physical_resource_and_scope_races_are_closed_outcomes() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, mut a, b, _) = setup(true).await;
    let original = request(&app, &a, &b).await;
    let encoded = serde_json::to_value(&original).unwrap();
    for (pointer, value) in [
        (
            "/localCandidate/observationDigest",
            serde_json::json!("ff".repeat(32)),
        ),
        (
            "/localCandidate/socket/socketGeneration",
            serde_json::json!(2),
        ),
        (
            "/peerCandidate/socket/instance/generation",
            serde_json::json!(2),
        ),
        (
            "/peerCandidate/socket/resource/key",
            serde_json::json!("unrelated-listener"),
        ),
        (
            "/carrier/listenerKey",
            serde_json::json!("unrelated-listener"),
        ),
        ("/instance/generation", serde_json::json!(2)),
        ("/configuration/digest", serde_json::json!("fe".repeat(32))),
    ] {
        let mut value_json = encoded.clone();
        *value_json.pointer_mut(pointer).unwrap() = value;
        let bad: PrepareUdpTraversalRequest = serde_json::from_value(value_json).unwrap();
        assert!(
            matches!(
                app.connectivity().prepare(&a.token, bad).await.unwrap(),
                PrepareUdpTraversalResponse::Unavailable { .. }
            ),
            "{pointer}"
        );
    }
    let first = offer(&app, &a, original.clone()).await;
    let mut bad_cas = a.cas(&first);
    bad_cas.digest = [99; 32].into();
    assert_eq!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, bad_cas)
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::AttemptChanged
        }
    );
    let mut another = original.clone();
    another.request_id = rid();
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, another.clone())
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::Capacity
        }
    );
    // Generation changes preserve the report's other facts but invalidate the offer.
    a.report.state.sockets.get_mut("socket").unwrap().generation = Sequence::new(2).unwrap();
    a.publish(&app).await;
    assert_eq!(
        app.connectivity()
            .accept(&b.token, first.attempt_id, b.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::CandidateChanged
        }
    );
    let QueryUdpTraversalResponse::Current { attempts, .. } =
        app.connectivity().query(&b.token, b.query()).await.unwrap()
    else {
        panic!();
    };
    assert!(attempts.as_slice().is_empty());
    another.local_candidate = a.candidate();
    let next = offer(&app, &a, another).await;
    assert_ne!(next.nonce, first.nonce);
    assert_ne!(next.attempt_id, first.attempt_id);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn withdrawal_between_offer_and_accept_never_installs_an_active_grant() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b, demand) = setup(true).await;
    let first = offer(&app, &a, request(&app, &a, &b).await).await;
    app.connectivity()
        .accept(&a.token, first.attempt_id, a.cas(&first))
        .await
        .unwrap();
    app.execution()
        .cancel_demand(
            &a.token,
            CancelDemandRequest {
                instance: a.instance.clone(),
                request_id: demand,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        app.connectivity()
            .accept(&b.token, first.attempt_id, b.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable { .. }
    ));
    assert!(matches!(
        app.connectivity().query(&a.token, a.query()).await.unwrap(),
        QueryUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::ScopeChanged
        }
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn metadata_is_available_without_business_but_candidate_reports_do_not_grant_punching() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b, _) = setup(false).await;
    let request = request(&app, &a, &b).await;
    assert_eq!(
        app.connectivity().prepare(&a.token, request).await.unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::NotAvailable
        }
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn actual_http_operations_use_node_credentials_and_return_success_for_expected_races() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b, _) = setup(true).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let router = crate::http::router(&app);
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let client = reqwest::Client::new();
    let url = format!("http://{address}/agent-api/v1/connectivity/udp");
    let mut stale = a.query();
    stale.instance.generation = Version::new(2).unwrap();
    let response = client
        .post(format!("{url}/query"))
        .bearer_auth(&a.token)
        .json(&stale)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.json::<QueryUdpTraversalResponse>().await.unwrap(),
        QueryUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::ScopeChanged
        }
    );
    let req = request(&app, &a, &b).await;
    let response = client
        .post(format!("{url}/attempts"))
        .bearer_auth(&a.token)
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let PrepareUdpTraversalResponse::Offer { offer } = response.json().await.unwrap() else {
        panic!();
    };
    let response = client
        .post(format!("{url}/attempts/{}/accept", offer.attempt_id))
        .bearer_auth(&a.token)
        .json(&a.cas(&offer))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(matches!(
        response.json::<AcceptUdpTraversalResponse>().await.unwrap(),
        AcceptUdpTraversalResponse::Waiting { .. }
    ));
    let response = client
        .post(format!("{url}/attempts/{}/cancel", offer.attempt_id))
        .bearer_auth(&b.token)
        .json(&b.cas(&offer))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.json::<CancelUdpTraversalResponse>().await.unwrap(),
        CancelUdpTraversalResponse::Cancelled
    );
    let response = client
        .post(format!("{url}/query"))
        .bearer_auth("invalid")
        .json(&a.query())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    app.stream_shutdown()();
    stop.send(()).unwrap();
    server.await.unwrap();
    app.shutdown().await.unwrap();
}

#[test]
fn openapi_registers_the_actual_closed_operations_with_node_security() {
    let spec = serde_json::to_value(crate::http::openapi()).unwrap();
    for (suffix, operation) in [
        ("/query", "queryUdpTraversal"),
        ("/attempts", "prepareUdpTraversal"),
        ("/attempts/{attemptId}/accept", "acceptUdpTraversal"),
        ("/attempts/{attemptId}/cancel", "cancelUdpTraversal"),
    ] {
        let route = &spec["paths"][format!("/agent-api/v1/connectivity/udp{suffix}")]["post"];
        assert_eq!(route["operationId"], operation);
        assert_eq!(route["security"], serde_json::json!([{"nodeSession":[]}]));
        assert!(route["responses"].get("409").is_none());
        assert!(route["responses"].get("404").is_none());
    }
}

#[tokio::test]
async fn original_absolute_expiry_and_gc_never_issue_a_replacement_nonce() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b, _) = setup(true).await;
    let mut req = request(&app, &a, &b).await;
    req.expires_at_ms = chrono::Utc::now().timestamp_millis() as u64 - 1;
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, req.clone())
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::AttemptExpired
        }
    );
    req.expires_at_ms = chrono::Utc::now().timestamp_millis() as u64 + 60_000;
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, req.clone())
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::NotAvailable
        }
    );
    req.expires_at_ms = chrono::Utc::now().timestamp_millis() as u64 + 500;
    let first = offer(&app, &a, req.clone()).await;
    tokio::time::sleep(std::time::Duration::from_millis(550)).await;
    let QueryUdpTraversalResponse::Current { attempts, .. } =
        app.connectivity().query(&a.token, a.query()).await.unwrap()
    else {
        panic!();
    };
    assert!(attempts.as_slice().is_empty());
    assert_eq!(
        app.connectivity().prepare(&a.token, req).await.unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::AttemptExpired
        }
    );
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable { .. }
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn final_peer_session_deadline_recheck_is_terminal_even_if_storage_later_recovers() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b, _) = setup(true).await;
    let first = offer(&app, &a, request(&app, &a, &b).await).await;
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Waiting { .. }
    ));
    // The SQL owner shortens the real peer-session deadline after prepare.
    let node = b.id;
    let old = app
        .executor
        .handle()
        .execute(move |connection| {
            let old: i64 = connection
                .query_row(
                    "SELECT max(expires_at) FROM node_sessions WHERE node_id=?1",
                    [node.as_uuid().as_bytes().as_slice()],
                    |row| row.get(0),
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE node_sessions SET expires_at=?2 WHERE node_id=?1",
                    rusqlite::params![
                        node.as_uuid().as_bytes().as_slice(),
                        chrono::Utc::now().timestamp_millis() + 100
                    ],
                )
                .unwrap();
            old
        })
        .await
        .unwrap();
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable { .. }
    ));
    app.executor
        .handle()
        .execute(move |connection| {
            connection
                .execute(
                    "UPDATE node_sessions SET expires_at=?2 WHERE node_id=?1",
                    rusqlite::params![node.as_uuid().as_bytes().as_slice(), old],
                )
                .unwrap();
        })
        .await
        .unwrap();
    assert!(matches!(
        app.connectivity()
            .accept(&b.token, first.attempt_id, b.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable { .. }
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn diagnostic_only_publication_discloses_no_automatic_endpoint_or_punch_offer() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, _) = setup(false).await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            crate::accounts::Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    app.observations()
        .create_observation(
            ReadIdentity::user_session(owner.token.expose(), network).unwrap(),
            meshway_protocol::observations::CreateObservation {
                service: meshway_configuration::execution::ObservationServiceScope::Tcp {
                    port: 443.try_into().unwrap(),
                },
                subscription_id: meshway_model::ObservationSubscriptionId::new(
                    random_uuid().unwrap(),
                )
                .unwrap(),
                source_node_id: a.id,
                target_node_id: b.id,
                selection: meshway_protocol::observations::ObservationPath::Explicit {
                    path: vec![a.id, b.id],
                },
                metrics: vec![meshway_model::observations::MetricKind::Rtt],
                interval_ms: 1000,
            },
        )
        .await
        .unwrap();
    let a = sync(&app, a.id, a.token, a.instance, "198.51.100.1:23456").await;
    let b = sync(&app, b.id, b.token, b.instance, "198.51.100.2:23456").await;
    assert!(a.target.configuration.permits.is_empty());
    assert!(!a.target.configuration.diagnostics.is_empty());
    let view = app
        .node_state_service()
        .unwrap()
        .query(
            ReadIdentity::Node {
                authorization: a.token.clone(),
            },
            None,
        )
        .await
        .unwrap();
    let Some(VisibleReport::Peer { report }) = &view.nodes[&b.id].report else {
        panic!();
    };
    assert!(report.endpoints.is_empty());
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, request(&app, &a, &b).await)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::NotAvailable
        }
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn complete_logical_path_visibility_does_not_disclose_nonadjacent_endpoints() {
    use meshway_configuration::{TopologyAction, TopologyDefault, TopologyRule};
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, _) = setup(false).await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            crate::accounts::Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let signer = ed25519_dalek::SigningKey::from_bytes(&[77; 32]);
    let relay = super::node::enroll_tag_node(
        &app,
        owner.token.expose(),
        network,
        &signer,
        "relay",
        [78; 32],
    )
    .await;
    let rt = session(
        &app.execution(),
        network,
        relay.node_id,
        &relay.keys,
        &signer,
    )
    .await;
    let lease = app
        .execution()
        .acquire(
            rt.token.expose(),
            AcquireInstanceRequest {
                instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let mut cfg = current.document.configuration.clone();
    cfg.defaults.topology = TopologyDefault::Deny;
    cfg.topology = [(a.id, relay.node_id), (relay.node_id, b.id)]
        .into_iter()
        .map(|(a, b)| TopologyRule {
            remark: String::new(),
            action: TopologyAction::Allow,
            a: format!("node:{a}"),
            b: format!("node:{b}"),
        })
        .collect();
    cfg.routing.mesh = vec![meshway_configuration::MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{}", a.id)],
        dst: vec![format!("node:{}", b.id)],
        via: vec![format!("node:{}", relay.node_id)],
    }];
    cfg.access = vec![meshway_configuration::AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", a.id)],
        dst: vec![format!("node:{}", b.id)],
        ip: vec!["udp:53".into()],
        bidirectional: false,
    }];
    // Apply through the real configuration owner, not a fabricated execution DTO.
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: meshway_protocol::WireSecret::new(
                    meshway_configuration::canonical_json(&cfg)
                        .unwrap()
                        .expose()
                        .into(),
                ),
            },
            rid(),
        )
        .await
        .unwrap();
    let response = app
        .execution()
        .demand(
            &a.token,
            DemandRequest {
                instance: a.instance.clone(),
                request_id: rid(),
                target: BusinessTarget::Mesh { node: b.id },
                protocol: BusinessProtocol::Udp,
                port: Some(53.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 60,
            },
        )
        .await
        .unwrap();
    let DemandDecision::Permitted { permit } = response.decision else {
        panic!();
    };
    assert!(
        permit
            .paths
            .iter()
            .all(|path| path.nodes == vec![a.id, relay.node_id, b.id])
    );
    let a = sync(&app, a.id, a.token, a.instance, "198.51.100.1:23456").await;
    let b = sync(&app, b.id, b.token, b.instance, "198.51.100.2:23456").await;
    let relay = sync(
        &app,
        relay.node_id,
        rt.token.expose().into(),
        InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        },
        "198.51.100.3:23456",
    )
    .await;
    let view = app
        .node_state_service()
        .unwrap()
        .query(
            ReadIdentity::Node {
                authorization: a.token.clone(),
            },
            None,
        )
        .await
        .unwrap();
    for (peer, count) in [(&b, 0), (&relay, 1)] {
        let Some(VisibleReport::Peer { report }) = &view.nodes[&peer.id].report else {
            panic!();
        };
        assert_eq!(report.endpoints.len(), count);
    }
    assert!(matches!(
        app.connectivity()
            .prepare(&a.token, request(&app, &a, &b).await)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable { .. }
    ));
    let _adjacent = offer(&app, &a, request(&app, &a, &relay).await).await;
    let full = app
        .user_state_service()
        .unwrap()
        .query(
            ReadIdentity::user_session(owner.token.expose(), network).unwrap(),
            None,
        )
        .await
        .unwrap();
    assert!(
        matches!(&full.nodes[&b.id].report, Some(VisibleReport::Full { report }) if !report.sockets.is_empty())
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn dialer_can_use_a_different_owned_listener_but_listener_side_is_exact() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, _) = setup(true).await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            crate::accounts::Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let mut cfg = current.document.configuration.clone();
    for node in [a.id, b.id] {
        cfg.nodes.get_mut(&node).unwrap().listeners.insert(
            "local-source".into(),
            serde_json::from_value(serde_json::json!({"type":"native_udp","listenPort":23457}))
                .unwrap(),
        );
    }
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: meshway_protocol::WireSecret::new(
                    meshway_configuration::canonical_json(&cfg)
                        .unwrap()
                        .expose()
                        .into(),
                ),
            },
            rid(),
        )
        .await
        .unwrap();
    let mut a = sync(&app, a.id, a.token, a.instance, "198.51.100.1:23456").await;
    let mut b = sync(&app, b.id, b.token, b.instance, "198.51.100.2:23456").await;
    let use_other_listener = |node: &mut Node| {
        let listener = node.report.state.listeners.remove("udp").unwrap();
        node.report
            .state
            .listeners
            .insert("local-source".into(), listener);
        let socket = node.report.state.sockets.get_mut("socket").unwrap();
        socket.resource = ResourceRef::Listener {
            key: "local-source".into(),
        };
        socket.bound_address.set_port(23457);
        let mut address: std::net::SocketAddr = socket.endpoints[0].address.parse().unwrap();
        address.set_port(23457);
        socket.endpoints[0].address = address.to_string();
    };
    use_other_listener(&mut a);
    a.publish(&app).await;
    let req = request(&app, &a, &b).await;
    assert_eq!(req.carrier.listener_key, "udp");
    let first = offer(&app, &a, req).await;
    app.connectivity()
        .cancel(&a.token, first.attempt_id, a.cas(&first))
        .await
        .unwrap();
    use_other_listener(&mut b);
    b.publish(&app).await;
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, request(&app, &a, &b).await)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::NotAvailable
        }
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn real_restart_changes_epoch_and_old_credentials_and_attempts_cannot_reappear() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (dir, app, network, a, b, _) = setup(true).await;
    let req = request(&app, &a, &b).await;
    let old = offer(&app, &a, req.clone()).await;
    let keys = meshway_protocol::node::NodeKeyMetadata {
        control: a.target.configuration.self_keys.control_key.clone(),
        dataplane: a.target.configuration.self_keys.dataplane_key.clone(),
    };
    app.shutdown().await.unwrap();
    let app = App::open(crate::Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    assert!(matches!(
        app.connectivity().query(&a.token, a.query()).await,
        Err(crate::error::Error::InvalidCredentials)
    ));
    let signer = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    let token = session(&app.execution(), network, a.id, &keys, &signer).await;
    let lease = app
        .execution()
        .acquire(
            token.token.expose(),
            AcquireInstanceRequest {
                instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let instance = InstanceRequest {
        instance_id: lease.instance_id,
        generation: lease.generation,
    };
    let configuration = lease.target.unwrap();
    let QueryUdpTraversalResponse::Current {
        state_epoch,
        attempts,
        ..
    } = app
        .connectivity()
        .query(
            token.token.expose(),
            QueryUdpTraversalRequest {
                instance: instance.clone(),
                configuration: configuration.clone(),
            },
        )
        .await
        .unwrap()
    else {
        panic!();
    };
    assert_ne!(state_epoch, old.state_epoch);
    assert!(attempts.as_slice().is_empty());
    let mut req = req;
    req.instance = instance.clone();
    req.configuration = configuration.clone();
    assert_eq!(
        app.connectivity()
            .prepare(token.token.expose(), req)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::ScopeChanged
        }
    );
    assert_eq!(
        app.connectivity()
            .accept(
                token.token.expose(),
                old.attempt_id,
                UdpTraversalAttemptRequest {
                    instance,
                    configuration,
                    state_epoch: old.state_epoch,
                    version: old.version,
                    digest: old.digest
                }
            )
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::ScopeChanged
        }
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn per_node_slots_are_bounded_across_distinct_authenticated_peer_pairs() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, _) = setup(false).await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            crate::accounts::Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let mut peers = vec![(b.id, b.token, b.instance, b.report.sequence)];
    for number in 0..8u8 {
        let signer = ed25519_dalek::SigningKey::from_bytes(&[100 + number; 32]);
        let enrolled = super::node::enroll_tag_node(
            &app,
            owner.token.expose(),
            network,
            &signer,
            &format!("peer-{number}"),
            [120 + number; 32],
        )
        .await;
        let token = session(
            &app.execution(),
            network,
            enrolled.node_id,
            &enrolled.keys,
            &signer,
        )
        .await;
        let lease = app
            .execution()
            .acquire(
                token.token.expose(),
                AcquireInstanceRequest {
                    instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                    capabilities: caps(),
                },
            )
            .await
            .unwrap();
        peers.push((
            enrolled.node_id,
            token.token.expose().into(),
            InstanceRequest {
                instance_id: lease.instance_id,
                generation: lease.generation,
            },
            Sequence::MIN,
        ));
    }
    for (peer, _, _, _) in &peers {
        let decision = app
            .execution()
            .demand(
                &a.token,
                DemandRequest {
                    instance: a.instance.clone(),
                    request_id: rid(),
                    target: BusinessTarget::Mesh { node: *peer },
                    protocol: BusinessProtocol::Udp,
                    port: Some(53.try_into().unwrap()),
                    profile: "mesh".into(),
                    duration_seconds: 60,
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            decision.decision,
            DemandDecision::Permitted { .. }
        ));
    }
    // Finish the real publication/receipt work before producing observations.
    // Each producer owns its report sequence; a full network State projection
    // is unnecessary here and would age earlier candidates during setup.
    let mut synchronized = Vec::new();
    for (id, token, instance, sequence) in
        std::iter::once((a.id, a.token, a.instance, a.report.sequence)).chain(peers)
    {
        let target = sync_execution(&app, &token, &instance).await;
        synchronized.push((id, token, instance, sequence, target));
    }
    let mut ready = Vec::new();
    for (index, (id, token, instance, sequence, target)) in synchronized.into_iter().enumerate() {
        let mut report = report(
            &target,
            &instance,
            &format!("198.51.100.{}:23456", index + 1),
        );
        report.sequence = sequence;
        let mut node = Node {
            id,
            token,
            instance,
            target,
            report,
        };
        node.publish(&app).await;
        ready.push(node);
    }
    let a = ready.remove(0);
    // The batch shares one actually queried epoch. Every prepare still checks
    // its exact instance/configuration and both candidate facts transactionally.
    let QueryUdpTraversalResponse::Current { state_epoch, .. } =
        app.connectivity().query(&a.token, a.query()).await.unwrap()
    else {
        panic!("current metadata");
    };
    let mut offers = Vec::new();
    for (index, peer) in ready.iter().enumerate() {
        let request = request_in_epoch(&a, peer, state_epoch);
        let result = app.connectivity().prepare(&a.token, request).await.unwrap();
        if index < 8 {
            let PrepareUdpTraversalResponse::Offer { offer } = result else {
                panic!(
                    "slot {index}: {result:?}; source report age={}, peer report age={}",
                    chrono::Utc::now() - a.report.observed_at,
                    chrono::Utc::now() - peer.report.observed_at,
                );
            };
            offers.push(offer);
        } else {
            assert_eq!(
                result,
                PrepareUdpTraversalResponse::Unavailable {
                    reason: UdpTraversalUnavailableReason::Capacity
                }
            );
        }
    }
    assert_eq!(offers.len(), 8);
    let QueryUdpTraversalResponse::Current { attempts, .. } =
        app.connectivity().query(&a.token, a.query()).await.unwrap()
    else {
        panic!("current attempts");
    };
    assert_eq!(attempts.as_slice().len(), 8);
    for attempt in attempts.as_slice() {
        assert_eq!(attempt.phase, UdpTraversalPhase::Waiting);
        assert!(offers.contains(&attempt.offer));
    }
    let first = &offers[0];
    assert_eq!(
        app.connectivity()
            .cancel(&a.token, first.attempt_id, a.cas(first))
            .await
            .unwrap(),
        CancelUdpTraversalResponse::Cancelled,
    );
    let last = ready.last().unwrap();
    let _replacement = offer(&app, &a, request_in_epoch(&a, last, state_epoch)).await;
    // Cancellation freed the slot: none of the original fixed deadlines has
    // elapsed, including through the current-authority query above.
    let now = chrono::Utc::now().timestamp_millis() as u64;
    assert!(offers.iter().all(|offer| offer.expires_at_ms > now));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn foreign_network_participants_cannot_prepare_accept_or_cancel_an_offer() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b, _) = setup(true).await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            crate::accounts::Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let network = app
        .networks()
        .create(owner.token.expose(), "foreign traversal".into(), rid())
        .await
        .unwrap();
    app.configurations()
        .apply(
            owner.token.expose(),
            network.id,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: network.current_revision,
                request_id: rid(),
                ops: vec![
                    meshway_configuration::ConfigurationOperation::ReplaceTagsOwners {
                        tags_owners: BTreeMap::from([("tag:test".into(), vec![])]),
                    },
                ],
            },
            rid(),
        )
        .await
        .unwrap();
    let signer = ed25519_dalek::SigningKey::from_bytes(&[90; 32]);
    let foreign = super::node::enroll_tag_node(
        &app,
        owner.token.expose(),
        network.id,
        &signer,
        "foreign",
        [91; 32],
    )
    .await;
    let token = session(
        &app.execution(),
        network.id,
        foreign.node_id,
        &foreign.keys,
        &signer,
    )
    .await;
    let lease = app
        .execution()
        .acquire(
            token.token.expose(),
            AcquireInstanceRequest {
                instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let foreign = sync(
        &app,
        foreign.node_id,
        token.token.expose().into(),
        InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        },
        "198.51.100.9:23456",
    )
    .await;
    assert!(matches!(
        app.connectivity()
            .prepare(&a.token, request(&app, &a, &foreign).await)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable { .. }
    ));
    let first = offer(&app, &a, request(&app, &a, &b).await).await;
    assert_eq!(
        app.connectivity()
            .accept(&foreign.token, first.attempt_id, foreign.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::NotAvailable
        }
    );
    assert_eq!(
        app.connectivity()
            .cancel(&foreign.token, first.attempt_id, foreign.cas(&first))
            .await
            .unwrap(),
        CancelUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::NotAvailable
        }
    );
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Waiting { .. }
    ));
    assert!(matches!(
        app.connectivity()
            .accept(&b.token, first.attempt_id, b.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Active { .. }
    ));
    assert!(matches!(
        app.connectivity()
            .query(owner.token.expose(), a.query())
            .await,
        Err(crate::error::Error::InvalidCredentials)
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn original_candidate_expiry_and_declared_listener_bind_are_checked_at_prepare() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, mut a, b, _) = setup(true).await;
    a.report
        .state
        .sockets
        .get_mut("socket")
        .unwrap()
        .bound_address
        .set_port(23457);
    a.publish(&app).await;
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, request(&app, &a, &b).await)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::CandidateChanged
        }
    );
    let socket = a.report.state.sockets.get_mut("socket").unwrap();
    socket.bound_address.set_port(23456);
    socket.endpoints[0].observed_at = chrono::Utc::now() - chrono::Duration::seconds(3);
    socket.endpoints[0].expires_at = chrono::Utc::now() - chrono::Duration::seconds(1);
    a.publish(&app).await;
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, request(&app, &a, &b).await)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::CandidateChanged
        }
    );
    app.shutdown().await.unwrap();
}
