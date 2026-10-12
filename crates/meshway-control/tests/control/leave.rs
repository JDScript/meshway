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
