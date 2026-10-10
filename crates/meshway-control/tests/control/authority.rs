use super::{
    faults::{Fault, Proxy},
    support::Server,
};
use meshway_control::Control;
use std::time::Duration;

#[tokio::test]
async fn packet_keys_are_the_registered_keys_and_not_authority() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let mut control = Control::open(
        &server.path("packet-identity"),
        &server.origin,
        meshway_test_server::client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    server.enroll(&owner, &mut control, "packet-identity").await;
    control.authenticate().await.unwrap();
    control.acquire_instance(Default::default()).await.unwrap();
    crate::support::sync(&mut control).await.unwrap();
    let netmap = control.netmap().unwrap();
    let keys = control.dataplane_keys().unwrap();
    assert!(keys.matches(&netmap.content.netmap.self_.self_keys));
    let mut other = netmap.content.netmap.self_.self_keys.clone();
    other.dataplane_key = other.control_key.clone();
    assert!(!keys.matches(&other));
    control.stop().await.unwrap();
    // Keys survive stop; only the published netmap is withdrawn.
    assert!(control.netmap().is_none());
    assert!(control.dataplane_keys().is_ok());
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn netmap_is_published_only_with_its_signed_lease_and_never_extended_by_instances() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let path = server.path("execution-handoff");
    let mut control = Control::open(
        &path,
        &server.origin,
        meshway_test_server::client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    assert!(control.netmap().is_none());
    server.enroll(&owner, &mut control, "handoff").await;
    control.authenticate().await.unwrap();
    assert!(control.netmap().is_none());
    control.acquire_instance(Default::default()).await.unwrap();
    assert!(control.netmap().is_none());
    let target = crate::support::sync(&mut control).await.unwrap().unwrap();
    let first = control.netmap().unwrap();
    assert_eq!(first.content.reference(), target);
    assert_eq!(first.lease.netmap, target);
    control.renew_instance().await.unwrap();
    let renewed = control.netmap().unwrap();
    assert_eq!(renewed.lease, first.lease);
    control.stop().await.unwrap();
    assert!(control.netmap().is_none());
    assert_eq!(control.netmap_reference(), Some(target));
    control.authenticate().await.unwrap();
    control.acquire_instance(Default::default()).await.unwrap();
    crate::support::sync(&mut control).await.unwrap();
    let fresh = control.netmap().unwrap();
    assert_eq!(fresh.content.reference(), target);
    assert!(fresh.lease.issued_at_ms >= first.lease.issued_at_ms);
    drop(control);
    let restored = Control::open(
        &path,
        &server.origin,
        meshway_test_server::client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    // The saved signed lease is published again without Server contact.
    assert_eq!(restored.netmap().unwrap().lease.netmap, fresh.lease.netmap);
    restored.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn stop_withdraws_the_netmap_before_a_held_release_http_response() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = Control::open(
        &server.path("execution-stop"),
        &proxy.origin,
        meshway_test_server::client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    server.enroll(&owner, &mut control, "execution-stop").await;
    control.authenticate().await.unwrap();
    control.acquire_instance(Default::default()).await.unwrap();
    crate::support::sync(&mut control).await.unwrap();
    assert!(control.netmap().is_some());
    proxy.fault("/instances/release", Fault::Hold);
    {
        let stop = control.stop();
        tokio::pin!(stop);
        tokio::select! {
            result = &mut stop => panic!("release completed before delivery: {result:?}"),
            result = tokio::time::timeout(Duration::from_secs(2), proxy.committed()) => { result.unwrap(); },
        }
        // Cancelled while the release is held.
    }
    assert!(control.netmap().is_none());
    proxy.release();
    proxy.close().await;
    drop(control);
    server.close().await;
}

#[tokio::test]
async fn idle_netmap_outlives_the_http_session_without_polling_control() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = Control::open(
        &server.path("execution-expiry"),
        &proxy.origin,
        meshway_test_server::client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    server
        .enroll(&owner, &mut control, "execution-expiry")
        .await;
    proxy.fault("/auth/session", Fault::ShortSession);
    control.authenticate().await.unwrap();
    control.acquire_instance(Default::default()).await.unwrap();
    crate::support::sync(&mut control).await.unwrap();
    let before = control.netmap().unwrap().lease;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(!control.status().authenticated);
    assert_eq!(control.netmap().unwrap().lease, before);
    assert_eq!(control.netmap_lease(), Some(before));
    drop(control);
    proxy.close().await;
    server.close().await;
}
