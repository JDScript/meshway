use super::{
    faults::{Fault, Proxy},
    refresh,
    support::Server,
};
use meshway_control::Control;

#[tokio::test]
async fn failed_final_identity_publish_keeps_durable_pending_material_for_recovery() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let path = server.path("write-rotation");
    let mut control = refresh::control(&server, &owner, &proxy.origin, "write-rotation").await;
    let original = control.status().binding.unwrap();
    proxy.fault("/auth/rotate", Fault::Hold);
    let record = path.join("control.json");
    let backup = path.join("test-pending.json");
    {
        let rotate = control.rotate_keys();
        tokio::pin!(rotate);
        tokio::select! {
            result = &mut rotate => panic!("rotation should be held: {result:?}"),
            result = tokio::time::timeout(std::time::Duration::from_secs(3), proxy.committed()) => result.unwrap(),
        }
        // Keep the durable pending bytes intact, but prevent its atomic replace.
        std::fs::rename(&record, &backup).unwrap();
        std::fs::create_dir(&record).unwrap();
        proxy.release();
        assert!(rotate.await.is_err());
    }
    assert!(control.status().reload_required);
    assert!(control.authenticate().await.is_err());
    control.shutdown().await.unwrap();
    std::fs::remove_dir(&record).unwrap();
    std::fs::rename(&backup, &record).unwrap();
    let mut control =
        Control::open(&path, &proxy.origin, reqwest::Client::builder().no_proxy()).unwrap();
    assert!(control.rotation_pending());
    let keys = control.rotate_keys().await.unwrap();
    assert_eq!(
        keys.control.generation,
        original.keys.control.generation.checked_next().unwrap()
    );
    control.authenticate().await.unwrap();
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn rejected_expired_rotation_rolls_back_only_after_old_key_authentication() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = refresh::control(&server, &owner, &proxy.origin, "rollback-rotation").await;
    let original = control.status().binding.unwrap();
    proxy.fault("/auth/rotate", Fault::RejectBefore);
    assert!(control.rotate_keys().await.is_err());
    assert!(control.rotation_pending());
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    assert!(control.rotate_keys().await.is_err());
    assert!(!control.rotation_pending());
    assert_eq!(control.status().binding.unwrap(), original);
    assert!(control.status().authenticated);
    let keys = control.rotate_keys().await.unwrap();
    assert_eq!(
        keys.control.generation,
        original.keys.control.generation.checked_next().unwrap()
    );
    control.authenticate().await.unwrap();
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn rotation_retires_execution_and_atomically_restores_new_identity_after_restart() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let mut control = refresh::control(&server, &owner, &server.origin, "rotate").await;
    control.refresh_configuration().await.unwrap();
    let original = control.status().binding.unwrap();
    let execution = control.execution_snapshot().unwrap();
    let keys = control.rotate_keys().await.unwrap();
    assert_eq!(
        keys.control.generation,
        original.keys.control.generation.checked_next().unwrap()
    );
    assert_ne!(keys.control.id, original.keys.control.id);
    assert_ne!(
        keys.dataplane.public_key,
        original.keys.dataplane.public_key
    );
    assert!(!control.rotation_pending());
    assert!(execution.ensure_current().is_err());
    assert!(!control.status().authenticated);
    assert!(control.status().cached.is_none());
    control.shutdown().await.unwrap();
    let mut control = Control::open(
        &server.path("rotate"),
        &server.origin,
        reqwest::Client::builder().no_proxy(),
    )
    .unwrap();
    assert_eq!(control.status().binding.unwrap().keys, keys);
    control.authenticate().await.unwrap();
    control.acquire_instance(Default::default()).await.unwrap();
    control.refresh_configuration().await.unwrap();
    control
        .execution_snapshot()
        .unwrap()
        .ensure_current()
        .unwrap();
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn lost_committed_rotation_keeps_both_keys_and_recovers_the_same_generation() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = refresh::control(&server, &owner, &proxy.origin, "lost-rotation").await;
    let original = control.status().binding.unwrap();
    proxy.fault("/auth/rotate", Fault::Lost);
    assert!(control.rotate_keys().await.is_err());
    assert!(control.rotation_pending());
    assert_eq!(control.status().binding.unwrap(), original);
    assert!(!control.status().authenticated);
    control.shutdown().await.unwrap();
    let mut control = Control::open(
        &server.path("lost-rotation"),
        &proxy.origin,
        reqwest::Client::builder().no_proxy(),
    )
    .unwrap();
    assert!(control.rotation_pending());
    let keys = control.rotate_keys().await.unwrap();
    assert_eq!(
        keys.control.generation,
        original.keys.control.generation.checked_next().unwrap()
    );
    assert_eq!(proxy.count("/auth/rotate"), 2);
    control.authenticate().await.unwrap();
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn cancelled_committed_rotation_recovers_from_the_durable_challenge() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = refresh::control(&server, &owner, &proxy.origin, "cancel-rotation").await;
    let original = control.status().binding.unwrap();
    proxy.fault("/auth/rotate", Fault::Hold);
    {
        let rotate = control.rotate_keys();
        tokio::pin!(rotate);
        tokio::select! {
            result = &mut rotate => panic!("rotation should be held: {result:?}"),
            result = tokio::time::timeout(std::time::Duration::from_secs(3), proxy.committed()) => result.unwrap(),
        }
    }
    assert!(control.rotation_pending());
    proxy.release();
    control.shutdown().await.unwrap();
    let mut control = Control::open(
        &server.path("cancel-rotation"),
        &proxy.origin,
        reqwest::Client::builder().no_proxy(),
    )
    .unwrap();
    let keys = control.rotate_keys().await.unwrap();
    assert_eq!(
        keys.control.generation,
        original.keys.control.generation.checked_next().unwrap()
    );
    control.authenticate().await.unwrap();
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}
