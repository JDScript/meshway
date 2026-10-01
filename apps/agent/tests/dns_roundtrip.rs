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
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, UdpSocket},
    process::Command,
    time::timeout,
};

async fn enroll(
    base: &str,
    admin: &str,
    path: &Path,
    name: &str,
) -> meshway_agent::generated::types::NodeSummary {
    let response = reqwest::Client::new()
        .post(format!("{base}/api/v1/enrollment-tokens"))
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
        .get(format!("{base}/api/v1/network"))
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    let network: NetworkSnapshot = serde_json::from_slice(&network.bytes().await.unwrap()).unwrap();
    let mut dns = network.dns;
    dns.enabled = true;
    let saved = client
        .post(format!("{base}/api/v1/network"))
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
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    timeout(Duration::from_secs(10), async {
        let mut dns_revision = None;
        let mut runtime_revision = None;
        while let Some(line) = lines.next_line().await.unwrap() {
            let value: serde_json::Value = serde_json::from_str(&line).unwrap();
            if value["event"] == "managed_dns" && value["state"] == "active" {
                dns_revision = value["revision"].as_u64();
            }
            if apply_connections && value["runtime"].is_object() {
                runtime_revision = value["runtime"]["revision"].as_u64();
            }
            if dns_revision.is_some()
                && (!apply_connections
                    || runtime_revision.is_some_and(|revision| dns_revision >= Some(revision)))
            {
                return;
            }
        }
        let mut error = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut error)
            .await
            .unwrap();
        panic!("DNS exited without becoming active: {error}");
    })
    .await
    .unwrap();
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
    let revoked = client
        .post(format!("{base}/api/v1/nodes/revoke"))
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
    let rebound_tcp = TcpListener::bind(address).await.unwrap();
    let rebound_udp = UdpSocket::bind(address).await.unwrap();
    drop((rebound_tcp, rebound_udp));
    server.abort();
}
