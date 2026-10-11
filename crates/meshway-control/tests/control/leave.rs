use super::{
    faults::{Fault, Proxy},
    support::{Server, request},
};
use meshway_control::Control;
use meshway_test_server::client::GetNodePath;

fn open(path: &std::path::Path, origin: &str) -> Control {
    Control::open(path, origin, meshway_test_server::http()).unwrap()
}

#[tokio::test]
async fn leave_clears_the_identity_only_after_the_server_deleted_the_node() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = server
        .control(&owner, &proxy.origin, "leaving", Default::default())
        .await;
    crate::support::sync(&mut control).await.unwrap();
    let binding = control.status().binding.unwrap();
    let path = server.path("leaving");
    assert!(path.join("netmap.json").exists());

    // An unreachable or failing Server changes nothing local.
    proxy.fault("/leave", Fault::RejectBefore);
    assert!(control.leave(request()).await.is_err());
    assert_eq!(control.status().binding, Some(binding.clone()));
    assert!(path.join("netmap.json").exists());
    let get = GetNodePath {
        network_id: owner.network,
        node_id: binding.node_id,
    };
    server
        .client
        .get_node(&get, owner.token.expose())
        .await
        .unwrap();

    control.authenticate().await.unwrap();
    let left = control.leave(request()).await.unwrap();
    assert_eq!(
        (left.network_id, left.node_id),
        (binding.network_id, binding.node_id)
    );
    let status = control.status();
    assert!(status.binding.is_none() && status.held.is_none() && status.netmap.is_none());
    assert!(!status.authenticated);
    assert!(!path.join("netmap.json").exists());
    assert!(!path.join("peer-facts.json").exists());
    assert!(
        server
            .client
            .get_node(&get, owner.token.expose())
            .await
            .is_err()
    );
    // No identity is left to authenticate or to leave again.
    assert!(control.authenticate().await.is_err());
    assert!(control.leave(request()).await.is_err());
    control.shutdown().await.unwrap();

    // The directory reopens as an initialized Node without identity, ready
    // for a new enrollment.
    let mut reopened = open(&path, &proxy.origin);
    assert!(reopened.status().binding.is_none());
    assert!(reopened.status().held.is_none());
    assert!(reopened.begin_enrollment("rejoined").await.is_ok());
    reopened.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}
