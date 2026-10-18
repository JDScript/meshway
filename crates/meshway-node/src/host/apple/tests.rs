use super::*;

#[derive(Default)]
struct SettingsLog(Mutex<Vec<Option<TunnelSettings>>>);
impl TunnelSettingsSink for SettingsLog {
    fn apply(&self, settings: Option<TunnelSettings>) -> SendFuture<'_, io::Result<()>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(settings);
            Ok(())
        })
    }
}

#[tokio::test]
async fn withdrawal_and_restart_retain_only_negative_capture_until_down() {
    let fixture = meshway_desired_fixture::fixture::Chain::new(2).unwrap();
    let desired = fixture.authority.desired(fixture.path[0]).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let directory = Arc::new(
        meshway_private_storage::Directory::open(
            &directory.path().canonicalize().unwrap().join("node"),
            "test.lock",
        )
        .unwrap(),
    );
    let (socket, _peer) = std::os::unix::net::UnixDatagram::pair().unwrap();
    let settings = Arc::new(SettingsLog::default());
    let apple = Apple::new(socket.into(), "utun123456".into(), settings.clone()).unwrap();
    let mut host = apple
        .open(
            directory.clone(),
            "http://127.0.0.1:1",
            Default::default(),
            Default::default(),
        )
        .await
        .unwrap();
    host.install(desired.clone()).await.unwrap();
    let applied = settings.0.lock().unwrap()[0].clone().unwrap();
    assert_eq!(applied.ipv4, Some(fixture.addresses[0]));
    assert_eq!(
        applied.routes,
        vec!["100.64.0.0/24".parse::<ipnet::IpNet>().unwrap()]
    );
    assert!(host.clamp_mss(Default::default()).is_err());
    assert!(host.kernel_changes().is_none());
    assert!(host.control_relay().is_none());
    host.suspend().await.unwrap();
    assert_eq!(settings.0.lock().unwrap().len(), 1);
    assert!(matches!(
        host.sockets()
            .datagram_listener(&desired, meshway_desired_fixture::fixture::LISTENER_KEY)
            .await,
        Err(meshway_connectivity::Error::Underlay)
    ));
    host.shutdown(true).await.unwrap();
    let mut restarted = apple
        .open(
            directory.clone(),
            "http://127.0.0.1:1",
            Default::default(),
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        settings.0.lock().unwrap().as_slice(),
        &[Some(applied.clone()), Some(applied)]
    );
    assert!(matches!(
        restarted
            .sockets()
            .datagram_listener(&desired, meshway_desired_fixture::fixture::LISTENER_KEY)
            .await,
        Err(meshway_connectivity::Error::Underlay)
    ));
    restarted.down().await.unwrap();
    assert!(settings.0.lock().unwrap().last().unwrap().is_none());
    assert!(directory.file(JOURNAL, false).unwrap().is_none());
    restarted.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn refresh_keeps_http_path_while_path_event_interrupts_it() {
    let (socket, _) = std::os::unix::net::UnixDatagram::pair().unwrap();
    let apple = Apple::new(
        socket.into(),
        "utun123456".into(),
        Arc::new(SettingsLog::default()),
    )
    .unwrap();
    let interrupt = PathInterrupt(apple.paths.subscribe());
    let mut waiting = meshway_control_client::RequestInterrupt::interrupted(&interrupt);
    apple.path_changed(None, false).unwrap();
    assert!(futures_util::poll!(waiting.as_mut()).is_pending());
    apple.path_changed(None, true).unwrap();
    tokio::time::timeout(std::time::Duration::from_millis(100), waiting)
        .await
        .unwrap();
}
