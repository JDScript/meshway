//! Real Controller and protected Agent startup in one disposable Linux namespace.
use super::*;
use meshway_protocol::{CreateEnrollmentTokenRequest, CreateEnrollmentTokenResponse, NetworkList};
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
#[ignore = "requires disposable Linux namespace, NET_ADMIN and audit utilities"]
async fn controller_bootstrap_recovers_identity_and_disabled_scope() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
        Ok("1")
    );
    let directory = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let controller_dir = directory.path().join("controller");
    local::private_dir(&controller_dir).unwrap();
    let state = meshway_controller::AppState::open(&controller_dir).unwrap();
    let admin = meshway_controller::admin_token(&controller_dir).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, meshway_controller::router(state))
            .await
            .unwrap();
    });
    let admin_http = reqwest::Client::new();
    let networks: NetworkList = serde_json::from_slice(
        &admin_http
            .get(format!("{origin}/api/v1/networks"))
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
    )
    .unwrap();
    let network = &networks.networks[0].network_id;
    let token: CreateEnrollmentTokenResponse = serde_json::from_slice(
        &admin_http
            .post(format!(
                "{origin}/api/v1/networks/{network}/enrollment-tokens"
            ))
            .bearer_auth(&admin)
            .header("content-type", "application/json")
            .body(
                serde_json::to_vec(&CreateEnrollmentTokenRequest {
                    expires_in_seconds: 300,
                })
                .unwrap(),
            )
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
    )
    .unwrap();
    let path = directory.path().join("agent.json");
    let mut agent =
        Agent::prepare_enrollment(&path, &origin, &token.token, "protected-startup").unwrap();
    agent.protect_underlay = true;
    let case = std::env::var("MESHWAY_UNDERLAY_CASE").unwrap_or_else(|_| "pending".into());
    let mut expected_node = None;
    if matches!(case.as_str(), "lost" | "old") {
        let owner_dir = sidecar(&path, ".underlay");
        local::private_dir(&owner_dir).unwrap();
        let print = fingerprint(&agent.identity).unwrap();
        let owner = BootstrapUnderlay::prepare(BootstrapPreparation {
            state_directory: &owner_dir,
            fingerprint: &print,
        })
        .await
        .unwrap();
        let mut channel = owner
            .channel(reqwest::Url::parse(&origin).unwrap())
            .await
            .unwrap();
        let response = channel
            .enroll(
                &token.token,
                EnrollAgentRequest {
                    name: agent.identity.name.clone(),
                    agent_version: agent.identity.enrollment_version.clone(),
                    node_credential: agent.identity.node_credential.clone(),
                },
            )
            .await
            .unwrap();
        expected_node = Some(response.node.id.clone());
        if case == "old" {
            agent.accept(response).unwrap();
        }
        channel.retire().await;
        drop(owner);
        assert!(!sidecar(&path, ".network-binding.json").exists());
        assert!(agent.cache.is_none());
        agent.heartbeat().await.unwrap();
    } else if case == "binding-failure" {
        let binding_path = sidecar(&path, ".network-binding.json");
        let fault =
            crate::test_local::precommit_faults::ParentUnavailable::during_write(&binding_path);
        assert!(agent.enroll(&token.token).await.is_err());
        drop(fault);
        assert!(agent.identity.node_id.is_some());
        assert!(!binding_path.exists());
        assert!(!sidecar(&path, ".underlay").join("underlay.json").exists());
        agent.heartbeat().await.unwrap();
    } else {
        agent.enroll(&token.token).await.unwrap();
    }
    let binding = load_binding(&agent).unwrap().unwrap();
    assert_eq!(&binding.network_id, network);
    assert_eq!(Some(&binding.node_id), agent.identity.node_id.as_ref());
    if let Some(expected_node) = expected_node {
        assert_eq!(binding.node_id, expected_node);
    }
    let dns: meshway_protocol::AgentDnsSnapshot = connections::convert(
        agent
            .controller_client()
            .with_api_key(&agent.identity.node_credential)
            .get_agent_dns()
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(!dns.payload.dns.enabled);
    assert_eq!(dns.payload.network_id, binding.network_id);
    assert_eq!(dns.payload.node_id, binding.node_id);
    let owner_dir = sidecar(&path, ".underlay");
    assert!(!owner_dir.join("bootstrap.json").exists());
    let journal: serde_json::Value =
        serde_json::from_str(&local::read_private(&owner_dir.join("underlay.json")).unwrap())
            .unwrap();
    assert_eq!(journal["version"], 3);
    assert_eq!(journal["network"], binding.network_id);
    assert_eq!(journal["installed"].as_array().unwrap().len(), 4);
    agent.sync().await.unwrap();
    let retired = agent
        .controller_client()
        .with_api_key(&agent.identity.node_credential);
    agent.close_control_plane().await;
    drop(agent);
    assert!(retired.get_agent_dns().await.is_err());
    drop(retired);
    let mut agent = Agent::load(&path).unwrap();
    agent.protect_underlay = true;
    agent.heartbeat().await.unwrap();
    assert_eq!(
        agent.identity.node_id.as_deref(),
        Some(binding.node_id.as_str())
    );
    assert_eq!(
        load_binding(&agent).unwrap().unwrap().network_id,
        binding.network_id
    );
    drop(agent);
    server.abort();
    let _ = server.await;
}
