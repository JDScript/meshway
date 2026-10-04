#![cfg(unix)]
mod authority;
mod dns;
mod faults;
mod recovery;
mod support;

use meshway_control::Control;
use meshway_control_client::{
    DeleteNodePath, GetConfigurationProgressPath, GetNetworkPath, GetNodePath, GetNodeStatePath,
};
use meshway_model::state::SystemState;
use meshway_protocol::{ApproveEnrollmentRequest, DeleteNodeRequest, state::VisibleReport};
use std::collections::BTreeMap;
use support::Server;

fn system() -> SystemState {
    SystemState {
        hostname: "control-fixture".into(),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        agent_version: env!("CARGO_PKG_VERSION").into(),
    }
}

#[tokio::test]
async fn real_enrollment_authentication_saved_state_stop_and_reacquire() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let path = server.path("node");
    let mut control =
        Control::open(&path, &server.origin, reqwest::Client::builder().no_proxy()).unwrap();
    let pending = control.begin_enrollment("control-fixture").await.unwrap();
    assert_eq!(
        control.poll_enrollment().await.unwrap().phase,
        meshway_protocol::enrollment::EnrollmentPhase::Pending
    );
    server
        .client
        .approve_device_authorization(
            &meshway_control_client::ApproveDeviceAuthorizationPath {
                enrollment_request_id: pending.id,
            },
            &ApproveEnrollmentRequest {
                network_id: owner.network,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    control.poll_enrollment().await.unwrap();
    let binding = control.redeem_enrollment().await.unwrap();
    control.authenticate().await.unwrap();
    let lease = control.acquire_instance(BTreeMap::new()).await.unwrap();
    assert!(control.status().authenticated);
    assert_eq!(
        control.acquire_instance(BTreeMap::new()).await.unwrap(),
        lease
    );
    let target = control.sync_configuration().await.unwrap().unwrap();
    assert_eq!(target, control.status().cached.unwrap());
    control.acknowledge_saved().await.unwrap();
    control.acknowledge_saved().await.unwrap();
    control.report_control_state(system()).await.unwrap();
    let state = server
        .client
        .get_node_state(
            &GetNodeStatePath {
                network_id: binding.network_id,
                node_id: binding.node_id,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    let Some(VisibleReport::Full { report }) = &state.nodes[&binding.node_id].report else {
        panic!("expected own full report")
    };
    assert_eq!(report.configuration.saved, Some(target.clone()));
    assert_eq!(report.configuration.applied, None);
    assert!(report.capabilities.is_empty() && report.links.is_empty() && report.paths.is_empty());
    let progress = server
        .client
        .get_configuration_progress(
            &GetConfigurationProgressPath {
                network_id: binding.network_id,
                revision: target.revision,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    assert_eq!(progress.summary.applied, 0);
    assert!(
        progress
            .nodes
            .values()
            .all(|node| node.phase != meshway_protocol::live::ApplicationPhase::Applied)
    );
    let renewed = control.renew_instance().await.unwrap();
    assert_eq!(renewed.instance_id, lease.instance_id);
    control.stop().await.unwrap();
    assert!(!control.status().authenticated && control.status().instance.is_none());
    assert!(control.report_control_state(system()).await.is_err());
    // A parked driver reuses its still-valid in-memory proof session, avoiding
    // exhaustion of the Server's bounded session slots on repeated up/down.
    for _ in 0..10 {
        control.authenticate().await.unwrap();
        control.acquire_instance(BTreeMap::new()).await.unwrap();
        control.stop().await.unwrap();
    }
    control.shutdown().await.unwrap();
    let mut restored =
        Control::open(&path, &server.origin, reqwest::Client::builder().no_proxy()).unwrap();
    assert!(!restored.status().authenticated && restored.status().instance.is_none());
    assert_eq!(restored.status().binding.unwrap(), binding);
    assert_eq!(restored.status().cached, Some(target));
    restored.authenticate().await.unwrap();
    let fresh = restored.acquire_instance(BTreeMap::new()).await.unwrap();
    assert_ne!(fresh.instance_id, lease.instance_id);
    assert!(fresh.generation > lease.generation);
    restored.sync_configuration().await.unwrap();
    restored.acknowledge_saved().await.unwrap();
    let current = server
        .client
        .get_network(
            &GetNetworkPath {
                network_id: owner.network,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    server
        .client
        .delete_node(
            &DeleteNodePath {
                network_id: owner.network,
                node_id: binding.node_id,
            },
            &DeleteNodeRequest {
                request_id: support::request(),
                expected_revision: current.network.current_revision,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    assert!(restored.renew_instance().await.is_err());
    assert!(!restored.status().authenticated && restored.status().instance.is_none());
    assert!(restored.authenticate().await.is_err());
    assert!(
        server
            .client
            .get_node(
                &GetNodePath {
                    network_id: owner.network,
                    node_id: binding.node_id
                },
                owner.token.expose()
            )
            .await
            .is_err()
    );
    restored.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn pending_enrollment_restarts_with_identical_keys_and_device_cancels_it() {
    let server = Server::start().await;
    let path = server.path("pending");
    let mut control =
        Control::open(&path, &server.origin, reqwest::Client::builder().no_proxy()).unwrap();
    assert!(control.begin_enrollment("bad.name").await.is_err());
    let first = control.begin_enrollment("pending-node").await.unwrap();
    control.shutdown().await.unwrap();
    let mut control =
        Control::open(&path, &server.origin, reqwest::Client::builder().no_proxy()).unwrap();
    assert_eq!(
        control.begin_enrollment("PENDING-NODE").await.unwrap(),
        first
    );
    control.cancel_enrollment().await.unwrap();
    let second = control.begin_enrollment("pending-node").await.unwrap();
    assert_ne!(second.id, first.id);
    assert_ne!(second.control_key, first.control_key);
    let owner = server.owner().await;
    server
        .client
        .approve_device_authorization(
            &meshway_control_client::ApproveDeviceAuthorizationPath {
                enrollment_request_id: second.id,
            },
            &ApproveEnrollmentRequest {
                network_id: owner.network,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    control.poll_enrollment().await.unwrap();
    server
        .client
        .deny_device_authorization(
            &meshway_control_client::DenyDeviceAuthorizationPath {
                enrollment_request_id: second.id,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    assert_eq!(
        control.poll_enrollment().await.unwrap().phase,
        meshway_protocol::enrollment::EnrollmentPhase::Withdrawn
    );
    control.cancel_enrollment().await.unwrap();
    assert!(control.status().enrollment_request_id.is_none());
    control.shutdown().await.unwrap();
    server.close().await;
}
