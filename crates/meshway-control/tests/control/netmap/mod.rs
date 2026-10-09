//! Netmap authority through the sync long poll. Session, instance and the
//! enrollment come from the real Server; sync replies are scripted until the
//! Server serves the route.
use super::{
    Control, Server,
    faults::{Fault, Proxy},
};
use meshway_control::{Error, NetmapChange, sync_loop};
use meshway_control_client::ClientError;
use meshway_model::{Digest, Sequence};
use meshway_protocol::{EnrollmentResponse, PeerFactsDelta, StateCursor, SyncNetmap};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use support::{Facts, Script, compiled, full, lease, signed, unchanged};

mod restart;
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
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    proxy.fault("/redeem", Fault::ServerKey);
    let binding = server.enroll(&owner, &mut control, name).await;
    assert_eq!(binding.server_signing_key, Some(support::server_key().1));
    control.authenticate().await.unwrap();
    let script = Script::new();
    control.set_sync_transport(script.clone());
    (control, binding, script)
}

#[tokio::test]
async fn server_stop_expires_session_and_instance_while_the_netmap_snapshot_lives() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "offline").await;
    proxy.fault("/auth/session", Fault::ShortSession);
    control.refresh_authentication().await.unwrap();
    proxy.fault("/instances/acquire", Fault::ShortInstance);
    let instance = control.acquire_instance(BTreeMap::new()).await.unwrap();
    let netmap = compiled(&binding, "offline", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    let outcome = control.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(outcome.change, NetmapChange::New { previous: None });
    let snapshot = outcome.snapshot;
    assert_eq!(snapshot.netmap_ref(), Some(netmap.reference()));
    assert_eq!(
        snapshot.instance_binding().unwrap().instance_id,
        instance.instance_id
    );
    assert_eq!(script.requests()[0].netmap, None);
    proxy.close().await;
    server.close().await;
    tokio::time::sleep(Duration::from_millis(2_300)).await;
    let status = control.status();
    assert!(!status.authenticated && status.instance.is_none());
    assert!(control.renew_instance().await.is_err());
    snapshot.ensure_current().unwrap();
    assert!(
        snapshot
            .netmap_reading()
            .unwrap()
            .leased()
            .lease
            .valid_at(0)
    );
    let current = control.netmap_snapshot().unwrap();
    assert!(current.same_netmap(&snapshot));
    assert_eq!(current.instance_binding(), None);
    assert_eq!(status.netmap.unwrap().netmap, netmap.reference());
    assert!(
        snapshot.expires_at_ms()
            > u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap() + HOUR / 2
    );
    // The release cannot reach the stopped Server; authority still ends locally.
    assert!(control.shutdown().await.is_err());
    assert!(snapshot.ensure_current().is_err());
}

#[tokio::test]
async fn renewal_keeps_guards_and_a_newer_serial_retires_them() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "renewal").await;
    let first = compiled(&binding, "renewal", None);
    script.push(Ok(full(&binding, &first, 60_000)));
    let old = control.sync(&mut Facts::default()).await.unwrap().snapshot;
    let waiter = tokio::spawn({
        let old = old.clone();
        async move { old.invalidated().await }
    });
    let deadline = old.expires_at_ms();
    tokio::time::sleep(Duration::from_millis(5)).await;
    script.push(Ok(unchanged(&binding, &first, HOUR)));
    let renewed = control.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(renewed.change, NetmapChange::Renewed);
    assert!(renewed.snapshot.same_netmap(&old));
    assert!(old.expires_at_ms() > deadline + HOUR / 2);
    assert!(renewed.snapshot.netmap_continuation_from(&old).is_err());
    assert_eq!(script.requests()[1].netmap, Some(first.reference()));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!waiter.is_finished() && old.ensure_current().is_ok());

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
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .unwrap()
        .unwrap();
    assert!(old.ensure_current().is_err() && renewed.snapshot.ensure_current().is_err());
    assert_eq!(next.snapshot.netmap_ref(), Some(second.reference()));
    assert_eq!(
        next.snapshot
            .netmap_continuation_from(&old)
            .unwrap()
            .reference(),
        first.reference()
    );
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
    let current = control.sync(&mut Facts::default()).await.unwrap().snapshot;

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
    current.ensure_current().unwrap();
    assert!(control.netmap_snapshot().unwrap().same_netmap(&current));
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
    let snapshot = control.sync(&mut Facts::default()).await.unwrap().snapshot;

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
    snapshot.ensure_current().unwrap();
    // 401 ends only the HTTP session.
    script.push(Err(ClientError::Http {
        status: 401,
        problem: None,
    }));
    assert!(control.sync(&mut Facts::default()).await.is_err());
    assert!(!control.status().authenticated);
    assert!(matches!(control.begin_sync(), Err(Error::Lifecycle)));
    snapshot.ensure_current().unwrap();
    control.authenticate().await.unwrap();
    // 403 rejects this identity: authority ends at once.
    script.push(Err(ClientError::Http {
        status: 403,
        problem: None,
    }));
    assert!(control.sync(&mut Facts::default()).await.is_err());
    assert!(snapshot.ensure_current().is_err() && control.netmap_snapshot().is_err());
    assert_eq!(control.netmap_reference(), Some(netmap.reference()));
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn peer_facts_follow_the_installed_netmap_and_rejection_requests_full_facts() {
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
    };
    let mut response = full(&binding, &netmap, HOUR);
    response.peer_facts = Some(facts(None, 3));
    script.push(Ok(response));
    let mut sink = Facts::default();
    assert!(control.sync(&mut sink).await.unwrap().facts);
    assert_eq!(sink.applied[0].0, netmap.reference());
    let mut response = unchanged(&binding, &netmap, HOUR);
    response.peer_facts = Some(facts(Some(cursor(3)), 4));
    script.push(Ok(response));
    sink.reject = true;
    assert!(!control.sync(&mut sink).await.unwrap().facts);
    script.push(Ok(unchanged(&binding, &netmap, HOUR)));
    control.sync(&mut sink).await.unwrap();
    let requests = script.requests();
    assert_eq!(requests[1].state, Some(cursor(3)));
    assert_eq!(requests[2].state, None);
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
        tokio::sync::watch::channel::<Option<meshway_authority::ExecutionSnapshot>>(None);
    let stop = tokio_util::sync::CancellationToken::new();
    let observe = async {
        receiver.changed().await.unwrap();
        let first = receiver.borrow_and_update().clone().unwrap();
        let initial = first.expires_at_ms();
        receiver.changed().await.unwrap();
        let second = receiver.borrow_and_update().clone().unwrap();
        // One lease cell: the first snapshot observes the renewal too.
        assert!(second.same_netmap(&first));
        assert!(first.expires_at_ms() > initial + HOUR / 2);
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
async fn stop_and_key_rotation_revoke_netmap_authority() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "revoke").await;
    let netmap = compiled(&binding, "revoke", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    let first = control.sync(&mut Facts::default()).await.unwrap().snapshot;
    control.acquire_instance(BTreeMap::new()).await.unwrap();
    control.stop().await.unwrap();
    assert!(first.ensure_current().is_err() && control.netmap_snapshot().is_err());
    control.authenticate().await.unwrap();
    script.push(Ok(unchanged(&binding, &netmap, HOUR)));
    let again = control.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(again.change, NetmapChange::Reissued);
    assert!(!again.snapshot.same_netmap(&first));
    assert!(again.snapshot.netmap_continuation_from(&first).is_err());
    control.rotate_keys().await.unwrap();
    assert!(again.snapshot.ensure_current().is_err());
    assert!(control.netmap_snapshot().is_err());
    control.shutdown().await.unwrap();
    server.close().await;
}
