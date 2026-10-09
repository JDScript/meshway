use super::*;
use meshway_protocol::configuration::ConfigurationRequest;
use meshway_transport_spec::{PreparedListener, TransportProtocol};

#[test]
fn configuration_schema_exposes_optional_strict_transport_protocol_array() {
    let response = crate::configuration::catalog::schema(rid()).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&response.schema_json).unwrap();
    let rule = &schema["$defs"]["TopologyRule"];
    assert_eq!(rule["properties"]["transports"]["type"], "array");
    assert_eq!(
        rule["properties"]["transports"]["items"]["$ref"],
        "#/$defs/TransportProtocol"
    );
    assert!(
        !rule["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("transports"))
    );
    assert_eq!(
        schema["$defs"]["TransportProtocol"]["enum"],
        serde_json::json!(["native_udp", "native_tcp", "vless", "trojan"])
    );
}

#[tokio::test]
async fn topology_transport_validate_apply_and_execution_share_the_intersection() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, asigner, bsigner) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let configurations = app.configurations();
    let current = configurations
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let mut cfg = current.document.configuration.clone();
    cfg.topology = vec![TopologyRule {
        remark: String::new(),
        action: TopologyAction::Allow,
        a: format!("node:{a}"),
        b: format!("node:{b}"),
        transports: Input::Value(vec![TransportProtocol::NativeTcp]),
 keepalive: false,
    }];
    cfg.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{a}")],
        dst: vec![format!("node:{b}")],
        via: vec![],
    }];
    cfg.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{a}")],
        dst: vec![format!("node:{b}")],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    let request = |cfg: &NetworkConfiguration| ConfigurationRequest::Document {
        expected_revision: current.revision,
        request_id: rid(),
        source_jsonc: WireSecret::new(canonical_json(cfg).unwrap().expose().into()),
    };
    let validate = configurations
        .validate(owner.token.expose(), network, request(&cfg), rid())
        .await
        .err()
        .unwrap();
    let apply = configurations
        .apply(owner.token.expose(), network, request(&cfg), rid())
        .await
        .err()
        .unwrap();
    for error in [validate, apply] {
        let Error::InvalidConfiguration(ConfigurationError::Invalid(issues)) = error else {
            panic!("expected configuration error: {error:?}");
        };
        let issue = issues
            .iter()
            .find(|i| i.code == IssueCode::InvalidTransport)
            .unwrap();
        assert_eq!(issue.path, "/topology/0/transports");
        assert_eq!(issue.params["reason"], "edge_transport_unavailable");
        assert_eq!(issue.params["routingRule"], "/routing/mesh/0");
        let pair = [issue.params["a"].clone(), issue.params["b"].clone()];
        assert!(pair.contains(&a.to_string()) && pair.contains(&b.to_string()));
    }
    assert_eq!(
        configurations
            .read(owner.token.expose(), network, None)
            .await
            .unwrap()
            .revision,
        current.revision
    );
    cfg.defaults.dialers =
        serde_json::from_value(serde_json::json!({"native_udp":{},"native_tcp":{}})).unwrap();
    for settings in cfg.nodes.values_mut() {
        settings.listeners.insert(
            "tcp".into(),
            PreparedListener::prepare(
                serde_json::from_value(serde_json::json!({"type":"native_tcp","listenPort":23457}))
                    .unwrap(),
            )
            .unwrap(),
        );
    }
    let preview = configurations
        .validate(owner.token.expose(), network, request(&cfg), rid())
        .await
        .unwrap();
    assert_eq!(
        preview.preview.topology[0].transports,
        cfg.topology[0].transports
    );
    configurations
        .apply(owner.token.expose(), network, request(&cfg), rid())
        .await
        .unwrap();
    let s = app.execution();
    let mut sessions = Vec::new();
    let mut instances = Vec::new();
    for (node, keys, signer) in [(a, &ak, &asigner), (b, &bk, &bsigner)] {
        let session = session(&s, network, node, keys, signer).await;
        let mut capabilities = caps();
        capabilities.insert(
            "transport:native_tcp".into(),
            Capability::Transport {
                roles: vec![TransportRole::Dial, TransportRole::Listen],
                streams: vec![StreamKind::Tcp],
            },
        );
        let lease = s
            .acquire(
                session.token.expose(),
                AcquireInstanceRequest {
                    instance_id: iid(),
                    capabilities,
                },
            )
            .await
            .unwrap();
        instances.push(InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        });
        sessions.push(session);
    }
    let demand_id = rid();
    let result = s
        .demand(
            sessions[0].token.expose(),
            DemandRequest {
                instance: instances[0].clone(),
                request_id: demand_id,
                target: BusinessTarget::Mesh { node: b },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 60,
            },
        )
        .await
        .unwrap();
    assert!(matches!(result.decision, DemandDecision::Permitted { .. }));
    for (session, instance) in sessions.iter().zip(instances.iter()) {
        let response = s
            .configuration(
                session.token.expose(),
                NodeConfigurationRequest {
                    instance: instance.clone(),
                    base: None,
                },
            )
            .await
            .unwrap();
        let cfg = response.full.unwrap().configuration;
        assert_eq!(cfg.dialers.len(), 1);
        assert!(
            cfg.dialers
                .values()
                .all(|d| matches!(d.parameters, DialParameters::NativeTcp { .. }))
        );
        assert!(
            cfg.permits[&demand_id]
                .paths
                .iter()
                .flat_map(|p| p.dial_candidates.values())
                .flatten()
                .all(|key| key.ends_with("/tcp"))
        );
        assert!(
            cfg.listeners.contains_key("udp"),
            "node-level listener availability does not widen edge admission"
        );
    }
    app.shutdown().await.unwrap();
}
