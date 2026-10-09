//! Real Server/Control contracts. Capability samples are supplied observations,
//! not claims of a running kernel backend.
use super::{
    faults::{Fault, Proxy},
    support::{Owner, Server, request},
    system,
};
use meshway_control::{Control, Error};
use meshway_control_client::{
    ApplyConfigurationPath, GetConfigurationSourcePath, GetConfigurationSourceQuery,
    GetNodeStatePath,
};
use meshway_model::{
    exit::{
        EXIT_IPV4_FEATURE, EXIT_IPV4_VERSION, ExitCapability, ExitStatus, ExitUnavailableReason,
    },
    state::{Capability, ConfigurationState, NodeState},
};
use meshway_protocol::{ConfigurationRequest, WireSecret, state::VisibleReport};
use std::collections::BTreeMap;

async fn enable(server: &Server, owner: &Owner) {
    let source = server
        .client
        .get_configuration_source(
            &GetConfigurationSourcePath {
                network_id: owner.network,
            },
            &GetConfigurationSourceQuery::default(),
            owner.token.expose(),
        )
        .await
        .unwrap();
    let mut input = meshway_configuration::NetworkConfigurationInput::from(&source.configuration);
    for node in input.nodes.values_mut() {
        node.exit_enabled = true;
    }
    server
        .client
        .apply_configuration(
            &ApplyConfigurationPath {
                network_id: owner.network,
            },
            &ConfigurationRequest::Document {
                expected_revision: source.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
}

fn facts(control: &Control) -> NodeState {
    let target = control.status().cached.unwrap();
    NodeState {
        system: system(),
        capabilities: control
            .cached_target()
            .unwrap()
            .configuration
            .self_keys
            .capabilities
            .clone(),
        configuration: ConfigurationState {
            saved: Some(target.clone()),
            applied: None,
            progress: None,
        },
        listeners: BTreeMap::new(),
        sockets: BTreeMap::new(),
        links: BTreeMap::new(),
        paths: BTreeMap::new(),
        selection: Default::default(),
        probes: Vec::new(),
        problems: Vec::new(),
        exit_capability: Some(ExitCapability {
            instance_id: control.status().instance.unwrap().instance_id,
            target,
            supported_families: Vec::new(),
            installed_families: Vec::new(),
            status: ExitStatus::Unavailable {
                reason: ExitUnavailableReason::IpForwardDisabled,
            },
            observed_at: chrono::Utc::now(),
        }),
    }
}

fn capabilities() -> BTreeMap<String, Capability> {
    BTreeMap::from([(
        EXIT_IPV4_FEATURE.into(),
        Capability::Feature {
            version: EXIT_IPV4_VERSION,
        },
    )])
}

#[tokio::test]
async fn exit_role_from_another_instance_cannot_be_saved_or_issued_even_with_valid_digest() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = server
        .control(&owner, &proxy.origin, "exit-role-binding", capabilities())
        .await;
    enable(&server, &owner).await;
    proxy.fault("/configuration/refresh", Fault::WrongExitRoleInstance);
    assert!(matches!(
        control.refresh_configuration().await,
        Err(Error::Contract)
    ));
    assert!(control.cached_target().is_none());
    assert!(control.execution_snapshot().is_err());
    assert!(control.status().instance.is_none());
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn exit_failure_reports_before_applied_and_retry_preserves_original_observation() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = server
        .control(&owner, &proxy.origin, "exit-facts", capabilities())
        .await;
    enable(&server, &owner).await;
    control.refresh_configuration().await.unwrap();
    control.acknowledge_saved().await.unwrap();
    let snapshot = control.execution_snapshot().unwrap();
    assert_eq!(
        snapshot
            .configuration()
            .unwrap()
            .exit_role
            .as_ref()
            .unwrap()
            .instance_id,
        control.status().instance.unwrap().instance_id
    );
    let state = facts(&control);
    proxy.fault("/agent-api/v1/state", Fault::Lost);
    assert!(
        control
            .report_runtime_state(state.clone(), chrono::Utc::now())
            .await
            .is_err()
    );
    let accepted = control.retry_state_report().await.unwrap();
    assert!(accepted.duplicate);
    let node = control.status().binding.unwrap().node_id;
    let view = server
        .client
        .get_node_state(
            &GetNodeStatePath {
                network_id: owner.network,
                node_id: node,
            },
            owner.token.expose(),
        )
        .await
        .unwrap()
        .nodes
        .remove(&node)
        .unwrap();
    assert_eq!(
        view.report,
        Some(VisibleReport::Full {
            report: Box::new(state)
        })
    );
    snapshot.ensure_current().unwrap();
    control.stop().await.unwrap();
    assert!(snapshot.ensure_current().is_err());
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn exit_observation_wrong_binding_is_rejected_before_http_without_fencing() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = server
        .control(&owner, &proxy.origin, "exit-binding", capabilities())
        .await;
    enable(&server, &owner).await;
    control.refresh_configuration().await.unwrap();
    control.acknowledge_saved().await.unwrap();
    let snapshot = control.execution_snapshot().unwrap();
    let calls = proxy.count("/agent-api/v1/state");
    for wrong_instance in [true, false] {
        let mut state = facts(&control);
        let exit = state.exit_capability.as_mut().unwrap();
        if wrong_instance {
            exit.instance_id = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        } else {
            exit.target.digest = [9; 32].into();
        }
        assert!(matches!(
            control
                .report_runtime_state(state, chrono::Utc::now())
                .await,
            Err(Error::Contract)
        ));
        assert_eq!(proxy.count("/agent-api/v1/state"), calls);
        snapshot.ensure_current().unwrap();
    }
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}
