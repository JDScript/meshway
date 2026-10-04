#![cfg(unix)]

use std::{
    io::{BufRead, BufReader, Read},
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use meshway_controller::{App, Settings};
use meshway_protocol::AuthOptionsResponse;

struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn command(path: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_meshway-controller"));
    command
        .env_remove("MESHWAY_DATA_DIR")
        .env_remove("MESHWAY_LISTEN")
        .env_remove("MESHWAY_REGISTRATION_ENABLED")
        .env_remove("MESHWAY_SESSION_TTL_SECONDS")
        .arg("--data-dir")
        .arg(path)
        .arg("--listen")
        .arg("127.0.0.1:0")
        .stdin(Stdio::null());
    command
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn executable_serves_and_gracefully_releases_storage_on_both_signals() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("cli-data");
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    for signal in ["-INT", "-TERM"] {
        let mut process = Process(
            command(&path)
                .env("MESHWAY_REGISTRATION_ENABLED", "false")
                .env("MESHWAY_SESSION_TTL_SECONDS", "60")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let stdout = process.0.stdout.take().unwrap();
        let (send, receive) = mpsc::channel();
        let output = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            send.send(line.clone()).unwrap();
            reader.read_to_string(&mut line).unwrap();
            line
        });
        let line = receive.recv_timeout(Duration::from_secs(10)).unwrap();
        let address = line.trim().strip_prefix("Listening on ").unwrap();
        let options: AuthOptionsResponse = client
            .get(format!("http://{address}/api/v1/auth/options"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(!options.registration_enabled);
        assert!(
            Command::new("kill")
                .args([signal, &process.0.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "CLI shutdown timed out");
            thread::sleep(Duration::from_millis(20));
        }
        let output = output.join().unwrap();
        assert!(output.ends_with("Controller stopped.\n"));
        assert!(!output.contains("mws_"));
        let mut errors = String::new();
        process
            .0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut errors)
            .unwrap();
        assert!(errors.is_empty());
        App::open(Settings::new(&path))
            .await
            .unwrap()
            .shutdown()
            .await
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_event_streams_do_not_block_signal_shutdown_or_storage_release() {
    let temp = tempfile::tempdir().unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    for (index, signal) in ["-INT", "-TERM"].into_iter().enumerate() {
        let path = temp
            .path()
            .canonicalize()
            .unwrap()
            .join(format!("sse-{index}"));
        let mut process = Process(
            command(&path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let stdout = process.0.stdout.take().unwrap();
        let (send, receive) = mpsc::channel();
        let output = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            send.send(line.clone()).unwrap();
            reader.read_to_string(&mut line).unwrap();
            line
        });
        let line = receive.recv_timeout(Duration::from_secs(10)).unwrap();
        let address = line.trim().strip_prefix("Listening on ").unwrap();
        let base = format!("http://{address}");
        let session: meshway_protocol::SessionIssuedResponse = client
            .post(format!("{base}/api/v1/auth/register"))
            .timeout(Duration::from_secs(5))
            .json(&meshway_protocol::RegisterRequest {
                email: "shutdown@example.test".into(),
                password: meshway_protocol::WireSecret::new("valid shutdown test password".into()),
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let network: meshway_protocol::NetworkResponse = client
            .post(format!("{base}/api/v1/networks"))
            .timeout(Duration::from_secs(5))
            .bearer_auth(session.token.expose())
            .json(&meshway_protocol::CreateNetworkRequest {
                request_id: "12345678-1234-4321-8765-123456789abc".parse().unwrap(),
                name: "Shutdown stream".into(),
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        // Deliberately no response-body timeout: it must be the server that ends
        // this live stream, not the test client cancelling it before shutdown.
        let mut events = tokio::time::timeout(
            Duration::from_secs(5),
            client
                .get(format!(
                    "{base}/api/v1/networks/{}/events",
                    network.network.id
                ))
                .bearer_auth(session.token.expose())
                .send(),
        )
        .await
        .unwrap()
        .unwrap()
        .error_for_status()
        .unwrap();
        assert_eq!(events.headers()["content-type"], "text/event-stream");
        assert!(
            tokio::time::timeout(Duration::from_secs(5), events.chunk())
                .await
                .unwrap()
                .unwrap()
                .is_some()
        );
        assert!(
            Command::new("kill")
                .args([signal, &process.0.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(
                Instant::now() < deadline,
                "open SSE blocked graceful shutdown"
            );
            thread::sleep(Duration::from_millis(20));
        }
        assert!(output.join().unwrap().ends_with("Controller stopped.\n"));
        let mut errors = String::new();
        process
            .0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut errors)
            .unwrap();
        assert!(errors.is_empty());
        drop(events);
        App::open(Settings::new(&path))
            .await
            .unwrap()
            .shutdown()
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn bind_failure_and_invalid_storage_exit_without_taking_over_another_instance() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("bind-data");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_meshway-controller"))
        .env_remove("MESHWAY_SESSION_TTL_SECONDS")
        .arg("--data-dir")
        .arg(&path)
        .arg("--listen")
        .arg(listener.local_addr().unwrap().to_string())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "Controller listen failed.\n"
    );
    assert!(output.stdout.is_empty());
    App::open(Settings::new(&path))
        .await
        .unwrap()
        .shutdown()
        .await
        .unwrap();

    let output = command(Path::new("relative-sensitive-path"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "Controller storage startup failed.\n"
    );
}

#[test]
fn export_binary_needs_no_storage_and_matches_live_route_schema() {
    let output = Command::new(env!("CARGO_BIN_EXE_export-openapi"))
        .env_remove("MESHWAY_DATA_DIR")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let spec: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        spec,
        serde_json::to_value(meshway_controller::http::openapi()).unwrap()
    );
}
