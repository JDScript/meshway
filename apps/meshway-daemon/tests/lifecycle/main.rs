#![cfg(unix)]
#[cfg(target_os = "linux")]
mod cleanup;
mod leave;
#[cfg(not(target_os = "linux"))]
mod network;
#[cfg(target_os = "linux")]
mod probe;
mod recovery;
mod support;
use meshway_control_client::{
    ApplyConfigurationPath, ApproveDeviceAuthorizationPath, GetDeviceAuthorizationPath,
    GetNetworkPath, GetNodeStatePath,
};
use meshway_protocol::ipc::{Command, ErrorCode, Phase, ResultBody};
use meshway_protocol::{ApproveEnrollmentRequest, ConfigurationRequest, state::VisibleReport};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    time::Duration,
};
use support::*;
use tokio::time::Instant;

#[tokio::test]
async fn executable_enrolls_saves_reports_renews_and_restores_up_or_down_intent() {
    // C: observe the daemon's real RENEW / REPORT / POLL (20 s) cycle. The
    // daemon owns renewal; the test waits for a newer confirmed lease/report.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let server = Server::start(root.join("server")).await;
    let (user, network) = server.owner().await;
    let directory = root.join("node");
    // A real DNS listener already owns this port. Control-only enrollment and
    // configuration polling must not reserve system DNS or bind a second stub.
    let dns_tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dns_bind = dns_tcp.local_addr().unwrap();
    let _dns_udp = tokio::net::UdpSocket::bind(dns_bind).await.unwrap();
    let mut daemon = Process::start_with_options(
        &directory,
        &server.origin,
        "control-only",
        &["--dns-bind", &dns_bind.to_string()],
    );
    let initial = wait(&daemon.socket, Phase::Down).await;
    assert!(initial.applied.is_none() && !initial.authenticated);
    let up_id = request_id();
    let up = Command::Up {
        enrollment_key: None,
        expected_revision: initial.revision,
        reenroll: false,
    };
    call(&daemon.socket, up.clone(), up_id).await.unwrap();
    let pending = wait(&daemon.socket, Phase::AwaitingApproval).await;
    let enrollment = pending.enrollment_request_id.unwrap();
    let authorization = server
        .client
        .get_device_authorization(
            &GetDeviceAuthorizationPath {
                enrollment_request_id: enrollment,
            },
            user.expose(),
        )
        .await
        .unwrap();
    assert!(authorization.request.authorization_url.is_some());
    assert_eq!(
        pending.enrollment_url,
        authorization.request.authorization_url
    );
    server
        .client
        .approve_device_authorization(
            &ApproveDeviceAuthorizationPath {
                enrollment_request_id: enrollment,
            },
            &ApproveEnrollmentRequest {
                network_id: network,
            },
            user.expose(),
        )
        .await
        .unwrap();
    let ready = wait(&daemon.socket, Phase::ControlReady).await;
    let identity = ready.identity.clone().unwrap();
    let first = ready.instance.clone().unwrap();
    assert!(ready.authenticated && ready.saved.is_some() && ready.applied.is_none());
    assert_eq!(
        ready.data_plane,
        meshway_protocol::ipc::DataPlane::ControlOnly
    );
    assert_eq!(
        fs::metadata(directory.join("control.sock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let duplicate = call(&daemon.socket, up, up_id).await.unwrap();
    let ResultBody::Status { status: duplicate } = duplicate.result else {
        panic!("expected replay")
    };
    assert_eq!(duplicate.revision, ready.revision);
    assert_eq!(
        duplicate.instance.as_ref().unwrap().instance_id,
        first.instance_id
    );
    assert!(matches!(
        call(
            &daemon.socket,
            Command::Down {
                expected_revision: initial.revision
            },
            request_id()
        )
        .await
        .unwrap()
        .result,
        ResultBody::Error {
            code: ErrorCode::Conflict
        }
    ));
    let network_view = server
        .client
        .get_network(
            &GetNetworkPath {
                network_id: network,
            },
            user.expose(),
        )
        .await
        .unwrap();
    let change:ConfigurationRequest=serde_json::from_value(serde_json::json!({"type":"operations","requestId":request_id(),"expectedRevision":network_view.network.current_revision,
        "ops":[{"type":"setNodeSettings","nodeId":identity.node_id,"fields":{"hostname":"changed-on-server"}}]})).unwrap();
    let applied = server
        .client
        .apply_configuration(
            &ApplyConfigurationPath {
                network_id: network,
            },
            &change,
            user.expose(),
        )
        .await
        .unwrap();
    // The daemon must confirm renewal before the issued instance deadline;
    // preceding configuration HTTP work does not consume an invented 35 s cap.
    let now_ms = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap();
    let deadline =
        Instant::now() + Duration::from_millis(first.expires_at_ms.saturating_sub(now_ms));
    loop {
        let view = status(&daemon.socket).await.unwrap();
        if view
            .saved
            .as_ref()
            .is_some_and(|saved| saved.revision == applied.revision)
            && view
                .instance
                .as_ref()
                .is_some_and(|lease| lease.expires_at_ms > first.expires_at_ms)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "configuration polling or lease renewal stalled"
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    let state = server
        .client
        .get_node_state(
            &GetNodeStatePath {
                network_id: network,
                node_id: identity.node_id,
            },
            user.expose(),
        )
        .await
        .unwrap();
    let Some(VisibleReport::Full { report }) = &state.nodes[&identity.node_id].report else {
        panic!("expected real report")
    };
    assert!(report.configuration.saved.is_some() && report.configuration.applied.is_none());
    assert!(report.links.is_empty() && report.capabilities.is_empty());
    // Process shutdown preserves the user's up intent, but never the instance.
    daemon.exit("-TERM").await;
    assert!(!daemon.socket.exists());
    let mut logs = String::new();
    use std::io::Read;
    daemon
        .child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut logs)
        .unwrap();
    let events: Vec<serde_json::Value> = logs
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        events
            .iter()
            .any(|event| event["fields"]["message"] == "Node intent committed")
    );
    assert!(
        events
            .iter()
            .any(|event| event["fields"]["phase"] == "ControlReady")
    );
    assert!(
        events
            .iter()
            .any(|event| event["fields"]["message"] == "Node stopped")
    );
    for private in [
        "mwns_",
        "daemon test password",
        "authorize-device/",
        "privateKey",
        "sourceJsonc",
    ] {
        assert!(!logs.contains(private));
    }
    drop(daemon);
    let mut daemon = Process::start(&directory, &server.origin);
    // Cached netmap readiness and a newly acquired reporting instance are
    // independent observations. Wait for the latter before comparing it.
    let restored = support::wait_matching(&daemon.socket, |status| {
        status.phase == Phase::ControlReady && status.instance.is_some()
    })
    .await;
    assert_eq!(restored.identity.as_ref().unwrap(), &identity);
    assert_ne!(
        restored.instance.as_ref().unwrap().instance_id,
        first.instance_id
    );
    call(
        &daemon.socket,
        Command::Down {
            expected_revision: restored.revision,
        },
        request_id(),
    )
    .await
    .unwrap();
    let down = wait(&daemon.socket, Phase::Down).await;
    assert!(!down.authenticated && down.instance.is_none());
    assert_eq!(down.identity.as_ref(), Some(&identity));
    daemon.exit("-INT").await;
    drop(daemon);
    let mut daemon = Process::start(&directory, &server.origin);
    let down = wait(&daemon.socket, Phase::Down).await;
    assert!(!down.desired_up && down.instance.is_none());
    // The instance release is best-effort: the authenticated Server can go
    // offline before daemon shutdown without turning a clean join into failure.
    let up = Command::Up {
        enrollment_key: None,
        expected_revision: down.revision,
        reenroll: false,
    };
    call(&daemon.socket, up, request_id()).await.unwrap();
    wait(&daemon.socket, Phase::ControlReady).await;
    server.close().await;
    daemon.exit("-TERM").await;
}

#[tokio::test]
async fn exclusive_process_ownership_and_crash_socket_recovery_preserve_foreign_paths() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("node");
    let mut daemon = Process::start(&path, "http://127.0.0.1:9");
    wait(&daemon.socket, Phase::Down).await;
    let inode = fs::symlink_metadata(&daemon.socket).unwrap().ino();
    let mut duplicate = Process::start(&path, "http://127.0.0.1:9");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(exit) = duplicate.child.try_wait().unwrap() {
            assert!(!exit.success());
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(fs::symlink_metadata(&daemon.socket).unwrap().ino(), inode);
    daemon.child.kill().unwrap();
    daemon.child.wait().unwrap();
    drop(daemon);
    let mut recovered = Process::start(&path, "http://127.0.0.1:9");
    wait(&recovered.socket, Phase::Down).await;
    let socket = recovered.socket.clone();
    let displaced = path.join("displaced.sock");
    fs::rename(&socket, &displaced).unwrap();
    fs::write(&socket, b"foreign user file").unwrap();
    recovered.exit("-TERM").await;
    assert!(fs::read(&socket).unwrap() == b"foreign user file");
    fs::remove_file(&socket).unwrap();
    fs::rename(&displaced, &socket).unwrap();
    let mut final_process = Process::start(&path, "http://127.0.0.1:9");
    wait(&socket, Phase::Down).await;
    final_process.exit("-TERM").await;
}

#[tokio::test]
async fn status_and_down_remain_responsive_while_server_http_is_stalled() {
    use std::sync::Arc;
    use tokio::sync::{Notify, oneshot, watch};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let entered = Arc::new(Notify::new());
    let (release, held) = watch::channel(false);
    let app = axum::Router::new().fallback({
        let entered = entered.clone();
        move || {
            let entered = entered.clone();
            let mut held = held.clone();
            async move {
                entered.notify_one();
                while !*held.borrow() {
                    if held.changed().await.is_err() {
                        break;
                    }
                }
                axum::http::StatusCode::SERVICE_UNAVAILABLE
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let mut daemon = Process::start(&root.join("node"), &origin);
    let initial = wait(&daemon.socket, Phase::Down).await;
    call(
        &daemon.socket,
        Command::Up {
            enrollment_key: None,
            expected_revision: initial.revision,
            reenroll: false,
        },
        request_id(),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let current = tokio::time::timeout(Duration::from_secs(1), status(&daemon.socket))
        .await
        .unwrap()
        .unwrap();
    let started = Instant::now();
    call(
        &daemon.socket,
        Command::Down {
            expected_revision: current.revision,
        },
        request_id(),
    )
    .await
    .unwrap();
    let down = wait(&daemon.socket, Phase::Down).await;
    assert!(started.elapsed() < Duration::from_secs(4));
    assert!(!down.authenticated && down.instance.is_none() && !down.desired_up);
    daemon.exit("-TERM").await;
    release.send_replace(true);
    stop.send(()).unwrap();
    server.await.unwrap();
}

#[test]
fn invalid_logging_configuration_fails_before_creating_node_state() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("node");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_meshwayd"))
        .args([
            "--server",
            "http://127.0.0.1:9",
            "--log-filter",
            "meshway_daemon=not-a-level",
            "--data-dir",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success() && output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Invalid log filter")
    );
    assert!(!path.exists());
}
