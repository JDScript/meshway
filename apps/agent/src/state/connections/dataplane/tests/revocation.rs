//! Real generated-client requests against an intentionally incomplete raw HTTP
//! error response. The daemon must consume the status, not wait for its body.

use super::*;
use meshway_protocol::{AgentNodeResponse, NodeSummary};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Notify,
    time,
};

struct RawServer {
    armed: Arc<AtomicBool>,
    headers_sent: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for RawServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl RawServer {
    fn start(
        listener: TcpListener,
        snapshot: AgentDataplaneSnapshot,
        credential: String,
        target: &'static str,
    ) -> Self {
        let armed = Arc::new(AtomicBool::new(false));
        let headers_sent = Arc::new(Notify::new());
        let enabled = armed.clone();
        let sent = headers_sent.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (path, body) = request(&mut stream, &credential).await;
                if enabled.load(Ordering::SeqCst) && path == target {
                    stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: 1048576\r\nX-Meshway-Test: revoked\r\n\r\n{").await.unwrap();
                    sent.notify_one();
                    // Keep the incomplete response open beyond both the test
                    // deadline and client timeout. No EOF can explain completion.
                    std::future::pending::<()>().await;
                    drop(stream);
                    unreachable!();
                }
                let bytes = success(&path, &body, &snapshot);
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).as_bytes()).await.unwrap();
                stream.write_all(&bytes).await.unwrap();
                stream.shutdown().await.unwrap();
            }
        });
        Self {
            armed,
            headers_sent,
            task,
        }
    }
}

async fn request(stream: &mut TcpStream, credential: &str) -> (String, Vec<u8>) {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        assert!(bytes.len() < 8192, "bounded raw fixture request headers");
        bytes.push(stream.read_u8().await.unwrap());
    }
    let headers = std::str::from_utf8(&bytes).unwrap();
    let path = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    let fields: Vec<_> = headers
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .collect();
    assert!(
        fields
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("authorization")
                && value.trim() == format!("Bearer {credential}")),
        "generated request uses the enrolled node bearer"
    );
    assert!(
        !fields
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("transfer-encoding"))
    );
    let length = fields
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map_or(0, |(_, value)| value.trim().parse::<usize>().unwrap());
    assert!(length <= 8192);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await.unwrap();
    (path, body)
}

pub(super) fn success(path: &str, body: &[u8], snapshot: &AgentDataplaneSnapshot) -> Vec<u8> {
    let payload = &snapshot.payload;
    match path {
        "/api/v1/agents/heartbeat" => {
            let _: meshway_protocol::HeartbeatRequest = serde_json::from_slice(body).unwrap();
            let node = &payload.configuration.node;
            let now = chrono::Utc::now();
            serde_json::to_vec(&AgentNodeResponse {
                node: NodeSummary {
                    id: node.id.clone(),
                    name: node.name.clone(),
                    mesh_ipv4: node.mesh_ipv4.clone(),
                    mesh_ipv6: node.mesh_ipv6.clone(),
                    country: node.country.clone(),
                    configuration_revision: payload.revision,
                    acknowledged_revision: payload.revision,
                    agent_version: "test".into(),
                    last_seen: now,
                    registered_at: now,
                    revoked_at: None,
                },
            })
            .unwrap()
        }
        "/api/v1/agent/connection-identity" => {
            let request: meshway_protocol::PublishConnectionIdentityRequest =
                serde_json::from_slice(body).unwrap();
            assert_eq!(
                Some(&request.public_key),
                payload.connections.public_key.as_ref()
            );
            serde_json::to_vec(&PublishConnectionIdentityResponse {
                revision: payload.revision,
                node_id: payload.node_id.clone(),
                public_key: request.public_key,
                capabilities: request.capabilities,
            })
            .unwrap()
        }
        "/api/v1/agent/dataplane" => serde_json::to_vec(snapshot).unwrap(),
        "/api/v1/agents/configuration/ack" => {
            let request: AcknowledgeConfigurationRequest = serde_json::from_slice(body).unwrap();
            assert_eq!(request.revision, payload.revision);
            assert_eq!(
                request.digest,
                configuration_digest(&payload.configuration).unwrap()
            );
            serde_json::to_vec(&AcknowledgeConfigurationResponse {
                revision: request.revision,
                digest: request.digest,
            })
            .unwrap()
        }
        "/api/v1/agent/connections/ack" => {
            let request: AcknowledgeAgentConnectionsRequest = serde_json::from_slice(body).unwrap();
            assert_eq!(request.revision, payload.revision);
            assert_eq!(
                request.digest,
                connection_digest(&payload.connections).unwrap()
            );
            serde_json::to_vec(&AcknowledgeAgentConnectionsResponse {
                revision: request.revision,
                digest: request.digest,
                status: ConnectionReceiptStatus::Saved,
            })
            .unwrap()
        }
        _ => panic!("unexpected generated operation: {path}"),
    }
}

async fn partial_403_withdraws_active_packets(target: &'static str) {
    const LEASE: Duration = Duration::from_secs(30);
    let dir = directory();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (mut agent, mut payload) = enrolled(dir.path(), &base);
    let remote_identity = meshway_dataplane::Identity::generate().unwrap();
    for peer in payload
        .connections
        .peers
        .iter_mut()
        .chain(&mut payload.identities)
    {
        peer.public_key = remote_identity.public_key_hex();
    }
    let server = RawServer::start(
        listener,
        snapshot(payload.clone()),
        agent.identity.node_credential.clone(),
        target,
    );
    let mut cache = State::load(&mut agent).unwrap();
    let (device, _from_kernel, mut delivered) = MockDevice::new();
    let mut manager = Manager::new_dataplane(
        None,
        LEASE,
        TunOptions {
            name: "mock0".into(),
            mtu: 1000,
        },
        base,
        false,
    );
    manager.mock_tun(Tun::mock(
        device.clone(),
        &PacketPlan::compile(&payload, 1000).unwrap(),
    ));
    let synced = cache.sync(&mut agent, &mut manager).await.unwrap();
    let fresh_at = synced.fresh_at;
    let key = agent
        .connections
        .as_ref()
        .unwrap()
        .keys
        .application_key(&agent.path, &agent.identity)
        .unwrap();
    manager
        .reconcile_dataplane(synced.snapshot, key, fresh_at)
        .await
        .unwrap();
    let remote =
        super::stale::blocked_peer(&payload, remote_identity, &device, &mut delivered).await;
    assert_eq!(manager.observation().state, ConnectionRuntimeState::Applied);
    server.armed.store(true, Ordering::SeqCst);
    let mut dataplane = Some(cache);
    let started = Instant::now();
    let result = time::timeout(Duration::from_secs(2), agent.managed_loop(&mut manager, Duration::from_secs(1), &mut dataplane))
        .await.expect("terminal HTTP status must not wait for the 10-second client timeout or 30-second packet lease");
    assert!(matches!(
        result,
        Err(AgentError::Http {
            status: 403,
            retryable: false
        })
    ));
    time::timeout(Duration::from_secs(1), server.headers_sent.notified())
        .await
        .unwrap();
    assert!(
        !server.task.is_finished(),
        "raw 403 body remains incomplete and open"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(fresh_at.elapsed() < LEASE / 2);
    assert_eq!(
        manager.observation().state,
        ConnectionRuntimeState::Withdrawn
    );
    device.allow_writes();
    let _ = remote
        .send_packet(
            payload.access_policy.grants[1].path.clone(),
            crate::managed_dataplane::tests::packet("100.64.0.2", "100.64.0.1", 1),
        )
        .await;
    assert!(
        time::timeout(Duration::from_millis(150), delivered.recv())
            .await
            .is_err(),
        "revocation cancels queued plaintext injection before returning the terminal error"
    );
    manager
        .withdraw(ConnectionRuntimeReason::ShuttingDown)
        .await;
    remote.shutdown().await;
}

#[tokio::test]
async fn heartbeat_partial_403_stops_active_packet_generation_without_reading_body() {
    partial_403_withdraws_active_packets("/api/v1/agents/heartbeat").await;
}

#[tokio::test]
async fn dataplane_partial_403_stops_active_packet_generation_without_reading_body() {
    partial_403_withdraws_active_packets("/api/v1/agent/dataplane").await;
}
