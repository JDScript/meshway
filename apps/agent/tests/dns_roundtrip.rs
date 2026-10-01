#![cfg(all(feature = "managed-connections", unix))]

use std::{net::SocketAddr, os::unix::fs::PermissionsExt, path::Path, time::Duration};

use hickory_proto::{
    op::{Message, MessageType, OpCode, Query, ResponseCode},
    rr::{
        Name, RData, RecordType,
        rdata::{A, AAAA, PTR},
    },
};
use meshway_agent::Agent;
use meshway_controller::{AppState, admin_token, router};
use meshway_protocol::{
    CreateEnrollmentTokenRequest, CreateEnrollmentTokenResponse, NetworkSnapshot,
    SaveNetworkRequest, canonical_node_dns_name,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    process::Command,
    time::timeout,
};

#[path = "support/network_scope.rs"]
mod network_scope;
use network_scope::admin_url;

async fn enroll(
    base: &str,
    admin: &str,
    path: &Path,
    name: &str,
) -> meshway_agent::generated::types::NodeSummary {
    let response = reqwest::Client::new()
        .post(admin_url(base, admin, "/enrollment-tokens").await)
        .bearer_auth(admin)
        .header("content-type", "application/json")
        .body(
            serde_json::to_vec(&CreateEnrollmentTokenRequest {
                expires_in_seconds: 3600,
            })
            .unwrap(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let token: CreateEnrollmentTokenResponse =
        serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    let mut agent = Agent::prepare_enrollment(path, base, &token.token, name).unwrap();
    agent.enroll(&token.token).await.unwrap().node
}

async fn free_port() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap()
}

async fn dns_cli(path: &Path, args: &[&str]) -> (i32, serde_json::Value) {
    // Respect the same bounded IPC admission budget used by every diagnostic.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let output = timeout(
        Duration::from_secs(15),
        Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
            .args(["dns"])
            .args(args)
            .arg("--json")
            .env("MESHWAY_STATE", path)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        text.lines().count(),
        1,
        "JSON stdout must have exactly one result: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for forbidden in [
        "node_credential",
        "private_key",
        "token",
        "secret",
        "instance_id",
    ] {
        assert!(
            !text.contains(forbidden),
            "public diagnostic leaked {forbidden}"
        );
    }
    (
        output.status.code().unwrap(),
        serde_json::from_str(&text).unwrap(),
    )
}

async fn await_dns(path: &Path, expected: &str) -> serde_json::Value {
    timeout(Duration::from_secs(10), async {
        loop {
            let (code, value) = dns_cli(path, &["status"]).await;
            if code == 0 && value["outcome"]["result"]["state"] == expected {
                return value;
            }
        }
    })
    .await
    .unwrap()
}

async fn save_dns(client: &reqwest::Client, base: &str, admin: &str, enabled: bool, zone: &str) {
    let response = client
        .get(admin_url(base, admin, "/network").await)
        .bearer_auth(admin)
        .send()
        .await
        .unwrap();
    let network: NetworkSnapshot =
        serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    let mut dns = network.dns;
    dns.enabled = enabled;
    dns.zone = zone.into();
    let response = client
        .post(admin_url(base, admin, "/network").await)
        .bearer_auth(admin)
        .header("content-type", "application/json")
        .body(
            serde_json::to_vec(&SaveNetworkRequest {
                expected_revision: network.revision,
                name: network.name,
                dns,
            })
            .unwrap(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

async fn ask(address: SocketAddr, owner: &str, kind: RecordType, tcp: bool) -> Message {
    let mut request = Message::new(3421, MessageType::Query, OpCode::Query);
    request.metadata.recursion_desired = true;
    request.add_query(Query::query(Name::from_ascii(owner).unwrap(), kind));
    let wire = request.to_vec().unwrap();
    let reply = if tcp {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_u16(wire.len() as u16).await.unwrap();
        stream.write_all(&wire).await.unwrap();
        let length = stream.read_u16().await.unwrap();
        let mut reply = vec![0; usize::from(length)];
        stream.read_exact(&mut reply).await.unwrap();
        reply
    } else {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        socket.send_to(&wire, address).await.unwrap();
        let mut reply = vec![0; 1232];
        let (length, _) = socket.recv_from(&mut reply).await.unwrap();
        reply.truncate(length);
        reply
    };
    let reply = Message::from_vec(&reply).unwrap();
    assert_eq!(reply.metadata.id, request.metadata.id);
    assert!(!reply.metadata.recursion_available);
    reply
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_controller_cli_udp_tcp_assigned_addresses_same_identity_lock_and_revocation() {
    roundtrip(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dns_coexists_with_managed_adjacency_daemon_and_original_state_lock() {
    roundtrip(true).await;
}

async fn roundtrip(apply_connections: bool) {
    let directory = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let controller_dir = directory.path().join("controller");
    let state = AppState::open(&controller_dir).unwrap();
    let admin = admin_token(&controller_dir).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });
    let path = directory.path().join("agent.json");
    let own = enroll(&base, &admin, &path, "demo-sha").await;
    let dupe = enroll(&base, &admin, &directory.path().join("peer1.json"), "dupe").await;
    enroll(&base, &admin, &directory.path().join("peer2.json"), "DUPE").await;
    let client = reqwest::Client::new();
    let network = client
        .get(admin_url(&base, &admin, "/network").await)
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    let network: NetworkSnapshot = serde_json::from_slice(&network.bytes().await.unwrap()).unwrap();
    let mut dns = network.dns;
    dns.enabled = true;
    let saved = client
        .post(admin_url(&base, &admin, "/network").await)
        .bearer_auth(&admin)
        .header("content-type", "application/json")
        .body(
            serde_json::to_vec(&SaveNetworkRequest {
                expected_revision: network.revision,
                name: network.name,
                dns,
            })
            .unwrap(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(saved.status(), 200);
    let identity = std::fs::read(&path).unwrap();
    let address = match std::env::var("MESHWAY_DNS_TEST_LISTEN")
        .ok()
        .filter(|_| !apply_connections)
    {
        Some(address) => address.parse().unwrap(),
        None => free_port().await,
    };
    let mut command = Command::new(env!("CARGO_BIN_EXE_meshway-agent"));
    command
        .args([
            "run",
            "--interval-seconds",
            "1",
            "--connection-lease-seconds",
            "10",
            "--dns-listen",
            &address.to_string(),
        ])
        .arg("--state")
        .arg(&path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if apply_connections {
        command.arg("--apply-connections");
    }
    let mut child = command.spawn().unwrap();
    let active = await_dns(&path, "active").await;
    assert_eq!(active["outcome"]["result"]["node_id"], own.id);
    assert!(active["outcome"]["result"]["network_id"].is_string());
    assert_eq!(active["outcome"]["result"]["listener"], address.to_string());
    if apply_connections {
        let output = Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
            .args(["status", "--live", "--json"])
            .env("MESHWAY_STATE", &path)
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
    }
    for tcp in [false, true] {
        for (input, kind, expected) in [
            ("demo-sha.meshway.test", "A", own.mesh_ipv4.as_str()),
            ("demo-sha.meshway.test", "AAAA", own.mesh_ipv6.as_str()),
            (own.mesh_ipv4.as_str(), "PTR", ""),
        ] {
            let mut args = vec!["query", input, "--type", kind];
            if tcp {
                args.push("--tcp");
            }
            let (code, value) = dns_cli(&path, &args).await;
            assert_eq!(code, 0, "{value}");
            assert_eq!(value["outcome"]["result"]["state"], "positive");
            if !expected.is_empty() {
                assert_eq!(value["outcome"]["result"]["answers"][0]["data"], expected);
            }
        }
    }
    for (input, expected, code) in [
        ("missing.meshway.test", "nxdomain", 0),
        ("meshway.test", "nodata", 0),
        ("outside.test", "refused", 1),
    ] {
        let (actual, value) = dns_cli(&path, &["query", input]).await;
        assert_eq!(actual, code);
        if code == 0 {
            assert_eq!(value["outcome"]["result"]["state"], expected);
        } else {
            assert_eq!(value["outcome"]["status"], "error");
            assert_eq!(value["observation"]["state"], expected);
        }
    }
    let (code, _) = dns_cli(&path, &["query", "bad/name"]).await;
    assert_eq!(code, 2);
    let (code, value) = dns_cli(
        &directory.path().join("missing"),
        &["status", "--state", path.to_str().unwrap()],
    )
    .await;
    assert_eq!(code, 0);
    assert_eq!(value["outcome"]["result"]["node_id"], own.id);
    if std::env::var_os("MESHWAY_DNS_TEST_DIG").is_some() {
        let output = Command::new("dig")
            .args([
                "@127.0.0.1",
                "-p",
                &address.port().to_string(),
                "demo-sha.meshway.test",
                "A",
                "+short",
            ])
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            own.mesh_ipv4
        );
        println!(
            "dig @127.0.0.1 -p {} demo-sha.meshway.test A +short => {} (controller assigned)",
            address.port(),
            own.mesh_ipv4
        );
        let output = Command::new("dig")
            .args([
                "@127.0.0.1",
                "-p",
                &address.port().to_string(),
                "demo-sha.meshway.test",
                "AAAA",
                "+tcp",
                "+short",
            ])
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            own.mesh_ipv6
        );
        println!(
            "dig @127.0.0.1 -p {} demo-sha.meshway.test AAAA +tcp +short => {} (address metadata)",
            address.port(),
            own.mesh_ipv6
        );
    }
    assert!(
        Agent::load(&path).is_err(),
        "DNS shares the root Agent's state lock"
    );
    for tcp in [false, true] {
        let a = timeout(
            Duration::from_secs(2),
            ask(address, "DEMO-SHA.MESHWAY.TEST.", RecordType::A, tcp),
        )
        .await
        .unwrap();
        assert!(a.metadata.authoritative);
        assert_eq!(
            a.answers[0].data,
            RData::A(A(own.mesh_ipv4.parse().unwrap()))
        );
        let aaaa = ask(address, "demo-sha.meshway.test.", RecordType::AAAA, tcp).await;
        assert_eq!(
            aaaa.answers[0].data,
            RData::AAAA(AAAA(own.mesh_ipv6.parse().unwrap()))
        );
        let reverse = Name::from(own.mesh_ipv4.parse::<std::net::Ipv4Addr>().unwrap()).to_string();
        let ptr = ask(address, &reverse, RecordType::PTR, tcp).await;
        let canonical = canonical_node_dns_name(&own.id, "meshway.test").unwrap();
        assert_eq!(
            ptr.answers[0].data,
            RData::PTR(PTR(Name::from_ascii(format!("{canonical}.")).unwrap()))
        );
        assert_eq!(
            ask(address, "dupe.meshway.test.", RecordType::A, tcp)
                .await
                .metadata
                .response_code,
            ResponseCode::NXDomain
        );
        let dupe_canonical = canonical_node_dns_name(&dupe.id, "meshway.test").unwrap();
        assert_eq!(
            ask(address, &dupe_canonical, RecordType::A, tcp)
                .await
                .answers[0]
                .data,
            RData::A(A(dupe.mesh_ipv4.parse().unwrap()))
        );
    }
    save_dns(&client, &base, &admin, false, "meshway.test").await;
    let disabled = await_dns(&path, "disabled").await;
    assert!(disabled["outcome"]["result"]["active_revision"].is_null());
    assert_eq!(
        dns_cli(&path, &["query", "demo-sha.meshway.test"]).await.0,
        1
    );
    save_dns(&client, &base, &admin, true, "changed.meshway.test").await;
    let changed = await_dns(&path, "active").await;
    assert_eq!(
        changed["outcome"]["result"]["zone"],
        "changed.meshway.test."
    );
    assert_eq!(
        dns_cli(&path, &["query", "demo-sha.changed.meshway.test"])
            .await
            .0,
        0
    );
    let revoked = client
        .post(admin_url(&base, &admin, "/nodes/revoke").await)
        .bearer_auth(&admin)
        .header("content-type", "application/json")
        .body(
            serde_json::to_vec(&meshway_protocol::RevokeNodeRequest {
                node_id: own.id.clone(),
            })
            .unwrap(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), 200);
    assert!(
        !timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    assert_eq!(std::fs::read(&path).unwrap(), identity);
    let (code, value) = dns_cli(&path, &["status"]).await;
    assert_eq!(code, 1);
    assert_eq!(value["outcome"]["status"], "error");
    let rebound_tcp = TcpListener::bind(address).await.unwrap();
    let rebound_udp = UdpSocket::bind(address).await.unwrap();
    drop((rebound_tcp, rebound_udp));
    server.abort();
}
