use super::{Control, Server};
use meshway_control::Error;
use std::{fs, os::unix::fs::PermissionsExt};

#[tokio::test]
async fn corrupt_record_and_wrong_origin_never_regenerate_identity() {
    let server = Server::start().await;
    let path = server.path("node");
    let mut control = Control::open(
        &path,
        &server.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    control.begin_enrollment("persistent-node").await.unwrap();
    assert!(matches!(
        Control::open(
            &path,
            &server.origin,
            meshway_control_client::http_client_builder().unwrap()
        ),
        Err(Error::Storage(meshway_private_storage::Error::InUse))
    ));
    control.shutdown().await.unwrap();
    let saved = fs::read(path.join("control.json")).unwrap();
    fs::remove_file(path.join("control.json")).unwrap();
    assert!(matches!(
        Control::open(
            &path,
            &server.origin,
            meshway_control_client::http_client_builder().unwrap()
        ),
        Err(Error::Record)
    ));
    fs::write(path.join("control.json"), &saved).unwrap();
    fs::set_permissions(path.join("control.json"), fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(
        Control::open(
            &path,
            "https://different.example",
            meshway_control_client::http_client_builder().unwrap()
        ),
        Err(Error::Record)
    ));
    assert!(fs::read(path.join("control.json")).unwrap() == saved);
    for invalid in [b"{".as_slice(), b"{}".as_slice(), b"[]".as_slice()] {
        fs::write(path.join("control.json"), invalid).unwrap();
        assert!(matches!(
            Control::open(
                &path,
                &server.origin,
                meshway_control_client::http_client_builder().unwrap()
            ),
            Err(Error::Record)
        ));
        assert!(fs::read(path.join("control.json")).unwrap() == invalid);
    }
    fs::write(path.join("control.json"), &saved).unwrap();
    fs::set_permissions(path.join("control.json"), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        Control::open(
            &path,
            &server.origin,
            meshway_control_client::http_client_builder().unwrap()
        ),
        Err(Error::Storage(
            meshway_private_storage::Error::UnsafeStorage
        ))
    ));
    server.close().await;
}

#[test]
fn remote_plaintext_and_ambient_url_credentials_are_rejected_before_storage() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("must-not-exist");
    for origin in [
        "http://example.com",
        "http://localhost:8420",
        "https://user:password@example.com",
        "https://example.com/path",
        "https://example.com?token=secret",
    ] {
        assert!(matches!(
            Control::open(
                &path,
                origin,
                meshway_control_client::http_client_builder().unwrap()
            ),
            Err(Error::Endpoint)
        ));
        assert!(!path.exists());
    }
}

#[tokio::test]
async fn committed_redemption_and_state_reports_retry_the_same_identity_after_delivery_loss() {
    use super::faults::{Fault, Proxy};
    use meshway_control_client::{ApproveDeviceAuthorizationPath, ListNodesPath};
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let path = server.path("recovery");
    let mut control = Control::open(
        &path,
        &proxy.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    let pending = control.begin_enrollment("recoverable").await.unwrap();
    server
        .client
        .approve_device_authorization(
            &ApproveDeviceAuthorizationPath {
                enrollment_request_id: pending.id,
            },
            &meshway_protocol::ApproveEnrollmentRequest {
                network_id: owner.network,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    control.poll_enrollment().await.unwrap();
    proxy.fault("/redeem", Fault::Lost);
    assert!(control.redeem_enrollment().await.is_err());
    // A consumed request is recoverable identity, never disposable pending work.
    assert!(matches!(
        control.cancel_enrollment().await,
        Err(Error::Lifecycle)
    ));
    control.shutdown().await.unwrap();
    let nodes = server
        .client
        .list_nodes(
            &ListNodesPath {
                network_id: owner.network,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    assert_eq!(nodes.items.len(), 1);
    let mut control = Control::open(
        &path,
        &proxy.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    let binding = control.redeem_enrollment().await.unwrap();
    assert_eq!(binding.node_id, nodes.items[0].id);
    assert_eq!(proxy.count("/redeem"), 2);
    control.authenticate().await.unwrap();
    control.acquire_instance(Default::default()).await.unwrap();
    control.sync_configuration().await.unwrap();
    proxy.fault("/configuration/receipts", Fault::Lost);
    assert!(control.acknowledge_saved().await.is_err());
    control.acknowledge_saved().await.unwrap();
    control.acknowledge_saved().await.unwrap();
    assert_eq!(proxy.count("/configuration/receipts"), 2);
    proxy.fault("/agent-api/v1/state", Fault::Lost);
    assert!(control.report_control_state(super::system()).await.is_err());
    let accepted = control.retry_state_report().await.unwrap();
    assert!(accepted.duplicate);
    assert_eq!(accepted.sequence.get(), 1);
    assert_eq!(
        control
            .report_control_state(super::system())
            .await
            .unwrap()
            .sequence
            .get(),
        2
    );
    let old_instance = control.status().instance.unwrap().instance_id;
    let retained = control.snapshot();
    proxy.fault("/instances/release", Fault::Hold);
    let mut stop = Box::pin(control.stop());
    tokio::select! {
        result = &mut stop => panic!("unexpected delivery: {}", result.is_ok()),
        _ = proxy.committed() => {},
    }
    assert!(!retained.status().authenticated && retained.status().instance.is_none());
    drop(stop);
    assert!(!control.status().authenticated && control.status().instance.is_none());
    proxy.release();
    control.authenticate().await.unwrap();
    assert_ne!(
        control
            .acquire_instance(Default::default())
            .await
            .unwrap()
            .instance_id,
        old_instance
    );
    control.sync_configuration().await.unwrap();
    let saved = control.status().cached;
    proxy.fault("/configuration/query", Fault::WrongInstance);
    assert!(matches!(
        control.sync_configuration().await,
        Err(Error::Contract)
    ));
    assert!(control.status().instance.is_none());
    assert_eq!(control.status().cached, saved);
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn foreign_installation_challenge_never_receives_a_session_signature() {
    use super::faults::{Fault, Proxy};
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = Control::open(
        &server.path("pin"),
        &proxy.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    let pending = control.begin_enrollment("pinned").await.unwrap();
    server
        .client
        .approve_device_authorization(
            &meshway_control_client::ApproveDeviceAuthorizationPath {
                enrollment_request_id: pending.id,
            },
            &meshway_protocol::ApproveEnrollmentRequest {
                network_id: owner.network,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    control.poll_enrollment().await.unwrap();
    control.redeem_enrollment().await.unwrap();
    proxy.fault("/auth/challenges", Fault::WrongController);
    assert!(matches!(control.authenticate().await, Err(Error::Contract)));
    assert_eq!(proxy.count("/auth/session"), 0);
    assert!(!control.status().authenticated);
    control.authenticate().await.unwrap();
    assert!(control.status().authenticated);
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn cancelled_http_waiter_keeps_its_durable_pending_intent() {
    use super::faults::{Fault, Proxy};
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let path = server.path("cancelled");
    let mut control = Control::open(
        &path,
        &proxy.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    proxy.fault("/enrollment-requests", Fault::Hold);
    let mut begin = Box::pin(control.begin_enrollment("cancelled-waiter"));
    tokio::select! {
        result = &mut begin => panic!("unexpected delivery: {}",result.is_ok()),
        _ = proxy.committed() => {},
    }
    drop(begin);
    let original = control.status().enrollment_request_id.unwrap();
    assert!(!control.status().reload_required);
    proxy.release();
    control.shutdown().await.unwrap();
    let mut restored = Control::open(
        &path,
        &proxy.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    assert_eq!(
        restored
            .begin_enrollment("cancelled-waiter")
            .await
            .unwrap()
            .id,
        original
    );
    restored.cancel_enrollment().await.unwrap();
    restored.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}
