//! Saved listener credentials and private keys never leave the Server: the
//! only document read shows them as `{"retained": true}`, and submitting that
//! marker keeps the saved value of the same listener.
use super::*;
use meshway_configuration::transport::{Listener, ListenerSecret};
use meshway_configuration::{ConfigurationOperation, NetworkConfigurationInput};
use meshway_protocol::configuration::*;

const UUID: &str = "11111111-1111-4111-8111-111111111111";

async fn apply(
    c: &Client,
    s: &Server,
    n: meshway_model::NetworkId,
    token: &str,
    expected: Revision,
    text: String,
) -> reqwest::Response {
    c.post(net(s, n, "configuration"))
        .bearer_auth(token)
        .json(&ConfigurationRequest::Document {
            expected_revision: expected,
            request_id: request(),
            source_jsonc: WireSecret::new(text),
        })
        .send()
        .await
        .unwrap()
}

async fn applied(
    c: &Client,
    s: &Server,
    n: meshway_model::NetworkId,
    token: &str,
    expected: Revision,
    text: String,
) -> Revision {
    let response = apply(c, s, n, token, expected, text).await;
    let status = response.status();
    let body = response.text().await.unwrap();
    assert!(status.is_success(), "{status}: {body}");
    serde_json::from_str::<ConfigurationAppliedResponse>(&body)
        .unwrap()
        .revision
}

async fn changes(
    c: &Client,
    s: &Server,
    n: meshway_model::NetworkId,
    token: &str,
    from: Revision,
    to: Revision,
) -> Vec<String> {
    let comparison: ConfigurationComparison = json(
        c.post(net(s, n, "configuration/compare"))
            .bearer_auth(token)
            .json(&CompareConfigurationRequest {
                from_revision: from,
                to_revision: to,
            }),
    )
    .await;
    comparison.changes.into_iter().map(|c| c.path).collect()
}

/// The invalid-transport reasons a rejected document reports.
async fn rejected(response: reqwest::Response) -> String {
    serde_json::to_string(
        &error(
            response,
            StatusCode::UNPROCESSABLE_ENTITY,
            "configuration_invalid",
        )
        .await
        .problem,
    )
    .unwrap()
}

fn vless(document: &mut NetworkConfigurationInput, node: meshway_model::NodeId) -> &mut Listener {
    document
        .nodes
        .get_mut(&node)
        .unwrap()
        .listeners
        .get_mut("vless")
        .unwrap()
}

#[tokio::test]
async fn source_reads_withhold_listener_secrets_and_the_marker_keeps_them() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "secrets@example.test", "valid password").await;
    let token = owner.token.expose();
    let n = create(&c, &s, token, "secrets").await.network.id;
    let node = super::super::live::enroll(&c, &s, n, token, 61)
        .await
        .0
        .node_id;
    let initial = source(&c, &s, n, token).await;
    let listener = serde_json::from_str(
        r#"{"type":"vless","listenPort":24443,"tls":{"serverName":"secrets.example.test"}}"#,
    )
    .unwrap();
    let generated: ConfigurationAppliedResponse =
        json(c.post(net(&s, n, "configuration")).bearer_auth(token).json(
            &ConfigurationRequest::Operations {
                expected_revision: initial.revision,
                request_id: request(),
                ops: vec![ConfigurationOperation::SetListener {
                    node_id: node,
                    key: "vless".into(),
                    listener: Some(listener),
                }],
            },
        ))
        .await;

    // Generated UUID and private key read as markers in both forms; the
    // public certificate is shown.
    let raw = c
        .get(net(&s, n, "configuration/source"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!raw.contains("PRIVATE KEY"), "{raw}");
    assert!(raw.contains("BEGIN CERTIFICATE"));
    let read: ConfigurationSourceResponse = serde_json::from_str(&raw).unwrap();
    assert_eq!(read.revision, generated.revision);
    let mut document = read.configuration.clone();
    let Listener::Vless(v) = vless(&mut document, node) else {
        panic!("VLESS")
    };
    assert_eq!(v.uuid, Some(ListenerSecret::Retained));
    assert_eq!(v.tls.private_key, Some(ListenerSecret::Retained));
    assert!(v.tls.certificate.is_some());

    // The redacted canonical document submitted as source keeps every saved value.
    let copied = applied(
        &c,
        &s,
        n,
        token,
        read.revision,
        serde_json::to_string(&read.configuration).unwrap(),
    )
    .await;
    assert_eq!(
        changes(&c, &s, n, token, read.revision, copied).await,
        Vec::<String>::new()
    );

    // An explicit value spelled out in the source is saved but read back as a
    // marker; comments survive, and the read text keeps the value too.
    let uuid_path = format!("/nodes/{node}/listeners/vless/uuid");
    let Listener::Vless(v) = vless(&mut document, node) else {
        unreachable!()
    };
    v.uuid = Some(meshway_configuration::transport::SecretString::new(UUID.into()).into());
    let explicit = applied(
        &c,
        &s,
        n,
        token,
        copied,
        format!(
            "// operator note\n{}",
            serde_json::to_string_pretty(&document).unwrap()
        ),
    )
    .await;
    assert_eq!(
        changes(&c, &s, n, token, copied, explicit).await,
        vec![uuid_path.clone()]
    );
    let text = source(&c, &s, n, token)
        .await
        .source_jsonc
        .expose()
        .to_owned();
    assert!(text.starts_with("// operator note\n"));
    assert!(!text.contains(UUID) && !text.contains("PRIVATE KEY"));
    assert!(text.contains(r#""uuid": {"retained":true}"#), "{text}");
    let resubmitted = applied(&c, &s, n, token, explicit, text).await;
    assert_eq!(
        changes(&c, &s, n, token, explicit, resubmitted).await,
        Vec::<String>::new()
    );
    let history = c
        .get(net(
            &s,
            n,
            &format!("configuration/source?revision={explicit}"),
        ))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!history.contains(UUID), "historical reads are redacted too");
    let history: ConfigurationSourceResponse = serde_json::from_str(&history).unwrap();
    assert_eq!(history.revision, explicit);

    // A new explicit value is still fully validated.
    let Listener::Vless(v) = vless(&mut document, node) else {
        unreachable!()
    };
    v.uuid = Some(meshway_configuration::transport::SecretString::new("not-a-uuid".into()).into());
    let reasons = rejected(
        apply(
            &c,
            &s,
            n,
            token,
            resubmitted,
            serde_json::to_string(&document).unwrap(),
        )
        .await,
    )
    .await;
    assert!(reasons.contains("invalid_vless_uuid"), "{reasons}");

    // A managed key is issued for one server name: a renamed listener has no
    // saved key the marker could keep, and the marker never means "generate".
    let mut renamed = read.configuration.clone();
    let Listener::Vless(v) = vless(&mut renamed, node) else {
        unreachable!()
    };
    v.tls.server_name = "renamed.example.test".into();
    v.tls.certificate = None;
    let reasons = rejected(
        apply(
            &c,
            &s,
            n,
            token,
            resubmitted,
            serde_json::to_string(&renamed).unwrap(),
        )
        .await,
    )
    .await;
    assert!(reasons.contains("retained_secret_unavailable"), "{reasons}");

    // Deleting the listener deletes its materials; a marker for the removed
    // listener does not resurrect them.
    let mut removed = read.configuration.clone();
    removed
        .nodes
        .get_mut(&node)
        .unwrap()
        .listeners
        .remove("vless");
    let deleted = applied(
        &c,
        &s,
        n,
        token,
        resubmitted,
        serde_json::to_string(&removed).unwrap(),
    )
    .await;
    assert!(
        !source(&c, &s, n, token).await.configuration.nodes[&node]
            .listeners
            .contains_key("vless")
    );
    let reasons = rejected(
        apply(
            &c,
            &s,
            n,
            token,
            deleted,
            serde_json::to_string(&read.configuration).unwrap(),
        )
        .await,
    )
    .await;
    assert!(reasons.contains("retained_secret_unavailable"), "{reasons}");
    s.close().await;
}
