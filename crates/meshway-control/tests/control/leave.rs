use super::{
    faults::{Fault, Proxy},
    support::{Owner, Server, request},
};
use meshway_control::{Control, Error, Left};
use meshway_test_server::client::GetNodePath;

fn open(path: &std::path::Path, origin: &str) -> Control {
    Control::open(path, origin, meshway_test_server::http()).unwrap()
}

async fn exists(server: &Server, owner: &Owner, path: &GetNodePath) -> bool {
    server
        .client
        .get_node(path, owner.token.expose())
        .await
        .is_ok()
}

fn forgotten(control: &Control, path: &std::path::Path) {
    let status = control.status();
    assert!(status.binding.is_none() && status.held.is_none() && status.netmap.is_none());
    assert!(!status.authenticated && status.leaving.is_none());
    assert!(!path.join("netmap.json").exists());
    assert!(!path.join("peer-facts.json").exists());
}

#[tokio::test]
async fn an_unanswered_leave_stays_pending_across_reopening_until_confirmed() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = server
        .control(&owner, &proxy.origin, "leaving", Default::default())
        .await;
    crate::support::sync(&mut control).await.unwrap();
    let binding = control.status().binding.unwrap();
    let path = server.path("leaving");
    let get = GetNodePath {
        network_id: owner.network,
        node_id: binding.node_id,
    };

    // No Server answer: the identity and the intent both stay.
    let first = request();
    proxy.fault("/leave", Fault::Unavailable);
    assert!(control.leave(first).await.is_err());
    assert_eq!(control.status().binding, Some(binding.clone()));
    assert_eq!(control.status().leaving, Some(first));
    assert!(exists(&server, &owner, &get).await);
    // Nothing rejoins while the leave is pending.
    assert!(matches!(control.authenticate().await, Err(Error::Pending)));
    assert!(matches!(control.rotate_keys().await, Err(Error::Pending)));
    control.shutdown().await.unwrap();

    // The intent survives reopening; the next attempt keeps its request ID.
    let mut control = open(&path, &proxy.origin);
    assert_eq!(control.status().leaving, Some(first));
    let Left::Confirmed(left) = control.leave(request()).await.unwrap() else {
        panic!("the Server confirms the pending leave")
    };
    assert_eq!(
        (left.network_id, left.node_id),
        (binding.network_id, binding.node_id)
    );
    forgotten(&control, &path);
    assert!(!exists(&server, &owner, &get).await);
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

/// N7: a Node the Server confirmed leaving binds to no Server: it opens
/// with another Server URL and enrolls there. A leave its Server has not
/// confirmed still belongs to that Server, and the refusal names both
/// Servers and the way out.
#[tokio::test]
async fn a_left_node_moves_to_another_server_and_a_leaving_one_names_its_server() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    // The proxy and the Server itself are two Server URLs.
    let (old, new) = (&proxy.origin, &server.origin);
    assert_ne!(
        meshway_control::server_origin(old).unwrap(),
        meshway_control::server_origin(new).unwrap()
    );

    let mut control = server
        .control(&owner, old, "moving", Default::default())
        .await;
    let path = server.path("moving");
    assert!(matches!(
        control.leave(request()).await.unwrap(),
        Left::Confirmed(_)
    ));
    control.shutdown().await.unwrap();
    let mut moved = Control::open(&path, new, meshway_test_server::http())
        .expect("a Node that left binds to no Server");
    forgotten(&moved, &path);
    assert!(moved.begin_enrollment("moved").await.is_ok());
    moved.shutdown().await.unwrap();
    // Its pending enrollment now belongs to the new Server.
    let refused = Control::open(&path, old, meshway_test_server::http())
        .err()
        .expect("a pending enrollment stays with its Server");
    let reason = refused.to_string();
    assert!(reason.contains("pending enrollment") && reason.contains("meshway down"));

    let mut control = server
        .control(&owner, old, "unconfirmed", Default::default())
        .await;
    let path = server.path("unconfirmed");
    proxy.fault("/leave", Fault::Unavailable);
    assert!(control.leave(request()).await.is_err());
    control.shutdown().await.unwrap();
    let saved = std::fs::read(path.join("control.json")).unwrap();
    let refused = Control::open(&path, new, meshway_test_server::http())
        .err()
        .expect("an unconfirmed leave stays with its Server");
    let reason = refused.to_string();
    for part in [
        "is leaving",
        &meshway_control::server_origin(old).unwrap(),
        &meshway_control::server_origin(new).unwrap(),
    ] {
        assert!(reason.contains(part), "{reason}");
    }
    assert_eq!(std::fs::read(path.join("control.json")).unwrap(), saved);
    // Back on its own Server the leave completes; then it may move.
    let mut control = open(&path, old);
    assert!(matches!(
        control.leave(request()).await.unwrap(),
        Left::Confirmed(_)
    ));
    control.shutdown().await.unwrap();
    open(&path, new).shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn a_lost_leave_answer_completes_on_the_identity_rejection() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = server
        .control(&owner, &proxy.origin, "lost", Default::default())
        .await;
    crate::support::sync(&mut control).await.unwrap();
    let binding = control.status().binding.unwrap();
    let path = server.path("lost");
    let get = GetNodePath {
        network_id: owner.network,
        node_id: binding.node_id,
    };

    // The Server deleted the Node; its answer never arrived.
    proxy.fault("/leave", Fault::Lost);
    assert!(control.leave(request()).await.is_err());
    assert!(!exists(&server, &owner, &get).await);
    assert!(control.status().binding.is_some() && control.status().leaving.is_some());

    // The retry's old session gets 401, and the fresh proof the identity
    // rejection that completes the leave.
    let challenges = proxy.count("/auth/challenges");
    assert!(matches!(
        control.leave(request()).await.unwrap(),
        Left::IdentityRejected
    ));
    assert_eq!(proxy.count("/auth/challenges"), challenges + 1);
    forgotten(&control, &path);
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn an_expired_session_reauthenticates_once_and_a_new_enrollment_replaces_the_intent() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = server
        .control(&owner, &proxy.origin, "expired", Default::default())
        .await;
    crate::support::sync(&mut control).await.unwrap();
    let binding = control.status().binding.unwrap();
    let get = GetNodePath {
        network_id: owner.network,
        node_id: binding.node_id,
    };
    // A 401 for a live identity: a fresh session, then the same leave.
    proxy.fault("/leave", Fault::RejectBefore);
    assert!(matches!(
        control.leave(request()).await.unwrap(),
        Left::Confirmed(_)
    ));
    assert_eq!(proxy.count("/leave"), 2);
    assert!(!exists(&server, &owner, &get).await);
    control.shutdown().await.unwrap();

    let mut control = server
        .control(&owner, &proxy.origin, "replaced", Default::default())
        .await;
    let binding = control.status().binding.unwrap();
    let get = GetNodePath {
        network_id: owner.network,
        node_id: binding.node_id,
    };
    proxy.fault("/leave", Fault::Unavailable);
    assert!(control.leave(request()).await.is_err());
    // Only an explicit new enrollment drops the pending intent unanswered.
    control.abandon_leave().await.unwrap();
    forgotten(&control, &server.path("replaced"));
    assert!(
        exists(&server, &owner, &get).await,
        "the Server never answered"
    );
    assert!(matches!(
        control.abandon_leave().await,
        Err(Error::Lifecycle)
    ));
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}
