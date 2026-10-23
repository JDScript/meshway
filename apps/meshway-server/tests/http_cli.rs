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

use meshway_protocol::AuthOptionsResponse;
use meshway_server::{App, Settings};

struct Process(Child);

/// The shutdown user and its network, prepared in process with the offline
/// connector. The network's configuration disables the built-in rule-set
/// providers, so the executable later started on `path` (which uses the
/// production connector) has no catalog to fetch: the test stays off the
/// Internet without a Server setting for it.
async fn prepared(path: &Path) -> meshway_model::NetworkId {
    let app = App::open_with_connector(
        Settings::new(path),
        std::sync::Arc::new(meshway_server::resources::download::Offline),
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = meshway_server::http::router(&app);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured(meshway_transport_stream::tls::platform_client_config().unwrap())
        .no_proxy()
        .build()
        .unwrap();
    let session: meshway_protocol::SessionIssuedResponse = client
        .post(format!("{base}/api/v1/auth/register"))
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
    let id = network.network.id;
    let source: meshway_protocol::configuration::ConfigurationSourceResponse = client
        .get(format!("{base}/api/v1/networks/{id}/configuration/source"))
        .bearer_auth(session.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut configuration = source.configuration;
    for (provider, _) in meshway_configuration::providers::BUILTIN_PROVIDERS {
        configuration
            .rule_set_providers
            .insert(provider.to_owned(), None);
    }
    client
        .post(format!("{base}/api/v1/networks/{id}/configuration"))
        .bearer_auth(session.token.expose())
        .json(
            &meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: source.revision,
                request_id: "12345678-1234-4321-8765-123456789abd".parse().unwrap(),
                source_jsonc: meshway_protocol::WireSecret::new(
                    serde_json::to_string(&configuration).unwrap(),
                ),
            },
        )
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let _ = stop.send(());
    server.await.unwrap();
    app.shutdown().await.unwrap();
    id
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn command(path: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_meshway-server"));
    command
        .env_remove("MESHWAY_DATA_DIR")
        .env_remove("MESHWAY_LISTEN")
        .env_remove("MESHWAY_REGISTRATION_ENABLED")
        .env_remove("MESHWAY_SESSION_TTL_SECONDS")
        .args([
            "--log-format",
            "json",
            "--log-filter",
            "info,meshway_server=trace,tower_http=trace",
        ])
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
        .tls_backend_preconfigured(meshway_transport_stream::tls::platform_client_config().unwrap())
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
        let response = client
            .get(format!(
                "http://{address}/api/v1/auth/options?token=LOG_QUERY_SECRET"
            ))
            .bearer_auth("LOG_HEADER_SECRET")
            .header("x-request-id", "LOG_CLIENT_CORRELATION")
            .send()
            .await
            .unwrap();
        let request_id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let options: AuthOptionsResponse = response.json().await.unwrap();
        assert!(!options.registration_enabled);
        let response = client
            .get(format!("http://{address}/api/v1/networks/LOG_PATH_SECRET"))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_client_error());
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
        assert!(output.ends_with("Server stopped.\n"));
        assert!(!output.contains("mws_"));
        let mut errors = String::new();
        process
            .0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut errors)
            .unwrap();
        let events = log_events(&errors);
        assert!(
            events
                .iter()
                .any(|event| event["fields"]["message"] == "Server ready")
        );
        assert!(
            events
                .iter()
                .any(|event| event["fields"]["message"] == "Server stopped")
        );
        assert!(
            events
                .iter()
                .any(|event| event["span"]["request_id"] == request_id
                    && event["span"]["route"] == "/api/v1/auth/options"
                    && event["fields"]["status"] == 200)
        );
        assert!(
            events
                .iter()
                .any(|event| event["span"]["route"] == "/api/v1/networks/{networkId}")
        );
        for secret in [
            "LOG_QUERY_SECRET",
            "LOG_HEADER_SECRET",
            "LOG_CLIENT_CORRELATION",
            "LOG_PATH_SECRET",
        ] {
            assert!(!errors.contains(secret));
        }
        App::open_with_connector(
            Settings::new(&path),
            std::sync::Arc::new(meshway_server::resources::download::Offline),
        )
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
        .tls_backend_preconfigured(meshway_transport_stream::tls::platform_client_config().unwrap())
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
        let network = prepared(&path).await;
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
            .post(format!("{base}/api/v1/auth/login"))
            .timeout(Duration::from_secs(5))
            .json(&meshway_protocol::LoginRequest {
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
        // Deliberately no response-body timeout: it must be the server that ends
        // this live stream, not the test client cancelling it before shutdown.
        let mut events = tokio::time::timeout(
            Duration::from_secs(5),
            client
                .get(format!("{base}/api/v1/networks/{network}/events"))
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
        assert!(output.join().unwrap().ends_with("Server stopped.\n"));
        let mut errors = String::new();
        process
            .0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut errors)
            .unwrap();
        let events = log_events(&errors);
        assert!(
            events
                .iter()
                .any(|event| event["fields"]["message"] == "Server stopped")
        );
        assert!(!errors.contains(session.token.expose()));
        assert!(!errors.contains("valid shutdown test password"));
        assert!(!errors.contains("shutdown@example.test"));
        drop(events);
        App::open_with_connector(
            Settings::new(&path),
            std::sync::Arc::new(meshway_server::resources::download::Offline),
        )
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
    let output = Command::new(env!("CARGO_BIN_EXE_meshway-server"))
        .env_remove("MESHWAY_SESSION_TTL_SECONDS")
        .args(["--log-format", "json", "--log-filter", "info"])
        .arg("--data-dir")
        .arg(&path)
        .arg("--listen")
        .arg(listener.local_addr().unwrap().to_string())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let errors = log_events(&String::from_utf8(output.stderr).unwrap());
    assert!(
        errors.iter().any(|event| event["level"] == "ERROR"
            && event["fields"]["error"] == "Server listen failed.")
    );
    assert!(output.stdout.is_empty());
    App::open_with_connector(
        Settings::new(&path),
        std::sync::Arc::new(meshway_server::resources::download::Offline),
    )
    .await
    .unwrap()
    .shutdown()
    .await
    .unwrap();

    let output = command(Path::new("relative-sensitive-path"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let errors = log_events(&String::from_utf8(output.stderr).unwrap());
    assert!(errors.iter().any(|event| {
        event["level"] == "ERROR"
            && event["fields"]["error"]
                == "Server storage startup failed: data directory must be an absolute path \
                    without parent traversal."
    }));
}

fn log_events(logs: &str) -> Vec<serde_json::Value> {
    assert!(!logs.contains('\u{1b}'));
    logs.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
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
        serde_json::to_value(meshway_server::http::openapi()).unwrap()
    );
}

/// Storage that a restart cannot change exits with 78 (`EX_CONFIG`), so a
/// service manager does not restart into it. Only `migrate` and serving
/// convert an earlier schema; other maintenance refuses it first.
#[tokio::test]
async fn migrate_and_persistent_storage_blocks_have_their_own_exit_status() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    App::open_with_connector(
        Settings::new(&path),
        std::sync::Arc::new(meshway_server::resources::download::Offline),
    )
    .await
    .unwrap()
    .shutdown()
    .await
    .unwrap();
    let maintenance = |arguments: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_meshway-server"))
            .env_remove("MESHWAY_DATA_DIR")
            .env("MESHWAY_LOG_FORMAT", "json")
            .env("RUST_LOG", "info")
            .args(arguments)
            .arg("--data-dir")
            .arg(&path)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let output = maintenance(&["migrate"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Migration state: current\nDatabase is already at this build's schema; nothing migrated.\n"
    );
    let backup = temp.path().canonicalize().unwrap().join("backup");
    let output = maintenance(&["backup", "--output", backup.to_str().unwrap()]);
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .starts_with("Backed up the database (schema 9, "),
    );
    assert!(backup.join("master-key").exists());
    let output = maintenance(&["backup", "--output", backup.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "an existing output is no storage block"
    );
    let set_version = |version: i64| {
        let db = rusqlite::Connection::open(path.join("controller.sqlite")).unwrap();
        db.pragma_update(None, "user_version", version).unwrap();
    };
    let failed = |output: std::process::Output, message: &str| {
        assert_eq!(output.status.code(), Some(78));
        let errors = log_events(&String::from_utf8(output.stderr).unwrap());
        assert!(
            errors
                .iter()
                .any(|event| event["level"] == "ERROR" && event["fields"]["error"] == message),
            "{errors:?}"
        );
    };
    // Newer than this build.
    set_version(99);
    failed(
        maintenance(&["migrate"]),
        "Server storage startup failed: database application or schema version is unsupported.",
    );
    failed(
        command(&path).output().unwrap(),
        "Server storage startup failed: database application or schema version is unsupported.",
    );
    // Claims an earlier version but is not what that version wrote.
    set_version(8);
    failed(
        maintenance(&["rotate-netmap-key"]),
        "Server storage startup failed: database has an earlier schema version; run \
         `meshway-server migrate` with the Server stopped first.",
    );
    let refused = "Server storage startup failed: database has an earlier schema version; run \
                   `meshway-server migrate` with the Server stopped first.";
    failed(
        maintenance(&[
            "backup",
            "--output",
            temp.path().join("old").to_str().unwrap(),
        ]),
        refused,
    );
    let offline = |binary: &str, arguments: &[&str]| {
        Command::new(binary)
            .env("MESHWAY_LOG_FORMAT", "json")
            .env("RUST_LOG", "info")
            .arg("--data-dir")
            .arg(&path)
            .args(arguments)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    failed(
        offline(
            env!("CARGO_BIN_EXE_reset-password"),
            &[
                "--email",
                "user@example.test",
                "--console-origin",
                "https://console.example.test",
            ],
        ),
        refused,
    );
    failed(
        offline(env!("CARGO_BIN_EXE_ip-datasets"), &["status"]),
        refused,
    );
    failed(
        maintenance(&["migrate"]),
        "Server storage startup failed: database structure does not match the current schema: \
         unexpected table schema_migrations.",
    );
}

/// An upgrade compares a data directory's schema with this range before it
/// stops the running Server.
#[test]
fn version_names_the_build_and_its_schema_range() {
    let output = Command::new(env!("CARGO_BIN_EXE_meshway-server"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(
        lines[0],
        format!("meshway-server {}", meshway_build::identity())
    );
    assert_eq!(
        lines[1],
        format!(
            "schema: {} (migrates from {})",
            meshway_server::SCHEMA_VERSION,
            meshway_server::MIN_MIGRATABLE_SCHEMA
        )
    );
}

/// Each maintenance command's help line describes that command (PROD-1:
/// `backup` showed the key rotation text and `rotate-netmap-key` none).
#[test]
fn help_describes_each_maintenance_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_meshway-server"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let summary = |command: &str| {
        text.lines()
            .find_map(|line| line.trim_start().strip_prefix(command))
            .map(str::trim)
            .unwrap_or_else(|| panic!("{command} is listed: {text}"))
            .to_owned()
    };
    assert!(summary("migrate ").starts_with("Migrate the database"));
    assert!(summary("backup ").starts_with("Copy the database and the master key"));
    assert!(summary("rotate-netmap-key ").starts_with("Start a netmap signing-key rotation"));
}

/// `Type=notify`: the start timeout renewed while the storage opens,
/// readiness once it is open and the listener bound, and the stop as it begins.
#[cfg(target_os = "linux")]
#[test]
fn the_service_manager_hears_readiness_and_the_stop() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let socket = root.join("notify");
    let receiver = std::os::unix::net::UnixDatagram::bind(&socket).unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let mut process = Process(
        command(&root.join("data"))
            .env("NOTIFY_SOCKET", &socket)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut stdout = BufReader::new(process.0.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let address = line
        .trim()
        .strip_prefix("Listening on ")
        .unwrap()
        .to_owned();
    let mut buffer = [0; 256];
    let mut renewals = 0;
    loop {
        let length = receiver.recv(&mut buffer).unwrap();
        let message = std::str::from_utf8(&buffer[..length]).unwrap().to_owned();
        if message.starts_with("READY=1") {
            assert_eq!(message, format!("READY=1\nSTATUS=Serving on {address}"));
            break;
        }
        assert!(
            message.starts_with("EXTEND_TIMEOUT_USEC=120000000\nSTATUS="),
            "{message}"
        );
        renewals += 1;
    }
    // At least the renewal when the opening starts.
    assert!(renewals >= 1);
    assert!(
        Command::new("kill")
            .args(["-TERM", &process.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let length = receiver.recv(&mut buffer).unwrap();
    assert_eq!(&buffer[..length], b"STOPPING=1");
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    assert!(process.0.wait().unwrap().success());
}
