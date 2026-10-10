//! CLI/IPC transport fixtures. Mesh packet success is verified by Linux integration.
#![cfg(unix)]

use meshway_model::Sequence;
use meshway_protocol::ipc::{
    Command, PingEvent, PingTarget, Request, Response, ResultBody, VERSION,
};
use serde_json::json;
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Output, Stdio},
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    net::{UnixListener, UnixStream},
};

const ID: &str = "00000000-0000-4000-8000-000000000001";

fn accepted() -> PingEvent {
    serde_json::from_value(json!({"type":"accepted","subscriptionId":ID,"observationId":ID,"target":{"nodeId":ID,"hostname":"peer.mesh","addresses":{"ipv4":"10.0.0.2","ipv6":null}}})).unwrap()
}

fn fixture() -> (tempfile::TempDir, UnixListener) {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("control.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    (directory, listener)
}

fn command(root: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_meshway"));
    command
        .arg("--socket")
        .arg(root.join("control.sock"))
        .arg("ping");
    command
}

async fn cli(mut command: std::process::Command) -> Output {
    tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap()
}

async fn send(stream: &mut UnixStream, request: &Request, sequence: u64, event: PingEvent) {
    meshway_protocol::ipc::write(
        stream,
        &Response {
            version: VERSION,
            request_id: request.request_id,
            result: ResultBody::Ping {
                sequence: Sequence::new(sequence).unwrap(),
                event,
            },
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn json_stream_uses_typed_target_default_count_and_success_exit_status() {
    let (directory, listener) = fixture();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request: Request = meshway_protocol::ipc::read(&mut stream).await.unwrap();
        assert!(
            matches!(&request.command, Command::Ping { target: PingTarget::Name { name }, count: 10 } if name == "peer.mesh")
        );
        send(&mut stream, &request, 0, accepted()).await;
        send(
            &mut stream,
            &request,
            1,
            PingEvent::Preparing {
                elapsed_ms: Sequence::new(1523).unwrap(),
            },
        )
        .await;
        send(
            &mut stream,
            &request,
            2,
            PingEvent::Completed {
                sent: 10,
                success: 9,
                timeout: 1,
                failed: 0,
            },
        )
        .await;
        assert_eq!(stream.read(&mut [0]).await.unwrap(), 0);
    });
    let mut command = command(directory.path());
    command.args(["peer.mesh", "--json"]);
    let output = cli(command).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events: Vec<PingEvent> = std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 3);
    assert!(matches!(events[0], PingEvent::Accepted { .. }));
    assert!(matches!(events[1], PingEvent::Preparing { .. }));
    assert!(matches!(events[2], PingEvent::Completed { success: 9, .. }));
    server.await.unwrap();
}

#[tokio::test]
async fn ip_target_count_flags_timeout_and_terminal_failures_exit_unsuccessfully() {
    for (flag, failure) in [("-c", false), ("--count", true)] {
        let (directory, listener) = fixture();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request: Request = meshway_protocol::ipc::read(&mut stream).await.unwrap();
            assert!(
                matches!(&request.command, Command::Ping { target: PingTarget::MeshIp { address }, count: 1 } if address == "fd00::2")
            );
            if failure {
                send(
                    &mut stream,
                    &request,
                    0,
                    PingEvent::Failed {
                        code: meshway_protocol::ipc::PingError::PermissionDenied,
                    },
                )
                .await;
            } else {
                send(&mut stream, &request, 0, accepted()).await;
                send(
                    &mut stream,
                    &request,
                    1,
                    PingEvent::Completed {
                        sent: 1,
                        success: 0,
                        timeout: 1,
                        failed: 0,
                    },
                )
                .await;
            }
        });
        let mut command = command(directory.path());
        command.args(["fd00::2", flag, "1"]);
        let output = cli(command).await;
        assert_eq!(output.status.code(), Some(1));
        let text = std::str::from_utf8(&output.stdout).unwrap();
        assert!(text.contains(if failure {
            "PermissionDenied"
        } else {
            "sent=1 replied=0 timeout=1 local-failed=0"
        }));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn invalid_count_and_timeout_option_fail_argument_parsing() {
    let directory = tempfile::tempdir().unwrap();
    for args in [["--count", "0"], ["-c", "1001"], ["--timeout", "5"]] {
        let mut command = command(directory.path());
        command.arg("peer.mesh").args(args);
        assert_eq!(cli(command).await.status.code(), Some(2));
    }
}

#[tokio::test]
async fn ctrl_c_keeps_existing_exit_code_and_closes_subscription() {
    let (directory, listener) = fixture();
    let mut command = command(directory.path());
    command
        .args(["peer.mesh", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn().unwrap();
    let pid = child.id();
    let (mut stream, _) = listener.accept().await.unwrap();
    let request: Request = meshway_protocol::ipc::read(&mut stream).await.unwrap();
    send(&mut stream, &request, 0, accepted()).await;
    assert!(
        std::process::Command::new("/bin/kill")
            .args(["-INT", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    let output = tokio::task::spawn_blocking(move || child.wait_with_output().unwrap())
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(130));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), stream.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn eof_without_terminal_is_failure() {
    let (directory, listener) = fixture();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = meshway_protocol::ipc::read(&mut stream).await.unwrap();
        send(&mut stream, &request, 0, accepted()).await;
    });
    let mut command = command(directory.path());
    command.args(["peer.mesh", "--json"]);
    assert_eq!(cli(command).await.status.code(), Some(1));
    server.await.unwrap();
}
