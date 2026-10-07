use super::{refresh, support::Server};
use meshway_control::Control;
use std::time::Duration;

async fn next(stream: &mut meshway_control::NodeEvents) -> meshway_control::EventNotice {
    tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn real_watch_ignores_unchanged_projection_and_refreshes_configuration_change() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let mut control = refresh::control(&server, &owner, &server.origin, "events").await;
    control.refresh_configuration().await.unwrap();
    let original = control.execution_snapshot().unwrap();
    let mut stream = control.watch_node().unwrap();
    for _ in 0..3 {
        assert!(!control.receive_event(next(&mut stream).await).unwrap());
    }
    assert!(!control.receive_event(next(&mut stream).await).unwrap());
    assert!(original.ensure_current().is_ok());
    drop(stream);
    let mut stream = control.watch_node().unwrap();
    for _ in 0..4 {
        assert!(
            !control.receive_event(next(&mut stream).await).unwrap(),
            "reconnection is not reset"
        );
    }
    // Real Server's ten-second comment heartbeat must keep the same connection
    // usable beyond the ordinary twenty-second HTTP request budget.
    assert!(
        tokio::time::timeout(Duration::from_secs(22), stream.next())
            .await
            .is_err()
    );
    refresh::listeners(&server, &owner).await;
    let changed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if control.receive_event(stream.next().await.unwrap()).unwrap() {
                break;
            }
        }
    })
    .await;
    changed.unwrap();
    // Notification alone neither installs the new configuration nor extends it.
    assert!(
        control
            .cached_target()
            .unwrap()
            .configuration
            .listeners
            .is_empty()
    );
    control.refresh_configuration().await.unwrap();
    assert!(
        !control
            .cached_target()
            .unwrap()
            .configuration
            .listeners
            .is_empty()
    );
    drop(stream);
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn queued_old_stream_notice_cannot_rebind_after_authentication_or_stop() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let mut control = refresh::control(&server, &owner, &server.origin, "stale-events").await;
    let mut stream = control.watch_node().unwrap();
    for _ in 0..3 {
        assert!(!next(&mut stream).await.refresh_requested());
    }
    let ready = next(&mut stream).await;
    assert!(ready.refresh_requested());
    control.refresh_authentication().await.unwrap();
    assert!(!control.receive_event(ready).unwrap());
    assert!(stream.next().await.is_none());
    let mut stream = control.watch_node().unwrap();
    let notice = next(&mut stream).await;
    control.stop().await.unwrap();
    assert!(!control.receive_event(notice).unwrap());
    assert!(stream.next().await.is_none());
    drop(stream);
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn watch_requires_valid_instance_and_dropping_it_preserves_granted_execution() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let mut control = Control::open(
        &server.path("drop-events"),
        &server.origin,
        reqwest::Client::builder().no_proxy(),
    )
    .unwrap();
    assert!(control.watch_node().is_err());
    server.enroll(&owner, &mut control, "drop-events").await;
    control.authenticate().await.unwrap();
    assert!(control.watch_node().is_err());
    control.acquire_instance(Default::default()).await.unwrap();
    control.refresh_configuration().await.unwrap();
    let before = control.status().instance.unwrap();
    let execution = control.execution_snapshot().unwrap();
    let mut stream = control.watch_node().unwrap();
    next(&mut stream).await;
    drop(stream);
    assert_eq!(control.status().instance.unwrap(), before);
    execution.ensure_current().unwrap();
    control.shutdown().await.unwrap();
    server.close().await;
}
