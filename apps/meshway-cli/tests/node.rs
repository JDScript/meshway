#![cfg(unix)]
use meshway_control_client::{ApproveDeviceAuthorizationPath, Client, GetDeviceAuthorizationPath};
use meshway_model::RequestId;
use meshway_protocol::ipc::{Phase, Status};
use meshway_protocol::{
    ApproveEnrollmentRequest, CreateNetworkRequest, RegisterRequest, WireSecret,
};
use std::{path::Path, process::Output, time::Duration};
use tokio::{sync::oneshot, time::Instant};

fn id() -> RequestId {
    let mut b = [0; 16];
    getrandom::fill(&mut b).unwrap();
    RequestId::new(uuid::Builder::from_random_bytes(b).into_uuid()).unwrap()
}
async fn cli(socket: &Path, command: &str, json: bool) -> Output {
    let socket = socket.to_owned();
    let command = command.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut process = std::process::Command::new(env!("CARGO_BIN_EXE_meshway"));
        process.args(["--log-format", "json", "--log-filter", "debug"]);
        process.arg("--socket").arg(socket).arg(command);
        if json {
            process.arg("--json");
        }
        process.output().unwrap()
    })
    .await
    .unwrap()
}
async fn status(socket: &Path, wanted: Phase) -> Status {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let output = cli(socket, "status", true).await;
        if output.status.success() {
            let value: Status = serde_json::from_slice(&output.stdout).unwrap();
            if value.phase == wanted {
                return value;
            }
            assert_ne!(value.phase, Phase::Blocked, "{:?}", value.last_error);
        }
        assert!(
            Instant::now() < deadline,
            "CLI status did not reach {wanted:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_cli_controls_daemon_through_private_ipc_and_real_server() {
    real_cli_enrollment(Some("https://console.example.test:8443"), None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_cli_preserves_null_server_link_and_prompts_only_while_awaiting_approval() {
    real_cli_enrollment(None, None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_cli_enrolls_with_key_file_and_rotates_through_the_same_owner() {
    real_cli_enrollment(None, Some(false)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_cli_enrolls_with_key_stdin_without_exposing_the_secret() {
    real_cli_enrollment(None, Some(true)).await;
}

async fn real_cli_enrollment(console_origin: Option<&str>, key_stdin: Option<bool>) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut server_settings = meshway_server::Settings::new(root.join("server"));
    server_settings.console_origin =
        console_origin.map(|origin| meshway_server::console_origin(origin).unwrap());
    let server = meshway_server::App::open(server_settings).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = meshway_server::http::router(&server);
    let (server_stop, stopped) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let api = Client::new(
        &origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy()
            .timeout(Duration::from_secs(10)),
        16 * 1024 * 1024,
    )
    .unwrap();
    let owner = api
        .register(&RegisterRequest {
            email: "cli@example.test".into(),
            password: WireSecret::new("CLI fixture password".into()),
        })
        .await
        .unwrap();
    let network = api
        .create_network(
            &CreateNetworkRequest {
                request_id: id(),
                name: "CLI fixture".into(),
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    let path = root.join("daemon");
    let socket = path.join("control.sock");
    let settings = meshway_daemon::Settings::new(path.clone(), &origin, Some("cli-node"))
        .unwrap()
        .with_network_mode(meshway_protocol::ipc::NetworkMode::ControlOnly);
    let (stop, stopped) = oneshot::channel();
    let daemon = tokio::spawn(meshway_daemon::run(settings, async {
        let _ = stopped.await;
    }));
    status(&socket, Phase::Down).await;
    let mut raw = tokio::net::UnixStream::connect(&socket).await.unwrap();
    meshway_protocol::ipc::write(
        &mut raw,
        &meshway_protocol::ipc::Request {
            version: 3,
            request_id: id(),
            command: meshway_protocol::ipc::Command::Ping {
                target: meshway_protocol::ipc::PingTarget::Name {
                    name: "missing.mesh".into(),
                },
                count: 1,
            },
        },
    )
    .await
    .unwrap();
    let response: meshway_protocol::ipc::Response =
        meshway_protocol::ipc::read(&mut raw).await.unwrap();
    assert_eq!(response.version, 3);
    assert!(matches!(
        response.result,
        meshway_protocol::ipc::ResultBody::Error {
            code: meshway_protocol::ipc::ErrorCode::Version
        }
    ));
    drop(raw);
    let mut raw = tokio::net::UnixStream::connect(&socket).await.unwrap();
    meshway_protocol::ipc::write(
        &mut raw,
        &serde_json::json!({
            "version":meshway_protocol::ipc::VERSION,"requestId":id(),
            "command":{"type":"ping","target":{"type":"name","name":"missing.mesh"},"count":0}
        }),
    )
    .await
    .unwrap();
    assert!(
        meshway_protocol::ipc::read::<meshway_protocol::ipc::Response>(&mut raw)
            .await
            .is_err()
    );
    drop(raw);
    // Real IPC/runtime rejection must leave durable intent untouched.
    let before_ping = meshway_protocol::ipc::Client::new(&socket)
        .call(meshway_protocol::ipc::Command::Status)
        .await
        .unwrap();
    let mut ping = meshway_protocol::ipc::Client::new(&socket)
        .ping(
            meshway_protocol::ipc::PingTarget::Name {
                name: "missing.mesh".into(),
            },
            1,
        )
        .await
        .unwrap();
    assert!(matches!(
        ping.next().await.unwrap(),
        Some(meshway_protocol::ipc::PingEvent::Failed {
            code: meshway_protocol::ipc::PingError::Unsupported
        })
    ));
    assert!(ping.next().await.unwrap().is_none());
    let after_ping = meshway_protocol::ipc::Client::new(&socket)
        .call(meshway_protocol::ipc::Command::Status)
        .await
        .unwrap();
    assert_eq!(after_ping.revision, before_ping.revision);
    assert_eq!(after_ping.desired_up, before_ping.desired_up);
    if let Some(stdin) = key_stdin {
        key_enrollment(
            &api,
            network.network.id,
            owner.token.expose(),
            &root,
            &socket,
            stdin,
        )
        .await;
    } else {
        let output = cli(&socket, "up", true).await;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let pending: Status = serde_json::from_slice(&output.stdout).unwrap();
        let logs: Vec<serde_json::Value> = std::str::from_utf8(&output.stderr)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(!logs.is_empty());
        assert!(logs.iter().any(|event| event["fields"]["command"] == "up"));
        assert_eq!(pending.phase, Phase::AwaitingApproval);
        assert_eq!(pending.last_error, None);
        let enrollment = pending.enrollment_request_id.unwrap();
        // The daemon created and polled through its generated client. Compare its
        // projection with the same request read over the real user API, not a
        // locally reconstructed Console route.
        let authorization = api
            .get_device_authorization(
                &GetDeviceAuthorizationPath {
                    enrollment_request_id: enrollment,
                },
                owner.token.expose(),
            )
            .await
            .unwrap();
        let expected = authorization.request.authorization_url;
        assert_eq!(expected.is_some(), console_origin.is_some());
        assert_eq!(pending.enrollment_url, expected);
        let ipc = meshway_protocol::ipc::Client::new(&socket)
            .call(meshway_protocol::ipc::Command::Status)
            .await
            .unwrap();
        assert_eq!(ipc.enrollment_url, expected);
        let json = cli(&socket, "status", true).await;
        assert!(json.status.success());
        let current: Status = serde_json::from_slice(&json.stdout).unwrap();
        assert_eq!(current.phase, Phase::AwaitingApproval);
        assert_eq!(current.enrollment_url, expected);
        let human_pending = cli(&socket, "status", false).await;
        assert!(human_pending.status.success());
        let human_text = std::str::from_utf8(&human_pending.stdout).unwrap();
        match &expected {
            Some(url) => {
                assert!(human_text.contains("Approve this device in your browser:"));
                assert!(human_text.lines().any(|line| line == url.as_str()));
                assert!(!human_text.contains("ask its operator"));
            }
            None => {
                for bytes in [&output.stdout, &json.stdout] {
                    let value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
                    assert_eq!(value.get("enrollmentUrl"), Some(&serde_json::Value::Null));
                }
                assert!(human_text.contains("Awaiting approval."));
                assert!(human_text.contains("The Server has not configured a Console origin"));
                assert!(human_text.contains("ask its operator to configure --console-origin"));
                assert!(human_text.contains("MESHWAY_CONSOLE_ORIGIN"));
                assert!(!human_text.contains("Approve this device in your browser:"));
                // The only URL is this Node's own Server origin, never a link.
                let without_server = human_text.replace(&origin, "");
                assert!(
                    !without_server.contains("http://") && !without_server.contains("https://")
                );
            }
        }
        for result in [&output, &json, &human_pending] {
            let stdout = std::str::from_utf8(&result.stdout).unwrap();
            let stderr = std::str::from_utf8(&result.stderr).unwrap();
            for private in [
                owner.token.expose(),
                "CLI fixture password",
                "mwns_",
                "privateKey",
            ] {
                assert!(!stdout.contains(private) && !stderr.contains(private));
            }
            assert!(!stderr.contains("authorize-device/"));
            if let Some(url) = &expected {
                assert!(!stderr.contains(url));
            }
        }
        let approved = api
            .approve_device_authorization(
                &ApproveDeviceAuthorizationPath {
                    enrollment_request_id: enrollment,
                },
                &ApproveEnrollmentRequest {
                    network_id: network.network.id,
                },
                owner.token.expose(),
            )
            .await
            .unwrap();
        assert_eq!(approved.request.authorization_url, expected);
    }
    let ready = status(&socket, Phase::ControlReady).await;
    assert!(ready.authenticated && ready.saved.is_some() && ready.applied.is_none());
    let human = cli(&socket, "status", false).await;
    assert!(human.status.success());
    let human_text = String::from_utf8_lossy(&human.stdout);
    assert!(human_text.contains("  Data plane control only · mode control-only\n"));
    assert!(human_text.contains(" · not applied"));
    assert!(!human_text.contains("mwns_") && !human_text.contains('\u{1b}'));
    assert!(!human_text.contains("ask its operator"));
    assert!(human_text.contains("  Sync       "));
    // control-only has no packet plane to check; the Server answers.
    let netcheck = cli(&socket, "netcheck", true).await;
    assert!(netcheck.status.success());
    let netcheck: meshway_protocol::ipc::NetcheckReport =
        serde_json::from_slice(&netcheck.stdout).unwrap();
    assert_eq!(
        netcheck.unavailable,
        Some(meshway_protocol::ipc::NetcheckUnavailable::NotRunning)
    );
    assert_eq!(netcheck.server.reachable, Some(true));
    assert!(netcheck.server.rtt_us.is_some());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = meshway_protocol::ipc::Client::new(&socket)
            .call(meshway_protocol::ipc::Command::Status)
            .await
            .unwrap();
        if state.sync == meshway_protocol::ipc::SyncState::Connected && state.last_sync_at.is_some()
        {
            break;
        }
        assert!(Instant::now() < deadline, "sync did not connect");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let ready = if key_stdin == Some(false) {
        let rotated = cli(&socket, "rotate-key", true).await;
        assert!(
            rotated.status.success(),
            "{}",
            String::from_utf8_lossy(&rotated.stderr)
        );
        let rotated: Status = serde_json::from_slice(&rotated.stdout).unwrap();
        let before = ready.identity.as_ref().unwrap();
        let after = rotated.identity.as_ref().unwrap();
        assert_eq!(before.node_id, after.node_id);
        assert_eq!(
            before.keys.control.generation.get() + 1,
            after.keys.control.generation.get()
        );
        status(&socket, Phase::ControlReady).await
    } else {
        ready
    };
    let down = cli(&socket, "down", true).await;
    assert!(down.status.success());
    let down: Status = serde_json::from_slice(&down.stdout).unwrap();
    assert_eq!(down.phase, Phase::Down);
    assert!(!down.desired_up && !down.authenticated && down.instance.is_none());
    assert_eq!(down.identity, ready.identity);
    // Logout deletes this Node on the Server and forgets the identity.
    let node = ready.identity.as_ref().unwrap().node_id;
    let logout = cli(&socket, "logout", true).await;
    assert!(
        logout.status.success(),
        "{}",
        String::from_utf8_lossy(&logout.stderr)
    );
    let left: Status = serde_json::from_slice(&logout.stdout).unwrap();
    assert!(left.identity.is_none() && !left.desired_up);
    let gone = api
        .get_node(
            &meshway_control_client::GetNodePath {
                network_id: network.network.id,
                node_id: node,
            },
            owner.token.expose(),
        )
        .await;
    assert!(gone.is_err());
    let after = status(&socket, Phase::Down).await;
    assert!(after.identity.is_none() && after.saved.is_none());
    assert!(!root.join("daemon/identity/netmap.json").exists());
    // Nothing is left to delete: a second logout fails without a Server call.
    assert!(!cli(&socket, "logout", false).await.status.success());
    stop.send(()).unwrap();
    daemon.await.unwrap().unwrap();
    assert!(!socket.exists());
    server.stream_shutdown()();
    server_stop.send(()).unwrap();
    server_task.await.unwrap();
    server.shutdown().await.unwrap();
}

async fn key_enrollment(
    api: &Client,
    network: meshway_model::NetworkId,
    token: &str,
    root: &Path,
    socket: &Path,
    stdin: bool,
) {
    use meshway_control_client::{
        ApplyConfigurationPath, CreateEnrollmentKeyPath, GetConfigurationSourcePath,
        GetConfigurationSourceQuery,
    };
    use meshway_protocol::{
        ConfigurationRequest, CreateEnrollmentKeyRequest, enrollment::InitialNodeSettings,
    };
    let source = api
        .get_configuration_source(
            &GetConfigurationSourcePath {
                network_id: network,
            },
            &GetConfigurationSourceQuery::default(),
            token,
        )
        .await
        .unwrap();
    let mut input = serde_json::to_value(&source.configuration).unwrap();
    input["tagsOwners"] = serde_json::json!({"tag:service":[]});
    api.apply_configuration(
        &ApplyConfigurationPath {
            network_id: network,
        },
        &ConfigurationRequest::Document {
            expected_revision: source.revision,
            request_id: id(),
            source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
        },
        token,
    )
    .await
    .unwrap();
    let key = api
        .create_enrollment_key(
            &CreateEnrollmentKeyPath {
                network_id: network,
            },
            &CreateEnrollmentKeyRequest {
                request_id: id(),
                tags: vec!["tag:service".into()],
                initial_settings: InitialNodeSettings {
                    hostname: None,
                    listeners: Default::default(),
                },
                expires_at_ms: u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis(),
                )
                .unwrap()
                    + 300_000,
            },
            token,
        )
        .await
        .unwrap()
        .secret
        .unwrap();
    let path = root.join("enrollment-key");
    {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", key.expose()).unwrap();
    }
    let socket = socket.to_owned();
    let text = key.expose().to_owned();
    let output = tokio::task::spawn_blocking(move || {
        use std::{io::Write, process::Stdio};
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_meshway"));
        command.arg("--socket").arg(socket).args([
            "--log-format",
            "json",
            "--log-filter",
            "debug",
            "up",
            "--json",
            "--enrollment-key",
        ]);
        if stdin {
            command.arg("-");
        } else {
            command.arg(format!("@{}", path.display()));
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if stdin {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
        }
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for bytes in [&output.stdout, &output.stderr] {
        assert!(!String::from_utf8_lossy(bytes).contains(key.expose()));
    }
    let state: Status = serde_json::from_slice(&output.stdout).unwrap();
    assert!(state.identity.is_some());
    assert!(state.enrollment_url.is_none());
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let record = std::fs::read_to_string(root.join("daemon/daemon.json")).unwrap();
        if !record.contains(key.expose()) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "retired enrollment secret still persisted"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[test]
fn logging_options_preserve_json_stdout_and_filter_stderr() {
    let temp = tempfile::tempdir().unwrap();
    let run = |filter: Option<&str>| {
        let mut process = std::process::Command::new(env!("CARGO_BIN_EXE_meshway"));
        process
            .env("RUST_LOG", "error")
            .env("MESHWAY_LOG_FORMAT", "text")
            .args(["status", "--json", "--log-format", "json", "--socket"])
            .arg(temp.path().join("absent.sock"));
        if let Some(filter) = filter {
            process.args(["--log-filter", filter]);
        }
        process.output().unwrap()
    };
    let filtered = run(None);
    assert!(!filtered.status.success() && filtered.stdout.is_empty());
    let logs: Vec<serde_json::Value> = String::from_utf8(filtered.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0]["level"], "ERROR");
    let debug = run(Some("debug"));
    assert!(!debug.status.success() && debug.stdout.is_empty());
    let logs: Vec<serde_json::Value> = String::from_utf8(debug.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(logs.iter().any(|event| event["level"] == "DEBUG"));
    assert!(logs.iter().any(|event| event["level"] == "ERROR"));
    let invalid = run(Some("meshway_cli=not-a-level"));
    assert!(!invalid.status.success() && invalid.stdout.is_empty());
    assert!(
        String::from_utf8(invalid.stderr)
            .unwrap()
            .contains("Invalid log filter")
    );
}

#[test]
fn version_reports_build_and_protocol_identity_without_a_daemon() {
    let temp = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_meshway"))
        .args(["version", "--json", "--socket"])
        .arg(temp.path().join("absent.sock"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let version: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(version["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(version["ipc"], meshway_protocol::ipc::VERSION);
    assert_eq!(version["compilerVersion"], "meshway-netmap-1");
    assert!(version["wire"].as_u64().is_some_and(|wire| wire > 0));
    assert!(version["git"].as_str().is_some_and(|git| !git.is_empty()));
}
