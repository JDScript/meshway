//! Real admission, current configuration commits and Node bearer reads.
use super::{
    App,
    node::{enroll_user_node, fixture, session},
};
use crate::{
    accounts::{Secret, random_uuid},
    error::Error,
};
use meshway_configuration::*;
use meshway_model::*;
use meshway_protocol::{configuration::ConfigurationRequest, node::*};
use reqwest::{Client, StatusCode};

fn rid() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}

async fn edit(
    app: &App,
    owner: &str,
    network: NetworkId,
    ops: Vec<ConfigurationOperation>,
) -> Revision {
    let current = app
        .configurations()
        .read(owner, network, None)
        .await
        .unwrap();
    app.configurations()
        .apply(
            owner,
            network,
            ConfigurationRequest::Operations {
                expected_revision: current.revision,
                request_id: rid(),
                ops,
            },
            rid(),
        )
        .await
        .unwrap()
        .revision
}

async fn query(client: &Client, url: &str, token: &str, name: &str) -> NodeDnsQueryResponse {
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(&NodeDnsQueryRequest { name: name.into() })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let request_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let body: NodeDnsQueryResponse = response.json().await.unwrap();
    assert_eq!(body.request_id.to_string(), request_id);
    body
}

#[test]
fn discovery_openapi_has_node_security_and_a_single_required_name() {
    let spec = serde_json::to_value(crate::http::openapi()).unwrap();
    let route = &spec["paths"]["/agent-api/v1/dns/query"]["post"];
    assert_eq!(route["operationId"], "queryNodeDns");
    assert_eq!(route["security"], serde_json::json!([{"nodeSession":[]}]));
    let request = &spec["components"]["schemas"]["NodeDnsQueryRequest"];
    assert_eq!(request["required"], serde_json::json!(["name"]));
    assert_eq!(request["additionalProperties"], false);
    assert_eq!(request["properties"].as_object().unwrap().len(), 1);
    let response = &spec["components"]["schemas"]["NodeDnsQueryResponse"];
    assert!(
        response["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("record"))
    );
}

#[tokio::test]
async fn discovery_rechecks_lifecycle_without_a_new_configuration_revision() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, _, signer_a, _) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let signer = ed25519_dalek::SigningKey::from_bytes(&[91; 32]);
    let device = enroll_user_node(&app, owner.token.expose(), network, &signer, [92; 32]).await;
    let execution = app.execution();
    let source = session(&execution, network, a, &ak, &signer_a).await;
    let user_source = session(&execution, network, device.node_id, &device.keys, &signer).await;
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let name = |node| {
        node_dns_name(
            &current.document.configuration.nodes[&node].hostname,
            &current.document.configuration.dns.domain,
        )
        .unwrap()
    };
    let user_name = name(device.node_id);
    let before = execution
        .query_dns(
            source.token.expose(),
            NodeDnsQueryRequest {
                name: user_name.clone(),
            },
            rid(),
        )
        .await
        .unwrap();
    assert_eq!(before.record.unwrap().node_id, device.node_id);
    execution
        .authenticate(user_source.token.expose())
        .await
        .unwrap();
    let user = owner.session.user.id;
    // Exercise current lifecycle facts independently of historical configuration
    // bindings. Account disable has no public mutation endpoint.
    app.executor
        .handle()
        .write(move |tx| {
            let now = crate::store::execution::now()?;
            tx.execute(
                "UPDATE users SET disabled_at=?1 WHERE id=?2",
                rusqlite::params![now, user.as_uuid().as_bytes().as_slice()],
            )?;
            tx.execute(
                "UPDATE nodes SET credentials_revoked_at=?1 WHERE network_id=?2 AND id=?3",
                rusqlite::params![
                    now,
                    network.as_uuid().as_bytes().as_slice(),
                    b.as_uuid().as_bytes().as_slice()
                ],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    for hidden in [user_name.clone(), name(b)] {
        let after = execution
            .query_dns(
                source.token.expose(),
                NodeDnsQueryRequest { name: hidden },
                rid(),
            )
            .await
            .unwrap();
        assert_eq!(after.configuration_revision, before.configuration_revision);
        assert!(after.record.is_none());
    }
    assert!(matches!(
        execution
            .query_dns(
                user_source.token.expose(),
                NodeDnsQueryRequest { name: user_name },
                rid()
            )
            .await,
        Err(Error::InvalidCredentials)
    ));
    // The Tag device's identity does not inherit its creator's account status.
    assert_eq!(
        execution
            .query_dns(
                source.token.expose(),
                NodeDnsQueryRequest { name: name(a) },
                rid()
            )
            .await
            .unwrap()
            .record
            .unwrap()
            .node_id,
        a
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn discovery_http_uses_identity_access_and_current_atomic_configuration_without_instances() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let execution = app.execution();
    let source = session(&execution, network, a, &ak, &signer_a).await;
    let target = session(&execution, network, b, &bk, &signer_b).await;
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let mut dns = current.document.configuration.dns.clone();
    dns.domain = "BÜCHER.Example.".into();
    let a_name = node_dns_name("a", &dns.domain).unwrap();
    let b_name = node_dns_name("b", &dns.domain).unwrap();
    let revision = edit(
        &app,
        owner.token.expose(),
        network,
        vec![
            ConfigurationOperation::ReplaceDns { dns: dns.clone() },
            ConfigurationOperation::ReplaceAccess { access: vec![] },
            ConfigurationOperation::ReplaceTopology {
                topology: vec![TopologyRule {
                    transports: Input::Omitted,
                    remark: String::new(),
                    action: TopologyAction::Block,
                    a: "*".into(),
                    b: "*".into(),
                }],
            },
        ],
    )
    .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/agent-api/v1/dns/query",
        listener.local_addr().unwrap()
    );
    let router = crate::http::router(&app);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let client = Client::builder()
        .tls_backend_preconfigured(meshway_tls::platform_client_config().unwrap())
        .no_proxy()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();
    let own = query(&client, &url, source.token.expose(), "A.BÜCHER.Example.").await;
    assert_eq!(
        (
            own.network_id,
            own.source_node_id,
            own.configuration_revision
        ),
        (network, a, revision)
    );
    assert_eq!(own.record.as_ref().unwrap().node_id, a);
    assert_eq!(own.record.as_ref().unwrap().name, a_name);
    let mut forbidden = query(&client, &url, source.token.expose(), &b_name).await;
    let unknown = query(
        &client,
        &url,
        source.token.expose(),
        "missing.xn--bcher-kva.example",
    )
    .await;
    assert!(forbidden.record.is_none());
    // Only request correlation and the time of the read may differ.
    forbidden.request_id = unknown.request_id;
    forbidden.as_of = unknown.as_of;
    assert_eq!(forbidden, unknown);
    for name in [
        "b",
        "xn--bcher-kva.example",
        "b.xn--bcher-kva.example.evil",
        "extra.b.xn--bcher-kva.example",
    ] {
        assert!(
            query(&client, &url, source.token.expose(), name)
                .await
                .record
                .is_none()
        );
    }
    for body in [
        serde_json::json!({"name":b_name,"sourceNodeId":b}),
        serde_json::json!({"name":b_name,"networkId":network}),
        serde_json::json!({"name":b_name,"nodeId":b}),
        serde_json::json!({"name":"*.example"}),
        serde_json::json!({"name":"a..example"}),
        serde_json::json!({"name":"a.example.."}),
        serde_json::json!({"name":"é".repeat(MAX_DNS_NAME_INPUT_BYTES)}),
        serde_json::json!({"name":["a.example","b.example"]}),
    ] {
        let response = client
            .post(&url)
            .bearer_auth(source.token.expose())
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    for token in [owner.token.expose(), "mwns_invalid"] {
        let response = client
            .post(&url)
            .bearer_auth(token)
            .json(&NodeDnsQueryRequest {
                name: b_name.clone(),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let error: meshway_protocol::ErrorResponse = response.json().await.unwrap();
        assert!(matches!(
            error.problem,
            meshway_protocol::Problem::InvalidCredentials { .. }
        ));
    }

    let grant = AccessRule {
        remark: String::new(),
        src: vec![format!("node:{a}")],
        dst: vec![format!("node:{b}")],
        ip: vec!["tcp:65432".into()],
        bidirectional: false,
    };
    let revision = edit(
        &app,
        owner.token.expose(),
        network,
        vec![
            ConfigurationOperation::ReplaceAccess {
                access: vec![grant.clone()],
            },
            ConfigurationOperation::SetNodeSettings {
                node_id: a,
                fields: NodeSettingsPatch {
                    enabled: Input::Value(false),
                    ..Default::default()
                },
            },
            ConfigurationOperation::SetNodeSettings {
                node_id: b,
                fields: NodeSettingsPatch {
                    enabled: Input::Value(false),
                    hostname: Input::Value("renamed".into()),
                    ..Default::default()
                },
            },
            ConfigurationOperation::SetAddresses {
                node_id: b,
                addresses: NodeAddressesInput {
                    ipv4: Input::Value(None),
                    ipv6: Input::Omitted,
                },
            },
        ],
    )
    .await;
    let renamed = node_dns_name("renamed", &dns.domain).unwrap();
    let snapshot = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let found = query(&client, &url, source.token.expose(), &renamed).await;
    assert_eq!(found.configuration_revision, revision);
    assert_eq!(found.record.as_ref().unwrap().node_id, b);
    assert_eq!(
        found.record.as_ref().unwrap().addresses,
        snapshot.document.configuration.nodes[&b].addresses
    );
    assert!(found.record.unwrap().addresses.ipv4.is_none());
    assert!(
        query(&client, &url, source.token.expose(), &b_name)
            .await
            .record
            .is_none()
    );
    assert!(
        query(&client, &url, target.token.expose(), &a_name)
            .await
            .record
            .is_none()
    );
    assert!(
        query(&client, &url, source.token.expose(), &a_name)
            .await
            .record
            .is_some()
    );
    let mut reverse = grant;
    reverse.bidirectional = true;
    reverse.ip = vec!["icmp:*".into()];
    edit(
        &app,
        owner.token.expose(),
        network,
        vec![ConfigurationOperation::ReplaceAccess {
            access: vec![reverse],
        }],
    )
    .await;
    assert_eq!(
        query(&client, &url, target.token.expose(), &a_name)
            .await
            .record
            .unwrap()
            .node_id,
        a
    );

    dns.magic_dns = false;
    edit(
        &app,
        owner.token.expose(),
        network,
        vec![ConfigurationOperation::ReplaceDns { dns: dns.clone() }],
    )
    .await;
    assert!(
        query(&client, &url, source.token.expose(), &a_name)
            .await
            .record
            .is_none()
    );
    assert!(
        query(&client, &url, source.token.expose(), &renamed)
            .await
            .record
            .is_none()
    );
    dns.magic_dns = true;
    edit(
        &app,
        owner.token.expose(),
        network,
        vec![ConfigurationOperation::ReplaceDns { dns }],
    )
    .await;

    // Read authorization is fresh even after a previously successful check.
    execution.authenticate(source.token.expose()).await.unwrap();
    for (node, key) in [(b, &bk.control), (b, &bk.dataplane), (a, &ak.dataplane)] {
        execution
            .revoke_key(
                owner.token.expose(),
                network,
                node,
                RevokeNodeKeyRequest {
                    key_id: key.id,
                    expected_generation: key.generation,
                },
            )
            .await
            .unwrap();
    }
    let before = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let found = query(&client, &url, source.token.expose(), &renamed).await;
    assert_eq!(found.configuration_revision, before.revision);
    assert_eq!(found.record.unwrap().node_id, b);
    assert!(matches!(
        execution
            .query_dns(
                target.token.expose(),
                NodeDnsQueryRequest {
                    name: a_name.clone()
                },
                rid()
            )
            .await,
        Err(Error::InvalidCredentials)
    ));
    edit(
        &app,
        owner.token.expose(),
        network,
        vec![ConfigurationOperation::ReplaceAccess { access: vec![] }],
    )
    .await;
    assert!(
        query(&client, &url, source.token.expose(), &renamed)
            .await
            .record
            .is_none()
    );
    edit(
        &app,
        owner.token.expose(),
        network,
        vec![ConfigurationOperation::ReplaceAccess {
            access: vec![AccessRule {
                remark: String::new(),
                src: vec!["*".into()],
                dst: vec!["*".into()],
                ip: vec!["udp:40000-40009".into()],
                bidirectional: false,
            }],
        }],
    )
    .await;
    assert!(
        query(&client, &url, source.token.expose(), &renamed)
            .await
            .record
            .is_some()
    );
    edit(
        &app,
        owner.token.expose(),
        network,
        vec![ConfigurationOperation::DeleteNode { node_id: b }],
    )
    .await;
    assert!(
        query(&client, &url, source.token.expose(), &renamed)
            .await
            .record
            .is_none()
    );
    execution
        .revoke_key(
            owner.token.expose(),
            network,
            a,
            RevokeNodeKeyRequest {
                key_id: ak.control.id,
                expected_generation: ak.control.generation,
            },
        )
        .await
        .unwrap();
    let denied = client
        .post(&url)
        .bearer_auth(source.token.expose())
        .json(&NodeDnsQueryRequest { name: a_name })
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    // No lookup acquired an Instance, installed a demand, or extended a lease.
    app.executor
        .handle()
        .read(|tx| {
            let instances: i64 = tx.query_row(
                "SELECT count(*) FROM node_execution WHERE instance_id IS NOT NULL",
                [],
                |r| r.get(0),
            )?;
            let demands: i64 =
                tx.query_row("SELECT count(*) FROM execution_demands", [], |r| r.get(0))?;
            assert_eq!((instances, demands), (0, 0));
            Ok(())
        })
        .await
        .unwrap();
    app.stream_shutdown()();
    stop.send(()).unwrap();
    server.await.unwrap();
    app.shutdown().await.unwrap();
}
