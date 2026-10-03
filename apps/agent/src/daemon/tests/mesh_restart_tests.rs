//! Normal lifecycle regression adapted from the independent frozen A1 probe.
//! Real controller and generated node HTTP; only the OS device boundary is mock.
use super::local;
use super::*;
use crate::managed_dataplane::{PacketPlan, TunOptions, tests::MockDevice, tun::Tun};
use meshway_protocol::*;
use std::os::unix::fs::PermissionsExt;

fn json_body(
    request: reqwest::RequestBuilder,
    body: &impl serde::Serialize,
) -> reqwest::RequestBuilder {
    request
        .header("content-type", "application/json")
        .body(serde_json::to_vec(body).unwrap())
}

async fn response<T: serde::de::DeserializeOwned>(request: reqwest::RequestBuilder) -> T {
    let bytes = request
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .bytes()
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn normal_mesh_tick_restarts_after_offline_revision_change_and_same_config() {
    let dir = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let controller_dir = dir.path().join("controller");
    std::fs::create_dir(&controller_dir).unwrap();
    std::fs::set_permissions(&controller_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let state = meshway_controller::AppState::open(&controller_dir).unwrap();
    let admin = meshway_controller::admin_token(&controller_dir).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, meshway_controller::router(state))
            .await
            .unwrap()
    });
    let http = reqwest::Client::new();
    let networks: NetworkList = response(
        http.get(format!("{base}/api/v1/networks"))
            .bearer_auth(&admin),
    )
    .await;
    let network = &networks.networks[0].network_id;
    let token: CreateEnrollmentTokenResponse = response(json_body(
        http.post(format!(
            "{base}/api/v1/networks/{network}/enrollment-tokens"
        ))
        .bearer_auth(&admin),
        &CreateEnrollmentTokenRequest {
            expires_in_seconds: 3600,
        },
    ))
    .await;
    let path = dir.path().join("agent.json");
    let mut agent =
        Agent::prepare_enrollment(&path, &base, &token.token, "restart-regression").unwrap();
    agent.enroll(&token.token).await.unwrap();
    agent.publish_connection_identity().await.unwrap();
    let node_http = HttpClient::new()
        .with_base_url(&base)
        .with_api_key(&agent.identity.node_credential);
    let mut policy = meshway_policy::NetworkPolicy::default_template();
    let revision = node_http
        .get_agent_configuration()
        .await
        .unwrap()
        .payload
        .revision;
    let source_url = format!("{base}/api/v1/networks/{network}/policy/save");
    json_body(
        http.post(&source_url).bearer_auth(&admin),
        &serde_json::json!({"expected_revision":revision,"policy":policy}),
    )
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap();
    let epoch = node_http.get_connection_runtime_epoch().await.unwrap();
    let runtime = RuntimeLease::new().unwrap();
    let begun = node_http
        .begin_connection_runtime(
            convert(BeginConnectionRuntimeRequest {
                expected_epoch: u32::try_from(epoch.epoch).unwrap(),
                instance_id: runtime.instance_id.clone(),
            })
            .unwrap(),
        )
        .await
        .unwrap();
    let execution: MeshExecutionResponse = convert(
        node_http
            .negotiate_mesh_execution(
                convert(MeshExecutionRequest {
                    epoch: u32::try_from(begun.epoch).unwrap(),
                    instance_id: runtime.instance_id,
                    capabilities: vec![MeshExecutionCapability::Ipv4StatefulExactPathV3],
                })
                .unwrap(),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let make_manager = || {
        let (device, send, receive) = MockDevice::new();
        let mut manager = Manager::new_dataplane(
            None,
            Duration::from_secs(30),
            TunOptions {
                name: "mock0".into(),
                mtu: 1000,
            },
            base.clone(),
            true,
        );
        manager.mock_tun(Tun::mock(
            device,
            &PacketPlan::sink(&execution.snapshot.payload, 1000).unwrap(),
        ));
        (manager, send, receive)
    };
    let (mut manager, send, receive) = make_manager();
    let mut cache = Some(dataplane::State::load(&mut agent).unwrap());
    let mut lease = RuntimeLease::new().unwrap();
    let mut synced = false;
    let first = agent
        .managed_tick(&mut manager, &mut lease, &mut synced, &mut cache)
        .await;
    assert!(matches!(
        first,
        Ok(()) | Err(AgentError::RuntimeObservationChanged)
    ));
    assert!(synced);
    let mut saved_revision = manager.snapshot.as_ref().unwrap().payload.revision;
    manager.withdraw(Reason::ShuttingDown).await;
    drop((manager, send, receive, agent, cache));
    for advance in [false, true] {
        if advance {
            policy.grants.clear();
            json_body(
                http.post(&source_url).bearer_auth(&admin),
                &serde_json::json!({"expected_revision":saved_revision,"policy":policy}),
            )
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
            assert!(
                node_http
                    .get_agent_configuration()
                    .await
                    .unwrap()
                    .payload
                    .revision
                    > i64::from(saved_revision)
            );
        }
        let mut agent = Agent::load(&path).unwrap();
        let (mut manager, send, receive) = make_manager();
        let mut cache = Some(dataplane::State::load(&mut agent).unwrap());
        let mut lease = RuntimeLease::new().unwrap();
        let mut synced = false;
        let result = agent
            .managed_tick(&mut manager, &mut lease, &mut synced, &mut cache)
            .await;
        assert!(
            matches!(result, Ok(()) | Err(AgentError::RuntimeObservationChanged)),
            "normal restart advance={advance}: {result:?}"
        );
        assert!(synced);
        let execution: serde_json::Value = serde_json::from_str(
            &local::read_private(&path.with_extension("json.mesh-execution.json")).unwrap(),
        )
        .unwrap();
        saved_revision = manager.snapshot.as_ref().unwrap().payload.revision;
        assert_eq!(
            execution["snapshot"]["payload"]["revision"].as_u64(),
            Some(u64::from(saved_revision))
        );
        assert!(saved_revision > u32::try_from(revision).unwrap());
        manager.withdraw(Reason::ShuttingDown).await;
        drop((manager, send, receive, agent, cache));
    }
    server.abort();
}
