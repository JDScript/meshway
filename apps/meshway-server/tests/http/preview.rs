use super::live::{enroll, json};
use super::management::{create, net, request, source};
use super::*;
use meshway_configuration::transport::MaterialRequirement;
use meshway_configuration::{ConfigurationOperation, PreparationKind};
use meshway_protocol::configuration::*;

fn persisted(path: &Path) -> Vec<i64> {
    let db = rusqlite::Connection::open_with_flags(
        path.join("controller.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    [
        "SELECT count(*) FROM configuration_versions",
        "SELECT count(*) FROM configuration_commits",
        "SELECT count(*) FROM execution_publications",
        "SELECT sum(publication_sequence) FROM networks",
        "SELECT count(*) FROM address_retirements",
        "SELECT count(*) FROM credential_revocations",
    ]
    .into_iter()
    .map(|sql| db.query_row(sql, [], |r| r.get(0)).unwrap())
    .collect()
}

#[tokio::test]
async fn generated_listener_preview_is_read_only_and_matches_real_apply() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap().join("preview");
    let s = Server::start(&path, true).await;
    let c = client();
    let owner = register(&c, &s, "preview@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "preview")
        .await
        .network
        .id;
    let node = enroll(&c, &s, network, owner.token.expose(), 50).await.0;
    let listener: meshway_configuration::ListenerUpdate = serde_json::from_str(r#"{"type":"vless","listenPort":24443,"tls":{"serverName":"preview.example.test"},"stream":"websocket","websocket":{"path":"/mesh"}}"#).unwrap();
    for recreate in [false, true] {
        let before = source(&c, &s, network, owner.token.expose()).await;
        let mut ops = vec![];
        if recreate {
            ops.push(ConfigurationOperation::SetListener {
                node_id: node.node_id,
                key: "vless".into(),
                listener: None,
            });
        }
        ops.push(ConfigurationOperation::SetListener {
            node_id: node.node_id,
            key: "vless".into(),
            listener: Some(listener.clone()),
        });
        let draft = ConfigurationRequest::Operations {
            expected_revision: before.revision,
            request_id: request(),
            ops,
        };
        let stored = persisted(&path);
        let preview: ConfigurationPreviewResponse = json(
            c.post(net(&s, network, "configuration/validate"))
                .bearer_auth(owner.token.expose())
                .json(&draft),
        )
        .await;
        assert_eq!(persisted(&path), stored);
        let after_preview = source(&c, &s, network, owner.token.expose()).await;
        assert!(after_preview.configuration == before.configuration);
        assert_eq!(after_preview.revision, before.revision);
        let view = &preview.preview.nodes[&node.node_id].listeners["vless"];
        assert!(!view.credential_present);
        assert!(!view.tls.as_ref().unwrap().certificate_present);
        assert!(!view.tls.as_ref().unwrap().private_key_present);
        assert_eq!(preview.change_count as usize, preview.changes.len());
        assert_eq!(preview.will_generate.len(), 3);
        assert!(preview.requirements.iter().any(
            |r| r.kind == PreparationKind::Material(MaterialRequirement::CertificateValidation)
        ));
        assert!(
            preview
                .will_generate
                .iter()
                .all(|r| r.path == format!("/nodes/{}/listeners/vless", node.node_id))
        );
        let repeated: ConfigurationPreviewResponse = json(
            c.post(net(&s, network, "configuration/validate"))
                .bearer_auth(owner.token.expose())
                .json(&draft),
        )
        .await;
        assert_eq!(preview.preview, repeated.preview);
        assert_eq!(preview.changes, repeated.changes);
        assert_eq!(persisted(&path), stored);
        let applied: ConfigurationAppliedResponse = json(
            c.post(net(&s, network, "configuration"))
                .bearer_auth(owner.token.expose())
                .json(&draft),
        )
        .await;
        let actual = source(&c, &s, network, owner.token.expose()).await;
        assert_eq!(actual.revision, applied.revision);
        let declaration =
            actual.configuration.nodes[&node.node_id].listeners["vless"].declaration();
        let meshway_configuration::transport::Listener::Vless(vless) = declaration else {
            panic!("VLESS")
        };
        assert!(uuid::Uuid::parse_str(vless.uuid.as_ref().unwrap().expose()).is_ok());
        assert!(vless.tls.certificate.is_some() && vless.tls.private_key.is_some());
        if recreate {
            assert!(
                before.configuration.nodes[&node.node_id].listeners["vless"].declaration()
                    != declaration
            );
        }
        let comparison: ConfigurationComparison = json(
            c.post(net(&s, network, "configuration/compare"))
                .bearer_auth(owner.token.expose())
                .json(&CompareConfigurationRequest {
                    from_revision: before.revision,
                    to_revision: applied.revision,
                }),
        )
        .await;
        assert_eq!(preview.changes, comparison.changes);
        let complete: ConfigurationPreviewResponse = json(
            c.post(net(&s, network, "configuration/validate"))
                .bearer_auth(owner.token.expose())
                .json(&ConfigurationRequest::Operations {
                    expected_revision: actual.revision,
                    request_id: request(),
                    ops: vec![],
                }),
        )
        .await;
        assert!(complete.will_generate.is_empty() && complete.requirements.is_empty());
        assert_eq!(complete.change_count, 0);
    }
    let current = source(&c, &s, network, owner.token.expose()).await;
    let mut input = meshway_configuration::NetworkConfigurationInput::from(&current.configuration);
    input
        .nodes
        .get_mut(&node.node_id)
        .unwrap()
        .listeners
        .insert(
        "trojan".into(),
        serde_json::from_str(
            r#"{"type":"trojan","listenPort":24444,"tls":{"serverName":"trojan.example.test"}}"#,
        )
        .unwrap(),
    );
    let draft = ConfigurationRequest::Document {
        expected_revision: current.revision,
        request_id: request(),
        source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
    };
    let preview: ConfigurationPreviewResponse = json(
        c.post(net(&s, network, "configuration/validate"))
            .bearer_auth(owner.token.expose())
            .json(&draft),
    )
    .await;
    assert_eq!(preview.will_generate.len(), 3);
    assert!(
        preview
            .will_generate
            .iter()
            .any(|r| r.kind == PreparationKind::Material(MaterialRequirement::Password))
    );
    let _: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&draft),
    )
    .await;
    // Structural errors still fail even when some other listener needs automatic material.
    input.nodes.get_mut(&node.node_id).unwrap().hostname = "not a dns label".into();
    let current = source(&c, &s, network, owner.token.expose()).await;
    let response = c
        .post(net(&s, network, "configuration/validate"))
        .bearer_auth(owner.token.expose())
        .json(&ConfigurationRequest::Document {
            expected_revision: current.revision,
            request_id: request(),
            source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    s.close().await;
}
