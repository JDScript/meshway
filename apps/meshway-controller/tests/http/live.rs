//! Public HTTP + real SQLite + signed Node admission. No SQL-created inventory,
//! identity provider, clock substitution or fake execution payload.
use super::management::{create, join, net, request, source};
use super::*;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_controller::execution::proof;
use meshway_model::{state::*, *};
use meshway_protocol::{
    configuration::*, enrollment::*, events::StreamEvent, execution::*, live::*, node::*, state::*,
};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

#[test]
fn live_spec_closes_real_event_roots_and_resolves_every_dependency() {
    let spec = serde_json::to_value(http::openapi()).unwrap();
    fn references(value: &serde_json::Value, spec: &serde_json::Value) {
        match value {
            serde_json::Value::Object(object) => {
                if let Some(reference) = object.get("$ref").and_then(serde_json::Value::as_str) {
                    assert!(
                        spec.pointer(reference.strip_prefix('#').unwrap()).is_some(),
                        "unresolved {reference}"
                    );
                }
                for value in object.values() {
                    references(value, spec);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    references(value, spec);
                }
            }
            _ => {}
        }
    }
    references(&spec, &spec);
    for (path, event, scheme) in [
        (
            "/api/v1/networks/{networkId}/events",
            "UserEvent",
            "userSession",
        ),
        ("/agent-api/v1/events", "NodeEvent", "nodeSession"),
    ] {
        let operation = &spec["paths"][path]["get"];
        assert_eq!(
            operation["responses"]["200"]["content"]["text/event-stream"]["schema"]["type"],
            "string"
        );
        assert_eq!(
            operation["x-meshway-sse"]["eventSchema"]["$ref"],
            format!("#/components/schemas/{event}")
        );
        assert!(operation["security"][0][scheme].is_array());
        for branch in spec["components"]["schemas"][event]["oneOf"]
            .as_array()
            .unwrap()
        {
            assert_eq!(branch["additionalProperties"], false);
        }
    }
    assert!(
        spec["components"]["securitySchemes"]
            .get("nodeCredential")
            .is_none()
    );
    for invalid in [
        r#"[]"#,
        r#"{"type":"sync.ready","batch":"b","cursor":"c","heartbeatIntervalMs":100,"disconnectAfterMs":200,"extra":1}"#,
        r#"{"type":"config.snapshot","batch":"b","configuration":{"type":"member","networkId":"00000000-0000-0000-0000-000000000001","revision":1,"nodes":[]},"progress":null}"#,
    ] {
        assert!(serde_json::from_str::<meshway_protocol::UserEvent>(invalid).is_err());
        assert!(serde_json::from_str::<meshway_protocol::NodeEvent>(invalid).is_err());
    }
    assert!(serde_json::from_str::<SystemState>(r#"["host","linux","x86","1"]"#).is_err());
}

async fn json<T: serde::de::DeserializeOwned>(request: reqwest::RequestBuilder) -> T {
    let response = request.send().await.unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert!(status.is_success(), "{status}: {body}");
    serde_json::from_str(&body).unwrap_or_else(|error| panic!("{error}: {body}"))
}
async fn enroll(
    c: &Client,
    s: &Server,
    network: NetworkId,
    token: &str,
    seed: u8,
) -> (EnrollmentResponse, String, InstanceLease) {
    let signer = SigningKey::from_bytes(&[seed; 32]);
    let verifier = "x".repeat(43);
    let intent = EnrollmentIntent {
        id: EnrollmentRequestId::new(*request().as_uuid()).unwrap(),
        existing: None,
        control_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: signer.verifying_key().to_bytes().into(),
        },
        dataplane_key: NodePublicKey {
            algorithm: KeyAlgorithm::X25519,
            bytes: [seed + 20; 32].into(),
        },
        hostname: format!("node-{seed}"),
        code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
    };
    let id = intent.id;
    let digest = proof::request_digest(&intent).unwrap();
    let signature = |operation| {
        WireSecret::new(
            URL_SAFE_NO_PAD.encode(
                signer
                    .sign(&enrollment_transcript(id, digest, operation))
                    .to_bytes(),
            ),
        )
    };
    let _: EnrollmentRequestView = json(
        c.post(format!("{}/agent-api/v1/enrollment-requests", s.url))
            .json(&CreateEnrollmentRequest {
                intent,
                signature: signature("create"),
            }),
    )
    .await;
    let _: EnrollmentRequestView = json(
        c.post(format!(
            "{}/api/v1/device-authorizations/{id}/approve",
            s.url
        ))
        .bearer_auth(token)
        .json(&ApproveEnrollmentRequest {
            network_id: network,
        }),
    )
    .await;
    let poll: EnrollmentPollResponse = json(
        c.post(format!(
            "{}/agent-api/v1/enrollment-requests/{id}/query",
            s.url
        ))
        .json(&EnrollmentDeviceProof {
            verifier: WireSecret::new(verifier.clone()),
            signature: signature("poll"),
        }),
    )
    .await;
    let enrolled: EnrollmentResponse = json(
        c.post(format!(
            "{}/agent-api/v1/enrollment-requests/{id}/redeem",
            s.url
        ))
        .json(&RedeemEnrollmentRequest {
            proof: EnrollmentDeviceProof {
                verifier: WireSecret::new(verifier),
                signature: signature("redeem"),
            },
            code: poll.code.unwrap(),
        }),
    )
    .await;
    let intent = NodeSessionIntent {
        request_id: request(),
    };
    let challenge: NodeChallenge = json(
        c.post(format!("{}/agent-api/v1/auth/challenges", s.url))
            .json(&NodeChallengeRequest {
                network_id: network,
                node_id: enrolled.node_id,
                key_id: enrolled.keys.control.id,
                operation: ProofOperation::Session,
                request_digest: proof::request_digest(&intent).unwrap(),
            }),
    )
    .await;
    let session: NodeSessionResponse = json(
        c.post(format!("{}/agent-api/v1/auth/session", s.url))
            .json(&NodeSessionRequest {
                intent,
                proof: NodeProof {
                    challenge_id: challenge.id,
                    signature: WireSecret::new(
                        URL_SAFE_NO_PAD
                            .encode(signer.sign(&proof_transcript(&challenge)).to_bytes()),
                    ),
                },
            }),
    )
    .await;
    let token = session.token.expose().to_owned();
    let lease: InstanceLease = json(
        c.post(format!("{}/agent-api/v1/instances", s.url))
            .bearer_auth(&token)
            .json(&AcquireInstanceRequest {
                instance_id: InstanceId::new(*request().as_uuid()).unwrap(),
                capabilities: BTreeMap::from([(
                    "transport:native_udp".into(),
                    Capability::Transport {
                        roles: vec![TransportRole::Dial, TransportRole::Listen],
                        streams: vec![StreamKind::Udp],
                    },
                )]),
            }),
    )
    .await;
    (enrolled, token, lease)
}
struct Events {
    response: Response,
    buffer: Vec<u8>,
}
impl Events {
    async fn open(c: &Client, url: String, token: &str, cursor: Option<&str>) -> Self {
        let mut request = c.get(url).bearer_auth(token);
        if let Some(cursor) = cursor {
            request = request.header("last-event-id", cursor);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream"
        );
        assert_headers(&response);
        Self {
            response,
            buffer: vec![],
        }
    }
    async fn frame<T: serde::de::DeserializeOwned>(
        &mut self,
    ) -> Option<(Option<String>, T, String)> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(end) = self.buffer.windows(2).position(|bytes| bytes == b"\n\n") {
                    let bytes: Vec<_> = self.buffer.drain(..end + 2).collect();
                    let raw = String::from_utf8(bytes).unwrap();
                    if let Some(data) = raw.lines().find_map(|line| line.strip_prefix("data: ")) {
                        let id = raw
                            .lines()
                            .find_map(|line| line.strip_prefix("id: "))
                            .map(str::to_owned);
                        return Some((id, serde_json::from_str(data).unwrap(), raw));
                    }
                } else {
                    match self.response.chunk().await.unwrap() {
                        Some(bytes) => self.buffer.extend_from_slice(&bytes),
                        None => return None,
                    }
                }
            }
        })
        .await
        .expect("bounded SSE delivery")
    }
    async fn user(&mut self) -> (Option<String>, meshway_protocol::UserEvent, String) {
        self.frame().await.expect("user stream remains open")
    }
    async fn ready(&mut self) -> String {
        loop {
            let (id, event, _) = self.user().await;
            if matches!(event.0, StreamEvent::Ready { .. }) {
                return id.unwrap();
            }
        }
    }
}
fn report(lease: &InstanceLease, sequence: u64) -> StateReport {
    StateReport {
        instance_id: lease.instance_id,
        sequence: Sequence::new(sequence).unwrap(),
        observed_at: chrono::Utc::now(),
        state: NodeState {
            system: SystemState {
                hostname: "reported-host".into(),
                os: "linux".into(),
                arch: "aarch64".into(),
                agent_version: "test-source".into(),
            },
            capabilities: BTreeMap::new(),
            configuration: ConfigurationState::default(),
            listeners: BTreeMap::new(),
            sockets: BTreeMap::new(),
            links: BTreeMap::new(),
            paths: BTreeMap::new(),
            selection: SelectionState::default(),
            probes: vec![],
            problems: vec![],
        },
    }
}

#[tokio::test]
async fn production_reports_snapshots_delta_replay_permissions_and_shutdown() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("live"), true).await;
    let c = client();
    let owner = register(&c, &s, "live-owner@example.test", "valid password").await;
    let member = register(&c, &s, "live-member@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "live")
        .await
        .network
        .id;
    join(
        &c,
        &s,
        network,
        owner.token.expose(),
        member.token.expose(),
        MembershipRole::Member,
    )
    .await;
    let (node, node_token, lease) = enroll(&c, &s, network, member.token.expose(), 5).await;
    let owner_node = enroll(&c, &s, network, owner.token.expose(), 6).await.0;
    let before = source(&c, &s, network, owner.token.expose()).await;
    let mut input = meshway_configuration::NetworkConfigurationInput::from(&before.configuration);
    input
        .groups
        .insert("group:private".into(), vec![owner.user.email.clone()]);
    input.nodes.get_mut(&owner_node.node_id).unwrap().listeners = serde_json::from_str(r#"{"private":{"type":"vless","listenPort":24443,"tls":{"serverName":"private.example.test"},"stream":"websocket","websocket":{"path":"/private","responseHeaders":{"x-private":"PRIVATE-HEADER-VALUE"}}}}"#).unwrap();
    let _: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: before.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            }),
    )
    .await;
    let mut events =
        Events::open(&c, net(&s, network, "events"), member.token.expose(), None).await;
    let (_, first, raw) = events.user().await;
    let StreamEvent::ConfigSnapshot {
        configuration: UserConfigurationView::Member { nodes, .. },
        progress,
        ..
    } = first.0
    else {
        panic!("member projection")
    };
    assert_eq!(nodes.len(), 1);
    assert!(nodes.contains_key(&node.node_id));
    assert_eq!(progress.unwrap().scope, ProgressScope::Partial);
    for forbidden in [
        "group:private",
        "PRIVATE-HEADER-VALUE",
        "privateKey",
        &owner.user.email,
    ] {
        assert!(!raw.contains(forbidden), "{forbidden}");
    }
    let (_, entities, _) = events.user().await;
    let StreamEvent::EntitiesSnapshot { entities, .. } = entities.0 else {
        panic!("entities")
    };
    assert_eq!(entities.members.len(), 1);
    assert!(!entities.nodes[&node.node_id].can_delete);
    let inventory: NodeInventoryResponse = json(
        c.get(net(&s, network, "nodes"))
            .bearer_auth(member.token.expose()),
    )
    .await;
    assert_eq!(entities.nodes[&node.node_id], inventory.items[0]);
    events.ready().await;
    let accepted: StateAccepted = json(
        c.post(format!("{}/agent-api/v1/state", s.url))
            .bearer_auth(&node_token)
            .json(&report(&lease, 1)),
    )
    .await;
    assert!(!accepted.duplicate);
    let (cursor, delta, _) = events.user().await;
    let StreamEvent::StateDelta { updated, .. } = delta.0 else {
        panic!("report delta")
    };
    assert_eq!(updated[&node.node_id].report_sequence.unwrap().get(), 1);
    let cursor = cursor.unwrap();
    let view: StateView = json(
        c.get(net(&s, network, &format!("nodes/{}/state", node.node_id)))
            .bearer_auth(member.token.expose()),
    )
    .await;
    assert_eq!(view.nodes[&node.node_id].report_sequence.unwrap().get(), 1);
    let peers: StateView = json(
        c.post(format!("{}/agent-api/v1/state/query", s.url))
            .bearer_auth(&node_token)
            .json(&StateQuery { nodes: None }),
    )
    .await;
    assert!(peers.nodes.contains_key(&node.node_id));
    let _: StateAccepted = json(
        c.post(format!("{}/agent-api/v1/state", s.url))
            .bearer_auth(&node_token)
            .json(&report(&lease, 2)),
    )
    .await;
    let (second, _, _) = events.user().await;
    drop(events);
    let mut events = Events::open(
        &c,
        net(&s, network, "events"),
        member.token.expose(),
        Some(&cursor),
    )
    .await;
    let (replayed, event, _) = events.user().await;
    assert_eq!(replayed, second);
    assert!(matches!(event.0, StreamEvent::StateDelta { .. }));
    let mut unknown = Events::open(
        &c,
        net(&s, network, "events"),
        member.token.expose(),
        Some("unavailable-cursor"),
    )
    .await;
    assert!(matches!(
        unknown.user().await.1.0,
        StreamEvent::Reset { .. }
    ));
    unknown.ready().await;
    let mut node_events = Events::open(
        &c,
        format!("{}/agent-api/v1/events", s.url),
        &node_token,
        None,
    )
    .await;
    let (_, first, _) = node_events
        .frame::<meshway_protocol::NodeEvent>()
        .await
        .unwrap();
    let StreamEvent::ConfigSnapshot { configuration, .. } = first.0 else {
        panic!("node configuration")
    };
    assert_eq!(configuration.node_id, node.node_id);
    let target: NodeConfigurationResponse = json(
        c.post(format!("{}/agent-api/v1/configuration/query", s.url))
            .bearer_auth(&node_token)
            .json(&NodeConfigurationRequest {
                instance: InstanceRequest {
                    instance_id: lease.instance_id,
                    generation: lease.generation,
                },
                base: None,
            }),
    )
    .await;
    assert_eq!(configuration.target, target.lease.target);
    let now = chrono::Utc::now();
    let history = meshway_protocol::observations::HistoryReport {
        instance_id: lease.instance_id,
        events: vec![meshway_model::observations::ObservationEvent {
            event_id: EventId::new(*request().as_uuid()).unwrap(),
            configuration: target.lease.target.unwrap(),
            occurred_at: now,
            payload: meshway_model::observations::ObservationPayload::CollectionGap {
                dropped: Sequence::new(2).unwrap(),
                from: now - chrono::Duration::seconds(2),
                to: now - chrono::Duration::seconds(1),
            },
        }],
    };
    let history_accepted: meshway_protocol::observations::HistoryAccepted = json(
        c.post(format!("{}/agent-api/v1/history", s.url))
            .bearer_auth(&node_token)
            .json(&history),
    )
    .await;
    assert_eq!(history_accepted.accepted, 1);
    let history_duplicate: meshway_protocol::observations::HistoryAccepted = json(
        c.post(format!("{}/agent-api/v1/history", s.url))
            .bearer_auth(&node_token)
            .json(&history),
    )
    .await;
    assert_eq!(history_duplicate.duplicates, 1);
    let page: meshway_protocol::observations::HistoryPage = json(
        c.post(net(&s, network, "history/query"))
            .bearer_auth(member.token.expose())
            .json(&meshway_protocol::observations::HistoryQuery {
                from: now - chrono::Duration::minutes(1),
                to: now + chrono::Duration::seconds(1),
                source_node_id: Some(node.node_id),
                peer_node_id: None,
                mode: meshway_protocol::observations::HistoryMode::Raw,
                bucket_seconds: None,
                cursor: None,
                limit: 20,
                kinds: None,
                link_id: None,
            }),
    )
    .await;
    let meshway_protocol::observations::HistoryItems::Raw { items } = page.result else {
        panic!("raw history")
    };
    assert_eq!(items.len(), 1);
    for token in [member.token.expose(), node_token.as_str()] {
        let url = if token == node_token {
            format!("{}/agent-api/v1/events", s.url)
        } else {
            net(&s, network, "events")
        };
        assert_eq!(
            c.get(url)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    c.post(s.auth("logout"))
        .bearer_auth(member.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert!(
        events
            .frame::<meshway_protocol::UserEvent>()
            .await
            .is_none()
    );
    assert!(
        unknown
            .frame::<meshway_protocol::UserEvent>()
            .await
            .is_none()
    );
    // Server actively closes the shared core while the Node stream remains
    // owned by a connected HTTP client; SQLite drains only after HTTP exits.
    tokio::time::timeout(Duration::from_secs(5), s.close())
        .await
        .unwrap();
    while node_events
        .frame::<meshway_protocol::NodeEvent>()
        .await
        .is_some()
    {}
}

#[tokio::test]
async fn production_config_changes_and_scope_changes_do_not_become_identity_loss() {
    use meshway_controller::state::{
        service::ReadIdentity,
        stream::{StreamPoll, StreamSource},
    };
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("changes"), true).await;
    let c = client();
    let owner = register(&c, &s, "changes-owner@example.test", "valid password").await;
    let member = register(&c, &s, "changes-member@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "changes")
        .await
        .network
        .id;
    let member_id = join(
        &c,
        &s,
        network,
        owner.token.expose(),
        member.token.expose(),
        MembershipRole::Admin,
    )
    .await;
    let service = s.app.user_state_service().unwrap();
    let identity = |token: &str| ReadIdentity::user_session(token, network).unwrap();
    let member_stream = service
        .open_stream(identity(member.token.expose()), None)
        .await
        .unwrap();
    // Do not poll this initial batch: role loss must discard even its first
    // queued privileged configuration, not merely terminate after draining it.
    let response = c
        .post(net(&s, network, &format!("members/{member_id}/role")))
        .bearer_auth(owner.token.expose())
        .json(&meshway_protocol::ChangeMemberRoleRequest {
            expected_version: Version::MIN,
            role: MembershipRole::Member,
        })
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "{}",
        response.text().await.unwrap()
    );
    assert!(matches!(
        service.poll(&member_stream).await.unwrap(),
        StreamPoll::Closed
    ));
    let owner_stream = service
        .open_stream(identity(owner.token.expose()), None)
        .await
        .unwrap();
    let member_stream = service
        .open_stream(identity(member.token.expose()), None)
        .await
        .unwrap();
    for stream in [&owner_stream, &member_stream] {
        for _ in 0..4 {
            assert!(matches!(
                service.poll(stream).await.unwrap(),
                StreamPoll::Frame(_)
            ));
        }
    }
    let mut http_events =
        Events::open(&c, net(&s, network, "events"), owner.token.expose(), None).await;
    http_events.ready().await;
    for pure_source in [false, true] {
        let current = source(&c, &s, network, owner.token.expose()).await;
        let mut input =
            meshway_configuration::NetworkConfigurationInput::from(&current.configuration);
        if !pure_source {
            input
                .groups
                .insert("group:hidden".into(), vec![owner.user.email.clone()]);
        }
        let submitted = request();
        let applied: ConfigurationAppliedResponse = json(
            c.post(net(&s, network, "configuration"))
                .bearer_auth(owner.token.expose())
                .json(&ConfigurationRequest::Document {
                    expected_revision: current.revision,
                    request_id: submitted,
                    source_jsonc: WireSecret::new(format!(
                        "// source-only={pure_source}\n{}",
                        serde_json::to_string(&input).unwrap()
                    )),
                }),
        )
        .await;
        loop {
            let (_, event, _) = http_events.user().await;
            match event.0 {
                StreamEvent::ConfigurationChanged {
                    revision, actor, ..
                } => {
                    assert_eq!(revision, applied.revision);
                    assert!(
                        matches!(actor, meshway_protocol::events::EventActor::User { user_id } if user_id == owner.user.id)
                    );
                    break;
                }
                StreamEvent::Reset { .. } => {
                    panic!("ordinary manager configuration changes retain HTTP stream continuity")
                }
                _ => {}
            }
        }
        let StreamPoll::Frame(frame) = service.poll(&owner_stream).await.unwrap() else {
            panic!("ordinary edit must not close stream")
        };
        let event: meshway_protocol::UserEvent = serde_json::from_str(&frame.data).unwrap();
        let StreamEvent::ConfigurationChanged {
            revision,
            actor,
            changed_pointers,
            ..
        } = event.0
        else {
            panic!("manager gets committed notice: {}", frame.data)
        };
        assert_eq!(revision, applied.revision);
        assert!(matches!(
            actor,
            meshway_protocol::events::EventActor::User { .. }
        ));
        assert_eq!(changed_pointers.is_empty(), pure_source);
        let StreamPoll::Frame(frame) = service.poll(&member_stream).await.unwrap() else {
            panic!("member notice")
        };
        let event: meshway_protocol::UserEvent = serde_json::from_str(&frame.data).unwrap();
        let StreamEvent::ConfigurationChanged {
            actor,
            changed_pointers,
            changes_complete,
            request_id,
            ..
        } = event.0
        else {
            panic!("member notice")
        };
        assert!(matches!(
            actor,
            meshway_protocol::events::EventActor::Redacted
        ));
        assert!(changed_pointers.is_empty());
        assert!(!changes_complete);
        assert!(request_id.is_none());
        for stream in [&owner_stream, &member_stream] {
            loop {
                match service.poll(stream).await.unwrap() {
                    StreamPoll::Frame(_) => {}
                    StreamPoll::Pending => break,
                    StreamPoll::Closed => panic!("configuration is not credential loss"),
                }
            }
        }
    }
    let queued = service
        .open_stream(identity(owner.token.expose()), None)
        .await
        .unwrap();
    for _ in 0..4 {
        assert!(matches!(
            service.poll(&queued).await.unwrap(),
            StreamPoll::Frame(_)
        ));
    }
    let current = source(&c, &s, network, owner.token.expose()).await;
    let first: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(format!(
                    "// first queued edit\n{}",
                    current.source_jsonc.expose()
                )),
            }),
    )
    .await;
    let StreamPoll::Frame(frame) = service.poll(&queued).await.unwrap() else {
        panic!("first notice")
    };
    assert!(matches!(
        serde_json::from_str::<meshway_protocol::UserEvent>(&frame.data)
            .unwrap()
            .0,
        StreamEvent::ConfigurationChanged { .. }
    ));
    let second: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: first.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(format!(
                    "// second queued edit\n{}",
                    current.source_jsonc.expose()
                )),
            }),
    )
    .await;
    loop {
        let StreamPoll::Frame(frame) = service.poll(&queued).await.unwrap() else {
            panic!("drain still-authorized manager queue")
        };
        match serde_json::from_str::<meshway_protocol::UserEvent>(&frame.data)
            .unwrap()
            .0
        {
            StreamEvent::ConfigurationChanged { revision, .. } => {
                assert_eq!(revision, second.revision);
                break;
            }
            StreamEvent::Reset { .. } => {
                panic!("a complete manager view retains ordered ordinary pending frames")
            }
            _ => {}
        }
    }
    // A write during an unfinished full sync discards that batch and sends a
    // new authorized snapshot. The old cursor never becomes ready.
    let initial = service
        .open_stream(identity(owner.token.expose()), None)
        .await
        .unwrap();
    assert!(matches!(
        service.poll(&initial).await.unwrap(),
        StreamPoll::Frame(_)
    ));
    let current = source(&c, &s, network, owner.token.expose()).await;
    let _: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(format!(
                    "// pending change\n{}",
                    current.source_jsonc.expose()
                )),
            }),
    )
    .await;
    let StreamPoll::Frame(frame) = service.poll(&initial).await.unwrap() else {
        panic!("resynchronize initial batch")
    };
    assert!(matches!(
        serde_json::from_str::<meshway_protocol::UserEvent>(&frame.data)
            .unwrap()
            .0,
        StreamEvent::Reset { .. }
    ));
    // Logout also invalidates a fully pending replacement batch immediately.
    c.post(s.auth("logout"))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert!(matches!(
        service.poll(&initial).await.unwrap(),
        StreamPoll::Closed
    ));
    s.close().await;
}

#[tokio::test]
async fn production_demand_receipts_peer_expansion_and_node_termination() {
    use meshway_configuration::execution::{BusinessProtocol, BusinessTarget};
    use meshway_controller::state::{
        service::ReadIdentity,
        stream::{StreamPoll, StreamSource},
    };
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(
        &dir.path().canonicalize().unwrap().join("execution-events"),
        true,
    )
    .await;
    let c = client();
    let owner = register(&c, &s, "execution-owner@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "execution-events")
        .await
        .network
        .id;
    let (a, at, ai) = enroll(&c, &s, network, owner.token.expose(), 11).await;
    let (b, bt, _) = enroll(&c, &s, network, owner.token.expose(), 12).await;
    let current = source(&c, &s, network, owner.token.expose()).await;
    let mut input = meshway_configuration::NetworkConfigurationInput::from(&current.configuration);
    for node in [a.node_id, b.node_id] {
        input.nodes.get_mut(&node).unwrap().listeners =
            serde_json::from_str(r#"{"udp":{"type":"native_udp","listenPort":23456}}"#).unwrap();
    }
    let applied: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            }),
    )
    .await;
    let progress_url = net(
        &s,
        network,
        &format!("configuration/revisions/{}/progress", applied.revision),
    );
    let fixed: ConfigurationApplyProgress =
        json(c.get(&progress_url).bearer_auth(owner.token.expose())).await;
    assert_eq!(fixed.summary.target_count, 2);
    let mut progress_events =
        Events::open(&c, net(&s, network, "events"), owner.token.expose(), None).await;
    progress_events.ready().await;
    let original = fixed.nodes[&a.node_id].target.clone();
    let instance = InstanceRequest {
        instance_id: ai.instance_id,
        generation: ai.generation,
    };
    for (phase, sequence) in [(ReceiptPhase::Saved, 0), (ReceiptPhase::Applied, 1)] {
        assert_eq!(
            c.post(format!("{}/agent-api/v1/configuration/receipts", s.url))
                .bearer_auth(&at)
                .json(&ReceiptRequest {
                    instance: instance.clone(),
                    target: original.clone(),
                    phase,
                    attempt: Version::MIN,
                    sequence: Sequence::new(sequence).unwrap(),
                    reported_at_ms: chrono::Utc::now().timestamp_millis() as u64,
                    failure: None
                })
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
        if phase == ReceiptPhase::Saved {
            let _: InstanceLease = json(
                c.post(format!("{}/agent-api/v1/instances/renew", s.url))
                    .bearer_auth(&at)
                    .json(&instance),
            )
            .await;
        }
    }
    loop {
        let (_, event, _) = progress_events.user().await;
        match event.0 {
            StreamEvent::ConfigurationProgress { progress } if progress.summary.applied == 1 => {
                let queried: ConfigurationApplyProgress =
                    json(c.get(&progress_url).bearer_auth(owner.token.expose())).await;
                assert!(progress == queried);
                assert_eq!(progress.nodes[&a.node_id].target, original);
                assert_eq!(
                    progress.nodes[&a.node_id].freshness,
                    ReceiptFreshness::Current
                );
                break;
            }
            StreamEvent::Reset { .. } => panic!("receipt is data, not manager authorization loss"),
            _ => {}
        }
    }
    let mut stream = Events::open(&c, format!("{}/agent-api/v1/events", s.url), &at, None).await;
    for _ in 0..4 {
        stream.frame::<meshway_protocol::NodeEvent>().await.unwrap();
    }
    let demand: DemandResponse = json(
        c.post(format!("{}/agent-api/v1/demands", s.url))
            .bearer_auth(&at)
            .json(&DemandRequest {
                instance: instance.clone(),
                request_id: request(),
                target: BusinessTarget::Mesh { node: b.node_id },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 60,
            }),
    )
    .await;
    let mut target = None;
    loop {
        let (_, event, _) = stream
            .frame::<meshway_protocol::NodeEvent>()
            .await
            .expect("demand is not credential loss");
        match event.0 {
            StreamEvent::ConfigSnapshot { configuration, .. } => target = configuration.target,
            StreamEvent::Ready { .. } => break,
            _ => {}
        }
    }
    let target = target.unwrap();
    assert_eq!(target.execution_publication, demand.publication);
    let peer: StateView = json(
        c.post(format!("{}/agent-api/v1/state/query", s.url))
            .bearer_auth(&at)
            .json(&StateQuery {
                nodes: Some(vec![b.node_id]),
            }),
    )
    .await;
    assert!(peer.nodes.contains_key(&b.node_id));
    for (phase, sequence) in [(ReceiptPhase::Saved, 0), (ReceiptPhase::Applied, 1)] {
        assert_eq!(
            c.post(format!("{}/agent-api/v1/configuration/receipts", s.url))
                .bearer_auth(&at)
                .json(&ReceiptRequest {
                    instance: instance.clone(),
                    target: target.clone(),
                    phase,
                    attempt: Version::MIN,
                    sequence: Sequence::new(sequence).unwrap(),
                    reported_at_ms: chrono::Utc::now().timestamp_millis() as u64,
                    failure: None
                })
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
    }
    let mut source_report = report(&ai, 1);
    source_report.state.configuration.saved = Some(target.clone());
    source_report.state.configuration.applied = Some(target.clone());
    let observed_at = source_report.observed_at;
    source_report.state.sockets.insert(
        "udp-socket".into(),
        SocketState {
            resource: ResourceRef::Listener { key: "udp".into() },
            configuration: Some(target.clone()),
            generation: Sequence::new(1).unwrap(),
            network: SocketNetwork::Udp,
            bound_address: "0.0.0.0:23456".parse().unwrap(),
            endpoints: vec![
                EndpointCandidate {
                    address: "192.0.2.10:23456".into(),
                    origin: EndpointSource::Interface {
                        interface: "private-interface-fixture".into(),
                    },
                    observed_at,
                    expires_at: observed_at + chrono::Duration::seconds(30),
                },
                EndpointCandidate {
                    address: "203.0.113.20:40000".into(),
                    origin: EndpointSource::Stun {
                        observer: "192.0.2.1:3478".into(),
                    },
                    observed_at,
                    expires_at: observed_at + chrono::Duration::seconds(30),
                },
            ],
        },
    );
    source_report.state.listeners.insert(
        "udp".into(),
        ListenerState {
            phase: ListenerPhase::Listening,
            socket_ids: vec!["udp-socket".into()],
            loaded_certificate_sha256: None,
            problem: None,
            configuration: Some(target.clone()),
        },
    );
    let _: StateAccepted = json(
        c.post(format!("{}/agent-api/v1/state", s.url))
            .bearer_auth(&at)
            .json(&source_report),
    )
    .await;
    let (_, event, _) = stream.frame::<meshway_protocol::NodeEvent>().await.unwrap();
    assert!(matches!(event.0, StreamEvent::StateDelta { .. }));
    let peer: StateView = json(
        c.post(format!("{}/agent-api/v1/state/query", s.url))
            .bearer_auth(&bt)
            .json(&StateQuery {
                nodes: Some(vec![a.node_id]),
            }),
    )
    .await;
    let VisibleReport::Peer {
        report: peer_report,
    } = peer.nodes[&a.node_id].report.as_ref().unwrap()
    else {
        panic!("peer projection")
    };
    assert_eq!(peer_report.endpoints.len(), 2);
    let peer_json = serde_json::to_string(&peer).unwrap();
    for private in [
        "private-interface-fixture",
        "0.0.0.0:23456",
        "192.0.2.1:3478",
    ] {
        assert!(!peer_json.contains(private));
    }
    // A self-report cannot turn an interface observation into a configured
    // endpoint absent from the Controller's exact published listener values.
    let mut invalid = source_report.clone();
    invalid.sequence = Sequence::new(2).unwrap();
    invalid
        .state
        .sockets
        .get_mut("udp-socket")
        .unwrap()
        .endpoints[0]
        .origin = EndpointSource::Configured {
        config_path: format!("/nodes/{}/listeners/udp/endpoints/0", a.node_id),
    };
    assert_eq!(
        c.post(format!("{}/agent-api/v1/state", s.url))
            .bearer_auth(&at)
            .json(&invalid)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    let after: ConfigurationApplyProgress =
        json(c.get(&progress_url).bearer_auth(owner.token.expose())).await;
    assert_eq!(after.summary.target_count, fixed.summary.target_count);
    assert_eq!(after.nodes[&a.node_id].phase, ApplicationPhase::Applied);
    assert_eq!(
        after.nodes[&a.node_id].freshness,
        ReceiptFreshness::Superseded
    );
    assert_eq!(
        after.nodes.keys().collect::<Vec<_>>(),
        fixed.nodes.keys().collect::<Vec<_>>()
    );
    assert!(
        after
            .nodes
            .values()
            .all(|node| node.target.execution_publication != demand.publication)
    );
    // A source-only revision does not invalidate the still-eligible earlier
    // target's peer dependencies (the target revision is deliberately older).
    let current = source(&c, &s, network, owner.token.expose()).await;
    let _: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(format!(
                    "// comment\n{}",
                    current.source_jsonc.expose()
                )),
            }),
    )
    .await;
    let peer: StateView = json(
        c.post(format!("{}/agent-api/v1/state/query", s.url))
            .bearer_auth(&at)
            .json(&StateQuery {
                nodes: Some(vec![b.node_id]),
            }),
    )
    .await;
    assert!(peer.nodes.contains_key(&b.node_id));
    // Use the production poll boundary to deterministically leave an initial
    // batch queued while the real HTTP key-revocation transaction commits.
    let node_service = s.app.node_state_service().unwrap();
    let queued = node_service
        .open_stream(
            ReadIdentity::Node {
                authorization: bt.clone(),
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        c.post(net(
            &s,
            network,
            &format!("nodes/{}/keys/revoke", b.node_id)
        ))
        .bearer_auth(owner.token.expose())
        .json(&RevokeNodeKeyRequest {
            key_id: b.keys.control.id,
            expected_generation: b.keys.control.generation
        })
        .send()
        .await
        .unwrap()
        .status(),
        StatusCode::NO_CONTENT
    );
    assert!(matches!(
        node_service.poll(&queued).await.unwrap(),
        StreamPoll::Closed
    ));
    assert_eq!(
        c.post(format!("{}/agent-api/v1/instances/release", s.url))
            .bearer_auth(&at)
            .json(&instance)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    // Drain only frames already emitted before the release. The transport must
    // terminate, independently of its keepalive connection or a new request.
    while stream
        .frame::<meshway_protocol::NodeEvent>()
        .await
        .is_some()
    {}
    s.close().await;
}
