#![cfg(all(feature = "managed-connections", unix))]

use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Output};

use meshway_agent::diagnostics::{
    self, Evidence, Incoming, LiveResponse, LiveState, LiveStatus, Operation, ProbeError,
    ProbeObservation, ProbeResponse, PublicIdentity, SCHEMA_VERSION, WhoisObservation,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
    process::Command,
};

async fn cli(state: &Path, args: &[&str]) -> Output {
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
            .args(args)
            .env("MESHWAY_STATE", state)
            .output(),
    )
    .await
    .unwrap()
    .unwrap()
}

fn directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("mw")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

fn identity() -> PublicIdentity {
    PublicIdentity {
        node_id: "a".repeat(64),
        name: "<SHA>\u{001b}[31m\u{0085}\u{202e}".into(),
        mesh_ipv4: "100.64.0.1".into(),
        mesh_ipv6: "fd00::1".into(),
    }
}

// Real UDS framing and subprocess CLI, using the same typed DTO as the daemon.
// Runtime/inventory lease and replacement behavior are exercised separately
// against real managed UDP channels in the Prober tests.
async fn fixture(state: &Path) -> tokio::task::JoinHandle<()> {
    let path = diagnostics::socket_path(state);
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let length = socket.read_u32().await.unwrap();
            assert!(length <= 4096);
            let mut bytes = vec![0; length as usize];
            socket.read_exact(&mut bytes).await.unwrap();
            let request: Incoming = serde_json::from_slice(&bytes).unwrap();
            let request = match request {
                Incoming::Versioned(request) => request,
                Incoming::Legacy(request) => {
                    request.validate().unwrap();
                    let response = if request.peer == "SHA" {
                        ProbeResponse::Ok(ProbeObservation {
                            generation: 7,
                            installed_revision: 3,
                            source_revision: 3,
                            evidence: Evidence::DirectAuthenticatedNeighborRoundTrip,
                            peer_node_id: "b".repeat(64),
                            peer_name: Some(identity().name),
                            local_node_id: "a".repeat(64),
                            initiator_node_id: "b".repeat(64),
                            transport: meshway_protocol::ConnectionTransport::NativeUdp,
                            rtt_ns: 123_456,
                        })
                    } else {
                        ProbeResponse::Error(ProbeError::UnconfiguredPeer)
                    };
                    let body = serde_json::to_vec(&response).unwrap();
                    socket.write_u32(body.len() as u32).await.unwrap();
                    socket.write_all(&body).await.unwrap();
                    continue;
                }
            };
            assert_eq!(request.schema_version, SCHEMA_VERSION);
            let response = match request.operation {
                Operation::Status {
                    peers,
                    offset,
                    limit,
                } => {
                    assert_eq!(offset, 0);
                    assert_eq!(limit, 64);
                    LiveResponse::Status(LiveStatus {
                        schema_version: SCHEMA_VERSION,
                        state: LiveState::Installed,
                        generation: 7,
                        installed_revision: 3,
                        source_revision: 3,
                        lease_remaining_ms: 5000,
                        local_node_id: "a".repeat(64),
                        local_identity: Some(identity()),
                        network_id: None,
                        authenticated_channel_count: 0,
                        peers: peers.then(Vec::new),
                        peer_offset: offset,
                        next_peer_offset: None,
                        evidence: Evidence::CurrentAuthenticatedChannelRegistry,
                    })
                }
                Operation::Whois { query } => {
                    assert!(matches!(query.as_str(), "SHA" | "100.64.0.1" | "fd00::1"));
                    LiveResponse::Whois(WhoisObservation {
                        schema_version: SCHEMA_VERSION,
                        generation: 7,
                        installed_revision: 3,
                        source_revision: 3,
                        identity: identity(),
                        evidence: Evidence::CurrentTrustedInventory,
                    })
                }
            };
            let body = serde_json::to_vec(&response).unwrap();
            socket.write_u32(body.len() as u32).await.unwrap();
            socket.write_all(&body).await.unwrap();
        }
    })
}

fn json(output: &Output) -> serde_json::Value {
    assert_eq!(
        output.stdout.iter().filter(|byte| **byte == b'\n').count(),
        1,
        "one JSON value without logs"
    );
    let text = std::str::from_utf8(&output.stdout).unwrap();
    for forbidden in [
        "private_key",
        "node_credential",
        "token",
        "instance",
        "secret",
        "\u{001b}",
        "<SHA>",
        "\u{0085}",
        "\u{202e}",
    ] {
        assert!(!text.contains(forbidden), "must not emit {forbidden:?}");
    }
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test]
async fn live_cli_json_human_lookup_and_state_override() {
    let dir = directory();
    let state = dir.path().join("a");
    let task = fixture(&state).await;
    let output = cli(&state, &["status", "--live", "--json"]).await;
    assert!(output.status.success(), "{:?}", output);
    let value = json(&output);
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["outcome"]["result"]["generation"], 7);
    assert!(value["outcome"]["result"]["peers"].is_null());
    let output = cli(&state, &["status", "--live", "--json", "--peers"]).await;
    assert!(
        json(&output)["outcome"]["result"]["peers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let output = cli(&state, &["status", "--live"]).await;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("DNS: not observed"));
    assert!(!text.contains('\u{001b}'));
    assert!(!text.contains("<SHA>"));
    for query in ["SHA", "100.64.0.1", "fd00::1"] {
        let output = cli(&state, &["whois", query, "--json"]).await;
        assert!(output.status.success());
        assert_eq!(
            json(&output)["outcome"]["result"]["identity"]["node_id"],
            "a".repeat(64)
        );
    }
    let output = cli(&state, &["ping", "SHA", "--json", "--count", "2"]).await;
    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["requested_count"], 2);
    assert_eq!(value["samples"].as_array().unwrap().len(), 2);
    assert_eq!(value["samples"][1]["sequence"], 2);
    assert_eq!(value["samples"][0]["outcome"]["result"]["rtt_ns"], 123_456);
    assert_eq!(value["samples"][0]["outcome"]["result"]["generation"], 7);
    assert_eq!(value["stopped_on_failure"], false);
    assert_eq!(value["result"]["status"], "ok");
    let output = cli(&state, &["ping", "unknown", "--json", "--count", "3"]).await;
    assert_eq!(output.status.code(), Some(1));
    let value = json(&output);
    assert_eq!(
        value["samples"][0]["outcome"]["result"],
        "unconfigured_peer"
    );
    assert_eq!(value["result"]["status"], "error");
    assert_eq!(value["result"]["result"], "unconfigured_peer");
    let output = cli(&state, &["ping", "SHA", "--count", "1"]).await;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("authenticated neighbor probe / 认证邻接探测 (非ICMP，非mesh trace)"));
    assert!(text.contains("seq=1 peer="));
    assert!(text.contains("accepted (peer initiated)"));
    assert!(!text.contains('\u{001b}'));
    let missing = dir.path().join("missing");
    let output = cli(
        &state,
        &[
            "status",
            "--live",
            "--json",
            "--state",
            missing.to_str().unwrap(),
        ],
    )
    .await;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json(&output)["outcome"]["result"], "unavailable");
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn cached_status_compatibility_missing_daemon_and_exit_codes() {
    let dir = directory();
    let state = dir.path().join("a");
    let agent = meshway_agent::Agent::prepare_enrollment(
        &state,
        "http://127.0.0.1:1",
        &"b".repeat(64),
        "local",
    )
    .unwrap();
    drop(agent);
    let expected = serde_json::to_value(meshway_agent::status(&state).unwrap()).unwrap();
    for args in [vec!["status"], vec!["status", "--json"]] {
        let output = cli(&state, &args).await;
        assert!(output.status.success());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            expected
        );
    }
    for args in [
        vec!["status", "--live", "--json"],
        vec!["whois", "local", "--json"],
        vec!["ping", "local", "--json", "--count", "3"],
    ] {
        let output = cli(&state, &args).await;
        assert_eq!(output.status.code(), Some(1));
        let value = json(&output);
        if args[0] == "ping" {
            assert_eq!(value["samples"].as_array().unwrap().len(), 1);
            assert_eq!(value["samples"][0]["outcome"]["result"], "unavailable");
            assert_eq!(value["stopped_on_failure"], true);
        } else {
            assert_eq!(value["outcome"]["result"], "unavailable");
        }
    }
    for args in [
        vec!["status", "--peers"],
        vec!["status", "--live", "--peers", "--peer-limit", "65"],
        vec!["ping", "local", "--count", "0"],
        vec!["whois"],
    ] {
        let output = cli(&state, &args).await;
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty());
    }
}
