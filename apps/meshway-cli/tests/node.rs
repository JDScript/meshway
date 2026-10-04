#![cfg(unix)]
use meshway_control_client::{ApproveDeviceAuthorizationPath, Client};
use meshway_ipc::{Phase, Status};
use meshway_model::RequestId;
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
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let server = meshway_server::App::open(meshway_server::Settings::new(root.join("server")))
        .await
        .unwrap();
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
        reqwest::Client::builder()
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
    let settings = meshway_daemon::Settings::new(
        path.clone(),
        &origin,
        Some("http://127.0.0.1:3001"),
        Some("cli-node"),
    )
    .unwrap()
    .with_network_mode(meshway_ipc::NetworkMode::ControlOnly);
    let (stop, stopped) = oneshot::channel();
    let daemon = tokio::spawn(meshway_daemon::run(settings, async {
        let _ = stopped.await;
    }));
    status(&socket, Phase::Down).await;
    let output = cli(&socket, "up", true).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pending: Status = serde_json::from_slice(&output.stdout).unwrap();
    let logs: Vec<serde_json::Value> = String::from_utf8(output.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(!logs.is_empty());
    assert!(logs.iter().any(|event| event["fields"]["command"] == "up"));
    assert_eq!(pending.phase, Phase::AwaitingApproval);
    let enrollment = pending.enrollment_request_id.unwrap();
    assert_eq!(
        pending.enrollment_url.unwrap(),
        format!("http://127.0.0.1:3001/authorize-device/{enrollment}")
    );
    api.approve_device_authorization(
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
    let ready = status(&socket, Phase::ControlReady).await;
    assert!(ready.authenticated && ready.saved.is_some() && ready.applied.is_none());
    let human = cli(&socket, "status", false).await;
    assert!(human.status.success());
    assert!(
        String::from_utf8_lossy(&human.stdout)
            .contains("Network mode: ControlOnly  Data plane: ControlOnly")
    );
    assert!(!String::from_utf8_lossy(&human.stdout).contains("Applied configuration:"));
    assert!(!String::from_utf8_lossy(&human.stdout).contains("mwns_"));
    let down = cli(&socket, "down", true).await;
    assert!(down.status.success());
    let down: Status = serde_json::from_slice(&down.stdout).unwrap();
    assert_eq!(down.phase, Phase::Down);
    assert!(!down.desired_up && !down.authenticated && down.instance.is_none());
    assert_eq!(down.identity, ready.identity);
    stop.send(()).unwrap();
    daemon.await.unwrap().unwrap();
    assert!(!socket.exists());
    server.stream_shutdown()();
    server_stop.send(()).unwrap();
    server_task.await.unwrap();
    server.shutdown().await.unwrap();
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
