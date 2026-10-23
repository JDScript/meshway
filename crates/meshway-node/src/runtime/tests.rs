//! The coordinator's own loop.
use super::*;
use crate::host::ControlOnly;
use meshway_protocol::ipc::NetworkMode;

/// F8a F2: the IPC service stopping drops every request sender. Its closed
/// channels are not ready again, so the coordinator reaches the listener's
/// join and stops with its failure instead of polling them forever.
#[tokio::test]
async fn a_stopped_ipc_service_ends_the_coordinator_with_its_failure() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("node");
    let settings = Settings::new(path, "http://127.0.0.1:9", Some("coordinator"))
        .unwrap()
        .with_network_mode(NetworkMode::ControlOnly);
    let node = Node::open(settings, Arc::new(ControlOnly)).await.unwrap();
    ipc::FAIL_SERVE.set(true);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        node.run(std::future::pending()),
    )
    .await
    .expect("the coordinator kept polling closed request channels");
    assert_eq!(result, Err("IPC service failed."));
}
