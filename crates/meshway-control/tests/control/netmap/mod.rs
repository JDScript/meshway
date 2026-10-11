//! Netmap authority through the sync long poll. Session, instance and the
//! enrollment come from the real Server; sync replies are scripted until the
//! Server serves the route.
use super::{
    Control, Server,
    faults::{Fault, Proxy},
};
use meshway_control::{Error, NetmapChange, sync_loop};
use meshway_model::{Digest, Sequence};
use meshway_protocol::{EnrollmentResponse, PeerFactsDelta, StateCursor, SyncNetmap};
use meshway_test_server::client::ClientError;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use support::{Facts, Script, compiled, full, lease, signed, unchanged};

mod restart;
mod review;
mod server_key;
mod signature;
pub mod support;

const HOUR: u64 = 60 * 60 * 1000;

/// Enrolled through `proxy`, which pins the fixture Server signing key.
pub async fn enrolled(
    server: &Server,
    proxy: &Proxy,
    name: &str,
) -> (Control, EnrollmentResponse, Arc<Script>) {
    let owner = server.owner().await;
    let mut control = Control::open(
        &server.path(name),
        &proxy.origin,
        meshway_test_server::client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    proxy.fault("/redeem", Fault::ServerKey);
    let binding = server.enroll(&owner, &mut control, name).await;
    assert_eq!(binding.server_signing_key, support::server_key().1);
    control.authenticate().await.unwrap();
    let script = Script::new();
    control.set_sync_transport(script.clone());
    (control, binding, script)
}

#[tokio::test]
async fn server_stop_expires_session_and_instance_while_the_netmap_stays_published() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "offline").await;
    proxy.fault("/auth/session", Fault::ShortSession);
    control.refresh_authentication().await.unwrap();
    proxy.fault("/instances/acquire", Fault::ShortInstance);
    control.acquire_instance(BTreeMap::new()).await.unwrap();
    let netmap = compiled(&binding, "offline", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    let outcome = control.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(outcome.change, NetmapChange::New { previous: None });
    let published = outcome.netmap;
    assert_eq!(published.content.reference(), netmap.reference());
    assert_eq!(script.requests()[0].netmap, None);
    proxy.close().await;
    server.close().await;
    tokio::time::sleep(Duration::from_millis(2_300)).await;
    let status = control.status();
    assert!(!status.authenticated && status.instance.is_none());
    assert!(control.renew_instance().await.is_err());
    let current = control.netmap().unwrap();
    assert_eq!(current.lease, published.lease);
    assert_eq!(status.netmap.unwrap().netmap, netmap.reference());
    assert!(
        current.lease.expires_at_ms
            > u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap() + HOUR / 2
    );
    // The release cannot reach the stopped Server; the lease is still withdrawn.
    assert!(control.shutdown().await.is_err());
}

#[tokio::test]
async fn same_content_replaces_only_the_lease_and_a_newer_serial_replaces_content() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "renewal").await;
    let first = compiled(&binding, "renewal", None);
    script.push(Ok(full(&binding, &first, 60_000)));
    let old = control.sync(&mut Facts::default()).await.unwrap().netmap;
    tokio::time::sleep(Duration::from_millis(5)).await;
    script.push(Ok(unchanged(&binding, &first, HOUR)));
    let renewed = control.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(renewed.change, NetmapChange::Renewed);
    assert!(Arc::ptr_eq(&renewed.netmap.content, &old.content));
    assert!(renewed.netmap.lease.expires_at_ms > old.lease.expires_at_ms + HOUR / 2);
    assert_eq!(script.requests()[1].netmap, Some(first.reference()));

    let second = compiled(&binding, "renewal-two", Some(&first));
    let delta = meshway_configuration::netmap::netmap_delta(&first, &second).unwrap();
    script.push(Ok(signed(
        &binding,
        lease(second.reference(), HOUR),
        SyncNetmap::Delta {
            delta: Box::new(delta),
        },
    )));
    let next = control.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(
        next.change,
        NetmapChange::New {
            previous: Some(first.reference())
        }
    );
    assert_eq!(next.netmap.content.reference(), second.reference());
    assert_eq!(next.netmap.lease.netmap, second.reference());
    assert_eq!(control.netmap_reference(), Some(second.reference()));
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn serial_rollback_foreign_content_and_wrong_delta_digest_are_rejected() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "reject").await;
    let first = compiled(&binding, "reject", None);
    let second = compiled(&binding, "reject-two", Some(&first));
    script.push(Ok(full(&binding, &second, HOUR)));
    let current = control.sync(&mut Facts::default()).await.unwrap().netmap;

    // An older serial is never installed over a newer one.
    script.push(Ok(full(&binding, &first, HOUR)));
    assert!(matches!(
        control.sync(&mut Facts::default()).await,
        Err(Error::Contract)
    ));
    // A delta whose reconstruction does not reproduce its target digest.
    let third = compiled(&binding, "reject-three", Some(&second));
    let mut delta = meshway_configuration::netmap::netmap_delta(&second, &third).unwrap();
    delta.target.digest = Digest::from_bytes([7; 32]);
    script.push(Ok(signed(
        &binding,
        lease(delta.target, HOUR),
        SyncNetmap::Delta {
            delta: Box::new(delta),
        },
    )));
    assert!(matches!(
        control.sync(&mut Facts::default()).await,
        Err(Error::Contract)
    ));
    // A delta from a base the request did not name.
    let delta = meshway_configuration::netmap::netmap_delta(&first, &third).unwrap();
    script.push(Ok(signed(
        &binding,
        lease(third.reference(), HOUR),
        SyncNetmap::Delta {
            delta: Box::new(delta),
        },
    )));
    assert!(matches!(
        control.sync(&mut Facts::default()).await,
        Err(Error::Contract)
    ));
    // A lease for other content than the response carries.
    let mut response = full(&binding, &third, HOUR);
    response.lease = lease(second.reference(), HOUR);
    script.push(Ok(response));
    assert!(matches!(
        control.sync(&mut Facts::default()).await,
        Err(Error::Contract)
    ));
    // Another Node's netmap with valid digest.
    let mut other = binding.clone();
    other.node_id = super::support::request().to_string().parse().unwrap();
    script.push(Ok(full(
        &binding,
        &compiled(&other, "other", Some(&second)),
        HOUR,
    )));
    assert!(matches!(
        control.sync(&mut Facts::default()).await,
        Err(Error::Contract)
    ));
    assert_eq!(control.netmap().unwrap().lease, current.lease);
    assert_eq!(control.netmap_reference(), Some(second.reference()));
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn superseded_replies_transport_failures_and_identity_rejection() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "replies").await;
    let netmap = compiled(&binding, "replies", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    control.sync(&mut Facts::default()).await.unwrap();

    // Two prepared calls name the same base; once one is applied the other
    // was replaced and changes nothing, even though its body is valid.
    script.push(Ok(unchanged(&binding, &netmap, HOUR)));
    script.push(Ok(unchanged(&binding, &netmap, 2 * HOUR)));
    let (first, second) = (control.begin_sync().unwrap(), control.begin_sync().unwrap());
    let (first, second) = (first.send().await, second.send().await);
    control
        .finish_sync(second, &mut Facts::default())
        .await
        .unwrap();
    let renewed = control.netmap_lease().unwrap();
    assert!(matches!(
        control.finish_sync(first, &mut Facts::default()).await,
        Err(Error::Pending)
    ));
    assert_eq!(control.netmap_lease(), Some(renewed));

    // Unreachable Server and 5xx keep authority.
    assert!(matches!(
        control.sync(&mut Facts::default()).await,
        Err(Error::Server(ClientError::Network { status: None }))
    ));
    script.push(Err(ClientError::Http {
        status: 503,
        problem: None,
    }));
    assert!(control.sync(&mut Facts::default()).await.is_err());
    assert_eq!(control.netmap_lease(), Some(renewed));
    // 401 ends only the HTTP session.
    script.push(Err(ClientError::Http {
        status: 401,
        problem: None,
    }));
    assert!(control.sync(&mut Facts::default()).await.is_err());
    assert!(!control.status().authenticated);
    assert!(matches!(control.begin_sync(), Err(Error::Lifecycle)));
    assert_eq!(control.netmap_lease(), Some(renewed));
    control.authenticate().await.unwrap();
    // A decoded identity rejection ends authority at once.
    script.push(Err(review::identity_rejection()));
    assert!(control.sync(&mut Facts::default()).await.is_err());
    assert!(control.netmap().is_none() && control.netmap_lease().is_none());
    assert_eq!(control.netmap_reference(), Some(netmap.reference()));
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn peer_facts_follow_the_installed_netmap_and_their_cursor() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "facts").await;
    let netmap = compiled(&binding, "facts", None);
    let epoch = "00000000-0000-4000-8000-0000000000e1".parse().unwrap();
    let cursor = |sequence| StateCursor {
        epoch,
        sequence: Sequence::new(sequence).unwrap(),
    };
    let facts = |base, sequence| PeerFactsDelta {
        base,
        cursor: cursor(sequence),
        netmap_serial: netmap.netmap.serial,
        updated: BTreeMap::new(),
        removed: vec![],
        refreshed: Default::default(),
    };
    let mut response = full(&binding, &netmap, HOUR);
    response.peer_facts = Some(facts(None, 3));
    script.push(Ok(response));
    let mut sink = Facts::default();
    assert!(control.sync(&mut sink).await.unwrap().facts);
    let mut response = unchanged(&binding, &netmap, HOUR);
    response.peer_facts = Some(facts(Some(cursor(3)), 4));
    script.push(Ok(response));
    assert!(control.sync(&mut sink).await.unwrap().facts);
    script.push(Ok(unchanged(&binding, &netmap, HOUR)));
    assert!(!control.sync(&mut sink).await.unwrap().facts);
    let requests = script.requests();
    assert_eq!(requests[1].state, Some(cursor(3)));
    assert_eq!(requests[2].state, Some(cursor(4)));
    assert!(sink.published.is_empty(), "no peer facts, nothing changed");
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn an_ended_lease_ends_the_facts_cursor_and_requests_complete_facts() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "facts-ended").await;
    let netmap = compiled(&binding, "facts-ended", None);
    let cursor = StateCursor {
        epoch: "00000000-0000-4000-8000-0000000000e3".parse().unwrap(),
        sequence: Sequence::new(3).unwrap(),
    };
    let mut response = full(&binding, &netmap, 300);
    response.peer_facts = Some(PeerFactsDelta {
        base: None,
        cursor,
        netmap_serial: netmap.netmap.serial,
        updated: BTreeMap::new(),
        removed: vec![],
        refreshed: Default::default(),
    });
    script.push(Ok(response));
    let mut sink = Facts::default();
    control.sync(&mut sink).await.unwrap();
    assert_eq!(
        control.begin_sync().unwrap().request().state,
        Some(cursor),
        "a live lease keeps the cursor"
    );
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(control.netmap().is_none(), "lease ended");
    script.push(Ok(unchanged(&binding, &netmap, HOUR)));
    assert!(
        control
            .sync(&mut sink)
            .await
            .unwrap()
            .netmap
            .lease
            .expires_at_ms
            > 0
    );
    let requests = script.requests();
    assert_eq!(
        requests.last().unwrap().state,
        None,
        "complete facts after the netmap ended"
    );
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn sync_loop_publishes_renewals_and_survives_transport_failure() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (control, binding, script) = enrolled(&server, &proxy, "loop").await;
    let netmap = compiled(&binding, "loop", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    script.push(Err(ClientError::Network { status: None }));
    script.push(Ok(unchanged(&binding, &netmap, 2 * HOUR)));
    let control = tokio::sync::Mutex::new(control);
    let (sender, mut receiver) =
        tokio::sync::watch::channel::<Option<meshway_engine::Netmap>>(None);
    let stop = tokio_util::sync::CancellationToken::new();
    let observe = async {
        receiver.changed().await.unwrap();
        let first = receiver.borrow_and_update().clone().unwrap();
        receiver.changed().await.unwrap();
        let second = receiver.borrow_and_update().clone().unwrap();
        // Same content, later lease: Engine extends its own timer.
        assert!(Arc::ptr_eq(&second.content, &first.content));
        assert!(second.lease.expires_at_ms > first.lease.expires_at_ms + HOUR / 2);
        stop.cancel();
    };
    let mut sink = Facts::default();
    let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(sync_loop(&control, &mut sink, &sender, &stop), observe)
    })
    .await
    .unwrap();
    result.unwrap();
    assert_eq!(script.requests().len(), 3);
    control.into_inner().shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn stop_and_key_rotation_withdraw_the_published_netmap() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "revoke").await;
    let netmap = compiled(&binding, "revoke", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    control.sync(&mut Facts::default()).await.unwrap();
    control.acquire_instance(BTreeMap::new()).await.unwrap();
    control.stop().await.unwrap();
    assert!(control.netmap().is_none());
    control.authenticate().await.unwrap();
    script.push(Ok(unchanged(&binding, &netmap, HOUR)));
    let again = control.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(again.change, NetmapChange::Renewed);
    assert!(control.netmap().is_some());
    control.rotate_keys().await.unwrap();
    assert!(control.netmap().is_none());
    control.shutdown().await.unwrap();
    server.close().await;
}
