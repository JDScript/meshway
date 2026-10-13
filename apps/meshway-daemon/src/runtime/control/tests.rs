use super::*;

struct Facts;
impl meshway_control::PeerFactsSink for Facts {
    fn publish(&mut self, _: meshway_control::KnownPeers) {}
}

#[tokio::test]
#[ignore = "requires an isolated Linux network namespace, TUN and NET_ADMIN"]
async fn live_tun_owner_conflict_retries_and_recovers_without_new_up() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let app = meshway_server::App::open(meshway_server::Settings::new(root.join("server")))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let router = meshway_server::http::router(&app);
    let server = tokio::spawn(async {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let client = meshway_control_client::Client::new(
        &origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
        16 * 1024 * 1024,
    )
    .unwrap();
    let user = client
        .register(&meshway_protocol::RegisterRequest {
            email: "owner-conflict@example.test".into(),
            password: meshway_protocol::WireSecret::new(
                "private owner conflict fixture password".into(),
            ),
        })
        .await
        .unwrap();
    let request = || meshway_model::RequestId::new(uuid::Uuid::from_u128(42)).unwrap();
    let network = client
        .create_network(
            &meshway_protocol::CreateNetworkRequest {
                request_id: request(),
                name: "owner-conflict".into(),
            },
            user.token.expose(),
        )
        .await
        .unwrap();
    let mut control = Control::open(
        &root.join("control"),
        &origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    let enrollment = control.begin_enrollment("owner").await.unwrap();
    client
        .approve_device_authorization(
            &meshway_control_client::ApproveDeviceAuthorizationPath {
                enrollment_request_id: enrollment.id,
            },
            &meshway_protocol::ApproveEnrollmentRequest {
                network_id: network.network.id,
            },
            user.token.expose(),
        )
        .await
        .unwrap();
    control.poll_enrollment().await.unwrap();
    control.redeem_enrollment().await.unwrap();
    control.authenticate().await.unwrap();
    let netmap = control.sync(&mut Facts).await.unwrap().netmap;
    let snapshot = meshway_engine::Desired::new(&netmap.content, &Default::default()).unwrap();

    let owner_directory = Arc::new(
        meshway_private_storage::Directory::open(&root.join("owner"), "owner.lock").unwrap(),
    );
    let options = meshway_underlay::Options {
        bootstrap_nameservers: Some(vec!["192.0.2.53:53".parse().unwrap()]),
        ..Default::default()
    };
    let mut owner = meshway_underlay::Host::open(owner_directory.clone(), options.clone())
        .await
        .unwrap();
    owner.install(snapshot).await.unwrap();
    let tun_index = owner.tun().unwrap().if_index().unwrap();
    let directory = Arc::new(
        meshway_private_storage::Directory::open(&root.join("restart"), "restart.lock").unwrap(),
    );
    directory
        .replace(
            "underlay.json",
            &std::fs::read(owner_directory.path().join("underlay.json")).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        meshway_underlay::Host::open(directory.clone(), options.clone()).await,
        Err(meshway_underlay::Error::Conflict)
    ));

    let revision = meshway_model::Version::MIN;
    let (desired, desired_rx) = watch::channel(Desired {
        revision,
        up: true,
        shutdown: false,
        retry_enrollment: false,
        enrollment_key: None,
        rotation: None,
        reenroll: false,
        logout: None,
    });
    let (reported, mut report) = watch::channel(Report::initial(revision));
    let (network_reported, _) = watch::channel(network::Report::stopped());
    let (pings, _) = watch::channel(None);
    let (server_probe, _) = watch::channel(None);
    let (sync, _) = watch::channel(SyncStatus::default());
    let mut settings = Settings::new(
        directory.path().to_owned(),
        "http://127.0.0.1:9",
        Some("restart"),
    )
    .unwrap();
    settings.underlay = options;
    let driver = tokio::spawn(run(
        Arc::new(settings),
        desired_rx,
        reported,
        directory,
        network_reported,
        pings,
        server_probe,
        sync,
        tokio::sync::mpsc::channel(1).1,
    ));
    let first = tokio::time::timeout(
        Duration::from_secs(10),
        report.wait_for(|r| r.problem == Some(Problem::Underlay)),
    )
    .await
    .unwrap()
    .unwrap()
    .clone();
    assert_eq!(first.phase, Phase::Retrying);
    assert!(first.retry_at.is_some());
    let second = tokio::time::timeout(
        Duration::from_secs(5),
        report.wait_for(|r| r.retry_at > first.retry_at),
    )
    .await
    .unwrap()
    .unwrap()
    .clone();
    assert_eq!(second.problem, Some(Problem::Underlay));
    assert_eq!(owner.tun().unwrap().if_index().unwrap(), tun_index);
    owner.shutdown(true).await.unwrap();
    let recovered = tokio::time::timeout(
        Duration::from_secs(10),
        report.wait_for(|r| r.problem == Some(Problem::Network)),
    )
    .await
    .unwrap()
    .unwrap()
    .clone();
    assert_eq!(recovered.revision, revision);
    assert!(recovered.retry_at.is_some());
    desired.send_modify(|d| {
        d.up = false;
        d.shutdown = true;
    });
    driver.await.unwrap().unwrap();
    control.shutdown().await.unwrap();
    app.shutdown().await.unwrap();
    let _ = stop.send(());
    server.await.unwrap();
}
