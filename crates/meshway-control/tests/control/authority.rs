use super::{
    faults::{Fault, Proxy},
    support::Server,
};
use meshway_control::{Control, Error};
use std::time::Duration;

#[tokio::test]
async fn packet_identity_uses_registered_keys_and_stops_with_the_execution_handoff() {
    use ed25519_dalek::{Signature, VerifyingKey};
    let server = Server::start().await;
    let owner = server.owner().await;
    let mut control = Control::open(
        &server.path("packet-identity"),
        &server.origin,
        reqwest::Client::builder().no_proxy(),
    )
    .unwrap();
    server.enroll(&owner, &mut control, "packet-identity").await;
    control.authenticate().await.unwrap();
    control.acquire_instance(Default::default()).await.unwrap();
    control.sync_configuration().await.unwrap();
    let snapshot = control.execution_snapshot().unwrap();
    let identity = control.dataplane_identity(snapshot.clone()).unwrap();
    let cfg = snapshot.configuration().unwrap();
    let digest: meshway_model::Digest = [7; 32].into();
    let signature = identity.sign_packet(digest).unwrap();
    let verifier =
        VerifyingKey::from_bytes(cfg.self_keys.control_key.public_key.bytes.as_bytes()).unwrap();
    verifier
        .verify_strict(
            &meshway_control::packet_proof_message(
                cfg.network_id,
                cfg.node_id,
                &cfg.self_keys.control_key,
                digest,
            ),
            &Signature::from_bytes(&signature),
        )
        .unwrap();
    let public = identity
        .with_noise_key(|key| {
            curve25519_dalek::montgomery::MontgomeryPoint::mul_base_clamped(*key).to_bytes()
        })
        .unwrap();
    assert_eq!(
        public.as_slice(),
        cfg.self_keys
            .dataplane_key
            .public_key
            .bytes
            .as_bytes()
            .as_slice()
    );
    control.stop().await.unwrap();
    assert!(identity.sign_packet(digest).is_err());
    let mut entered = false;
    assert!(identity.with_noise_key(|_| entered = true).is_err());
    assert!(!entered);
    assert!(control.dataplane_identity(snapshot).is_err());
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn execution_handoff_requires_current_saved_target_and_never_renews_old_snapshots() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let path = server.path("execution-handoff");
    let mut control =
        Control::open(&path, &server.origin, reqwest::Client::builder().no_proxy()).unwrap();
    assert!(control.execution_snapshot().is_err());
    server.enroll(&owner, &mut control, "handoff").await;
    control.authenticate().await.unwrap();
    assert!(control.execution_snapshot().is_err());
    control.acquire_instance(Default::default()).await.unwrap();
    assert!(control.execution_snapshot().is_err());
    let target = control.sync_configuration().await.unwrap().unwrap();
    let first = control.execution_snapshot().unwrap();
    let original_deadline = first.expires_at();
    assert_eq!(first.reference(), target);
    assert!(first.configuration().is_ok());
    control.renew_instance().await.unwrap();
    let renewed = control.execution_snapshot().unwrap();
    assert!(first.same_execution(&renewed));
    assert_eq!(first.expires_at(), original_deadline);
    assert!(renewed.expires_at() >= original_deadline);
    control.stop().await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), first.invalidated())
        .await
        .unwrap();
    assert!(matches!(renewed.configuration(), Err(Error::Expired)));
    control.authenticate().await.unwrap();
    control.acquire_instance(Default::default()).await.unwrap();
    control.sync_configuration().await.unwrap();
    let fresh = control.execution_snapshot().unwrap();
    assert!(!first.same_execution(&fresh));
    assert!(first.ensure_current().is_err());
    drop(control);
    tokio::time::timeout(Duration::from_secs(1), fresh.invalidated())
        .await
        .unwrap();
    let restored =
        Control::open(&path, &server.origin, reqwest::Client::builder().no_proxy()).unwrap();
    assert!(restored.cached_target().is_some());
    assert!(restored.execution_snapshot().is_err());
    restored.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn stopped_execution_wakes_waiters_before_a_held_release_http_response() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = Control::open(
        &server.path("execution-stop"),
        &proxy.origin,
        reqwest::Client::builder().no_proxy(),
    )
    .unwrap();
    server.enroll(&owner, &mut control, "execution-stop").await;
    control.authenticate().await.unwrap();
    control.acquire_instance(Default::default()).await.unwrap();
    control.sync_configuration().await.unwrap();
    let snapshot = control.execution_snapshot().unwrap();
    proxy.fault("/instances/release", Fault::Hold);
    {
        let stop = control.stop();
        tokio::pin!(stop);
        tokio::select! {
            result = &mut stop => panic!("release completed before delivery: {result:?}"),
            result = tokio::time::timeout(Duration::from_secs(2), proxy.committed()) => { result.unwrap(); },
        }
        tokio::time::timeout(Duration::from_secs(1), snapshot.invalidated())
            .await
            .unwrap();
        assert!(snapshot.configuration().is_err());
        proxy.release();
        stop.await.unwrap();
    }
    proxy.close().await;
    drop(control);
    server.close().await;
}

#[tokio::test]
async fn idle_execution_snapshot_expires_without_polling_control_or_delaying_http() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = Control::open(
        &server.path("execution-expiry"),
        &proxy.origin,
        reqwest::Client::builder().no_proxy(),
    )
    .unwrap();
    server
        .enroll(&owner, &mut control, "execution-expiry")
        .await;
    proxy.fault("/auth/session", Fault::ShortSession);
    control.authenticate().await.unwrap();
    control.acquire_instance(Default::default()).await.unwrap();
    control.sync_configuration().await.unwrap();
    let snapshot = control.execution_snapshot().unwrap();
    let before = snapshot.expires_at();
    tokio::time::timeout(Duration::from_secs(4), snapshot.invalidated())
        .await
        .unwrap();
    assert!(matches!(snapshot.ensure_current(), Err(Error::Expired)));
    assert_eq!(snapshot.expires_at(), before);
    assert!(control.execution_snapshot().is_err());
    drop(control);
    proxy.close().await;
    server.close().await;
}
