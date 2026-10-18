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

#[tokio::test]
async fn dns_uses_the_local_anchor_and_keeps_negative_settings_until_down() {
    let fixture = meshway_desired_fixture::fixture::Chain::new(2).unwrap();
    let desired = fixture.authority.desired(fixture.path[0]).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let directory = Arc::new(
        meshway_private_storage::Directory::open(
            &temporary.path().canonicalize().unwrap().join("node"),
            "test.lock",
        )
        .unwrap(),
    );
    let (socket, _) = std::os::unix::net::UnixDatagram::pair().unwrap();
    let settings = Arc::new(SettingsLog::default());
    let apple = Apple::new(socket.into(), "utun123456".into(), settings.clone()).unwrap();
    let mut host = apple
        .open(
            directory,
            "http://127.0.0.1:1",
            Default::default(),
            Default::default(),
        )
        .await
        .unwrap();
    host.install(desired.clone()).await.unwrap();
    let mut dns = host.prepare_dns().await.unwrap();
    let transport = dns.packet_transport().unwrap().unwrap();
    let plan = meshway_dns::SystemDnsPlan::new(
        &desired.netmap().projection.configuration.dns,
        &desired.netmap().dns.egress,
    )
    .unwrap();
    dns.install(&plan).await.unwrap();
    dns.observe().await.unwrap();
    let last = settings.0.lock().unwrap().last().unwrap().clone().unwrap();
    let installed = last.dns.as_ref().unwrap();
    assert_eq!(
        installed.server,
        meshway_dns::SystemDnsSettings::default().anchor_address
    );
    assert_eq!(installed.match_domains, ["perf.internal"]);
    assert_eq!(installed.search_domains, ["perf.internal"]);
    assert!(
        last.included_routes()
            .contains(&"198.18.255.254/32".parse().unwrap())
    );
    let count = settings.0.lock().unwrap().len();
    host.suspend().await.unwrap();
    assert_eq!(settings.0.lock().unwrap().len(), count);
    assert!(host.dns_packets().is_some());
    dns.down().await.unwrap();
    assert!(
        settings
            .0
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .as_ref()
            .unwrap()
            .dns
            .is_none()
    );
    drop(transport);
    dns.shutdown(false).await.unwrap();
    host.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn a_dns_anchor_inside_the_mesh_pool_cannot_install() {
    let fixture = meshway_desired_fixture::fixture::Chain::new(2).unwrap();
    let desired = fixture.authority.desired(fixture.path[0]).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let directory = Arc::new(
        meshway_private_storage::Directory::open(
            &temporary.path().canonicalize().unwrap().join("node"),
            "test.lock",
        )
        .unwrap(),
    );
    let (socket, _) = std::os::unix::net::UnixDatagram::pair().unwrap();
    let settings = Arc::new(SettingsLog::default());
    let apple = Apple::new(socket.into(), "utun123456".into(), settings.clone()).unwrap();
    let dns = meshway_dns::SystemDnsSettings {
        anchor_address: "100.64.0.53".parse().unwrap(),
        ..Default::default()
    };
    let mut host = apple
        .open(directory, "http://127.0.0.1:1", Default::default(), dns)
        .await
        .unwrap();
    host.install(desired).await.unwrap();
    let failure = host
        .prepare_dns()
        .await
        .err()
        .expect("conflicting service address");
    assert_eq!(failure.error, meshway_dns::SystemDnsError::Conflict);
    assert_eq!(failure.check, "apple_dns_mesh_pool");
    assert!(host.dns_packets().is_none());
    assert!(settings.0.lock().unwrap().iter().all(|settings| {
        settings
            .as_ref()
            .is_none_or(|settings| settings.dns.is_none())
    }));
    host.shutdown(false).await.unwrap();
}

#[test]
fn local_interface_address_conflicts_are_reported() {
    let failure = dns::conflict(std::net::Ipv4Addr::LOCALHOST, &[]).unwrap_err();
    assert_eq!(failure.error, meshway_dns::SystemDnsError::Conflict);
    assert_eq!(failure.check, "apple_dns_interface_address");
}

#[test]
fn exit_ranges_preserve_direct_holes_and_capture_both_ipv6_kinds() {
    let plan = meshway_configuration::execution::EgressCapturePlan {
        generation: meshway_model::Digest::from_bytes([0; 32]),
        managed_ipv4: vec![("11.0.0.1".parse().unwrap(), "11.0.0.6".parse().unwrap())],
        default_capture: false,
        managed_ipv6: vec![("3ffe::1".parse().unwrap(), "3ffe::6".parse().unwrap())],
        sink_ipv6: vec![("3ffe::10".parse().unwrap(), "3ffe::1f".parse().unwrap())],
        unavailable: None,
    };
    let routes = capture::routes(Some(&plan)).unwrap();
    for last in 0..=32 {
        for prefix in ["11.0.0.", "3ffe::"] {
            let address: IpAddr = if prefix == "3ffe::" {
                format!("{prefix}{last:x}")
            } else {
                format!("{prefix}{last}")
            }
            .parse()
            .unwrap();
            let expected =
                (1..=6).contains(&last) || (address.is_ipv6() && (16..=31).contains(&last));
            assert_eq!(
                routes.iter().any(|route| route.contains(&address)),
                expected,
                "{address}"
            );
        }
    }
    assert!(capture::routes(None).unwrap().is_empty());
}

#[tokio::test]
async fn exit_clear_stages_union_and_failed_shrink_keeps_restart_boundary() {
    struct Sink {
        log: SettingsLog,
        fail: std::sync::atomic::AtomicBool,
    }
    impl TunnelSettingsSink for Sink {
        fn apply(&self, settings: Option<TunnelSettings>) -> SendFuture<'_, io::Result<()>> {
            Box::pin(async move {
                self.log.apply(settings.clone()).await?;
                if self.fail.load(std::sync::atomic::Ordering::Relaxed)
                    && settings
                        .as_ref()
                        .is_some_and(|settings| settings.egress_routes.is_empty())
                {
                    return Err(io::Error::other("NE completion failed"));
                }
                Ok(())
            })
        }
    }
    let temporary = tempfile::tempdir().unwrap();
    let directory = Arc::new(
        meshway_private_storage::Directory::open(
            &temporary.path().canonicalize().unwrap().join("node"),
            "test.lock",
        )
        .unwrap(),
    );
    let sink = Arc::new(Sink {
        log: SettingsLog::default(),
        fail: false.into(),
    });
    let mut owner = network_settings::Owner {
        directory: directory.clone(),
        sink: sink.clone(),
        current: None,
        pools: vec![],
        failed: false,
    };
    let exit = TunnelSettings {
        ipv4: Some("100.64.0.1".parse().unwrap()),
        ipv6: None,
        routes: vec!["100.64.0.0/24".parse().unwrap()],
        egress_routes: vec!["0.0.0.0/0".parse().unwrap(), "::/0".parse().unwrap()],
        mtu: 1366,
        dns: None,
    };
    owner.apply(Some(exit.clone())).await.unwrap();
    let mesh = TunnelSettings {
        egress_routes: vec![],
        ..exit.clone()
    };
    sink.fail.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(owner.apply(Some(mesh.clone())).await.is_err());
    let record: CaptureRecord =
        serde_json::from_reader(directory.file(JOURNAL, false).unwrap().unwrap()).unwrap();
    assert_eq!(record.settings, exit);
    assert!(owner.failed);
    sink.fail.store(false, std::sync::atomic::Ordering::Relaxed);
    owner.apply(Some(mesh.clone())).await.unwrap();
    let record: CaptureRecord =
        serde_json::from_reader(directory.file(JOURNAL, false).unwrap().unwrap()).unwrap();
    assert_eq!(record.settings, mesh);
    assert_eq!(
        sink.log.0.lock().unwrap().as_slice(),
        &[
            Some(exit.clone()),
            Some(exit.clone()),
            Some(mesh.clone()),
            Some(exit),
            Some(mesh)
        ]
    );
}
