use super::*;
use crate::state::{connection_key::KeyStore, write_identity};
use axum::{Router, extract::State as HttpState, response::IntoResponse, routing::get};
use hickory_proto::{
    op::{Message, OpCode, Query, ResponseCode},
    rr::RecordType,
};
use meshway_protocol::{
    MeshDnsConfiguration, NodeDnsConfiguration, NodeDnsRecord, canonical_node_dns_name,
};
use std::sync::{Arc, RwLock};
use tokio::{net::TcpListener, sync::Notify};

mod inventory;
mod pins;

#[test]
fn service_errors_preserve_agent_validation_and_io_causes() {
    let invalid = meshway_agent_dns::validate_listen("0.0.0.0:5353".parse().unwrap()).unwrap_err();
    assert!(matches!(
        AgentError::from(invalid),
        AgentError::Invalid("DNS listen requires a loopback IP and port >= 1024")
    ));
    let error = std::io::Error::new(std::io::ErrorKind::AddrInUse, "occupied DNS listener");
    match AgentError::from(meshway_agent_dns::Error::Io(error)) {
        AgentError::File(error) => {
            assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
            assert_eq!(error.to_string(), "occupied DNS listener");
        }
        error => panic!("DNS IO cause was lost: {error}"),
    }
}

fn snapshot(id: &str, revision: u32) -> AgentDnsSnapshot {
    let zone = "meshway.test";
    let payload = NodeDnsConfiguration {
        schema_version: 1,
        revision,
        node_id: id.into(),
        network_id: "0".repeat(32),
        dns: MeshDnsConfiguration {
            enabled: true,
            zone: zone.into(),
            ttl_seconds: 30,
        },
        records: vec![NodeDnsRecord {
            node_id: id.into(),
            canonical_name: canonical_node_dns_name(id, zone).unwrap(),
            aliases: vec![format!("demo-sha.{zone}")],
            mesh_ipv4: "100.64.0.2".into(),
            mesh_ipv6: "fd00::2".into(),
        }],
        diagnostics: vec![],
    };
    AgentDnsSnapshot {
        digest: dns_digest(&payload).unwrap(),
        payload,
    }
}

fn rehash(snapshot: &mut AgentDnsSnapshot) {
    snapshot.digest = dns_digest(&snapshot.payload).unwrap();
}

async fn query(guard: &Guard, owner: &str) -> Message {
    let reservation = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listen = reservation.local_addr().unwrap();
    drop(reservation);
    let server = Server::bind(listen).await.unwrap();
    let service = tokio::spawn(server.serve(guard.clone()));
    let mut request = Message::new(5432, hickory_proto::op::MessageType::Query, OpCode::Query);
    request.add_query(Query::query(
        Name::from_ascii(owner).unwrap(),
        RecordType::A,
    ));
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.connect(listen).await.unwrap();
    socket.send(&request.to_vec().unwrap()).await.unwrap();
    let mut bytes = [0; 512];
    let length = tokio::time::timeout(Duration::from_secs(2), socket.recv(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    service.abort();
    assert!(service.await.unwrap_err().is_cancelled());
    Message::from_vec(&bytes[..length]).unwrap()
}

#[test]
fn strict_snapshot_bindings_names_addresses_digest_and_replay() {
    let id = "a".repeat(64);
    let good = snapshot(&id, 2);
    validate(&good, &id, None).unwrap();
    assert!(validate(&good, &"b".repeat(64), None).is_err());
    let mut invalid = good.clone();
    invalid.digest = "0".repeat(64);
    assert!(validate(&invalid, &id, None).is_err());
    let mutations: Vec<fn(&mut AgentDnsSnapshot)> = vec![
        |s| s.payload.schema_version = 2,
        |s| s.payload.revision = 0,
        |s| s.payload.network_id = "A".repeat(32),
        |s| s.payload.dns.ttl_seconds = 0,
        |s| s.payload.dns.ttl_seconds = 301,
        |s| s.payload.dns.zone = "test".into(),
        |s| s.payload.dns.zone = "meshway.test.".into(),
        |s| s.payload.records[0].canonical_name = "root.meshway.test".into(),
        |s| s.payload.records[0].aliases = vec!["root.example.org".into()],
        |s| s.payload.records[0].aliases = vec!["id.meshway.test".into()],
        |s| s.payload.records[0].aliases = vec!["RootNode.meshway.test".into()],
        |s| {
            s.payload.records[0]
                .aliases
                .push("extra.meshway.test".into())
        },
        |s| s.payload.records[0].mesh_ipv6 = "FD00:0::2".into(),
        |s| s.payload.records[0].mesh_ipv4 = "127.0.0.1".into(),
        |s| s.payload.records.clear(),
        |s| s.payload.dns.enabled = false,
    ];
    for mutate in mutations {
        let mut invalid = good.clone();
        mutate(&mut invalid);
        rehash(&mut invalid);
        assert!(validate(&invalid, &id, None).is_err());
    }
    let mut peer = good.payload.records[0].clone();
    peer.node_id = "b".repeat(64);
    peer.canonical_name = canonical_node_dns_name(&peer.node_id, "meshway.test").unwrap();
    peer.aliases.clear();
    let mut duplicate_ips = good.clone();
    duplicate_ips.payload.records.push(peer.clone());
    rehash(&mut duplicate_ips);
    assert!(validate(&duplicate_ips, &id, None).is_err());
    peer.mesh_ipv4 = "100.64.0.3".into();
    peer.mesh_ipv6 = "fd00::3".into();
    peer.aliases = good.payload.records[0].aliases.clone();
    let mut duplicate_name = good.clone();
    duplicate_name.payload.records.push(peer);
    rehash(&mut duplicate_name);
    assert!(validate(&duplicate_name, &id, None).is_err());
    let mut changed = good.clone();
    changed.payload.dns.ttl_seconds = 20;
    rehash(&mut changed);
    assert!(validate(&changed, &id, Some(&good)).is_err());
    changed.payload.revision = 3;
    rehash(&mut changed);
    validate(&changed, &id, Some(&good)).unwrap();
    changed.payload.network_id = "1".repeat(32);
    rehash(&mut changed);
    assert!(validate(&changed, &id, Some(&good)).is_err());
    assert!(validate(&snapshot(&id, 1), &id, Some(&good)).is_err());
    let mut cache_bytes = serde_json::to_vec(&good).unwrap();
    // Protocol DTOs are closed; unknown fields cannot be silently trusted.
    cache_bytes.pop();
    cache_bytes.extend_from_slice(b",\"extra\":true}");
    assert!(serde_json::from_slice::<AgentDnsSnapshot>(&cache_bytes).is_err());
}

#[derive(Clone)]
enum Source {
    Snapshot(AgentDnsSnapshot),
    Status(u16),
    Stalled,
}
type Shared = (Arc<RwLock<Source>>, Arc<Notify>, String);

async fn handler(
    HttpState((source, requested, credential)): HttpState<Shared>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some(format!("Bearer {credential}").as_str())
    {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }
    let source = source.read().unwrap().clone();
    requested.notify_one();
    match source {
        Source::Snapshot(snapshot) => axum::Json(snapshot).into_response(),
        Source::Status(status) => axum::http::StatusCode::from_u16(status)
            .unwrap()
            .into_response(),
        Source::Stalled => std::future::pending().await,
    }
}

async fn fixture() -> (
    tempfile::TempDir,
    Agent,
    Shared,
    tokio::task::JoinHandle<()>,
) {
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let directory = builder.tempdir().unwrap();
    local::private_dir(directory.path()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut agent = Agent::prepare_enrollment(
        directory.path().join("agent.json"),
        &format!("http://{address}/"),
        &local::credential().unwrap(),
        "demo-sha",
    )
    .unwrap();
    agent.identity.node_id = Some("a".repeat(64));
    write_identity(&agent.path, &agent.identity, true).unwrap();
    let shared = (
        Arc::new(RwLock::new(Source::Snapshot(snapshot(
            agent.identity.node_id.as_deref().unwrap(),
            1,
        )))),
        Arc::new(Notify::new()),
        agent.identity.node_credential.clone(),
    );
    let router = Router::new()
        .route("/api/v1/agent/dns", get(handler))
        .with_state(shared.clone());
    let task = tokio::spawn(async {
        axum::serve(listener, router).await.unwrap();
    });
    (directory, agent, shared, task)
}

fn sync(agent: &Agent, cache: Option<Cache>) -> Sync {
    Sync {
        client: agent.controller_client(),
        state: agent.path.clone(),
        identity: Identity::load(&agent.path).unwrap(),
        cache,
        guard: Guard::default(),
        listen: "127.0.0.1:5353".parse().unwrap(),
        interval: Duration::from_secs(1),
        lease: Duration::from_secs(10),
    }
}

#[tokio::test]
async fn fresh_get_private_cache_identity_stability_disabled_changed_zone_and_bad_source() {
    let (_directory, agent, shared, task) = fixture().await;
    let mut keys = KeyStore::load(&agent.path, &agent.identity).unwrap();
    keys.prepare(&agent.path, &agent.identity).unwrap();
    let identity_before = std::fs::read(&agent.path).unwrap();
    let key_before = std::fs::read(crate::state::connection_key::path(&agent.path)).unwrap();
    let mut runtime = sync(&agent, None);
    runtime.tick().await.unwrap();
    assert_eq!(
        query(&runtime.guard, "demo-sha.meshway.test.")
            .await
            .answers
            .len(),
        1
    );
    let cache = load(&agent.path, &agent.identity).unwrap().unwrap();
    let mut restarted = sync(&agent, Some(cache));
    assert_eq!(
        query(&restarted.guard, "demo-sha.meshway.test.")
            .await
            .metadata
            .response_code,
        ResponseCode::ServFail
    );
    restarted.tick().await.unwrap();
    let mut changed = snapshot(agent.identity.node_id.as_deref().unwrap(), 2);
    changed.payload.dns.zone = "changed.meshway.test".into();
    changed.payload.records[0].canonical_name =
        canonical_node_dns_name(&changed.payload.node_id, &changed.payload.dns.zone).unwrap();
    changed.payload.records[0].aliases = vec!["demo-sha.changed.meshway.test".into()];
    rehash(&mut changed);
    *shared.0.write().unwrap() = Source::Snapshot(changed.clone());
    restarted.tick().await.unwrap();
    assert!(
        query(&restarted.guard, "demo-sha.meshway.test.")
            .await
            .answers
            .is_empty()
    );
    assert_eq!(
        query(&restarted.guard, "demo-sha.changed.meshway.test.")
            .await
            .answers
            .len(),
        1
    );
    changed.payload.revision = 3;
    changed.payload.dns.enabled = false;
    changed.payload.records.clear();
    rehash(&mut changed);
    *shared.0.write().unwrap() = Source::Snapshot(changed);
    restarted.tick().await.unwrap();
    assert_eq!(
        query(&restarted.guard, "demo-sha.changed.meshway.test.")
            .await
            .metadata
            .response_code,
        ResponseCode::Refused
    );
    *shared.0.write().unwrap() =
        Source::Snapshot(snapshot(agent.identity.node_id.as_deref().unwrap(), 4));
    restarted.tick().await.unwrap();
    let mut bad = snapshot(agent.identity.node_id.as_deref().unwrap(), 5);
    bad.digest = "0".repeat(64);
    *shared.0.write().unwrap() = Source::Snapshot(bad);
    assert!(restarted.tick().await.is_err());
    assert!(
        query(&restarted.guard, "demo-sha.meshway.test.")
            .await
            .answers
            .is_empty()
    );
    assert_eq!(std::fs::read(&agent.path).unwrap(), identity_before);
    assert_eq!(
        std::fs::read(crate::state::connection_key::path(&agent.path)).unwrap(),
        key_before
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path(&agent.path))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    task.abort();
}

#[tokio::test]
async fn known_revision_conflict_revocation_and_lease_with_stalled_http() {
    let (_directory, agent, shared, task) = fixture().await;
    let mut runtime = sync(&agent, None);
    runtime.tick().await.unwrap();
    let guard = runtime.guard.clone();
    guard.observe_revision(2);
    assert!(
        query(&guard, "demo-sha.meshway.test.")
            .await
            .answers
            .is_empty()
    );
    assert!(runtime.tick().await.unwrap_err().retryable());
    *shared.0.write().unwrap() =
        Source::Snapshot(snapshot(agent.identity.node_id.as_deref().unwrap(), 2));
    runtime.tick().await.unwrap();
    guard.conflict(2);
    assert!(
        query(&guard, "demo-sha.meshway.test.")
            .await
            .answers
            .is_empty()
    );
    *shared.0.write().unwrap() =
        Source::Snapshot(snapshot(agent.identity.node_id.as_deref().unwrap(), 3));
    runtime.tick().await.unwrap();
    *shared.0.write().unwrap() = Source::Status(403);
    assert!(matches!(
        runtime.tick().await,
        Err(AgentError::Http { status: 403, .. })
    ));
    assert!(
        query(&guard, "demo-sha.meshway.test.")
            .await
            .answers
            .is_empty()
    );
    *shared.0.write().unwrap() =
        Source::Snapshot(snapshot(agent.identity.node_id.as_deref().unwrap(), 4));
    runtime.tick().await.unwrap();
    // Advance the guard's clock deterministically while the generated GET is
    // stalled. HTTP, heartbeats and reports have no capability to extend it.
    *shared.0.write().unwrap() = Source::Stalled;
    let request = runtime.tick();
    tokio::pin!(request);
    let _ = shared.1.notified().await; // Consume any notification from prior GET.
    tokio::select! {
        _ = &mut request => panic!("GET should be stalled"),
        _ = shared.1.notified() => {},
    }
    assert!(guard.expire(Instant::now() + Duration::from_secs(11)));
    assert!(
        query(&guard, "demo-sha.meshway.test.")
            .await
            .answers
            .is_empty()
    );
    task.abort();
}

#[tokio::test]
async fn cache_bindings_and_failed_durability_cannot_publish() {
    let (_directory, mut agent, _shared, task) = fixture().await;
    let mut runtime = sync(&agent, None);
    let fault = local::test_faults::DirectorySyncFailure::after_publication(&path(&agent.path));
    assert!(runtime.tick().await.is_err());
    assert!(
        query(&runtime.guard, "demo-sha.meshway.test.")
            .await
            .answers
            .is_empty()
    );
    drop(fault);
    local::ensure_durable(&path(&agent.path)).unwrap();
    runtime.tick().await.unwrap();
    agent.identity.node_credential = local::credential().unwrap();
    assert!(load(&agent.path, &agent.identity).is_err());
    task.abort();
}

#[tokio::test]
async fn cached_restart_socket_stays_unavailable_until_fresh_get() {
    let (_directory, mut agent, shared, task) = fixture().await;
    let mut initial = sync(&agent, None);
    initial.tick().await.unwrap();
    let cached = std::fs::read(path(&agent.path)).unwrap();
    *shared.0.write().unwrap() = Source::Stalled;
    let reservation = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listen = reservation.local_addr().unwrap();
    drop(reservation);
    let mut service = agent
        .dns_service(listen, Duration::from_secs(1), Duration::from_secs(10))
        .await
        .unwrap();
    #[cfg(unix)]
    service.enable_diagnostics(&agent).await.unwrap();
    let runtime = tokio::spawn(service.run());
    #[cfg(unix)]
    {
        let observed = crate::diagnostics::dns::status(&agent.path).await.unwrap();
        assert_eq!(observed.state, crate::diagnostics::dns::State::Unavailable);
        assert!(observed.listener_bound);
        assert!(
            observed.zone.is_none(),
            "cached zone must not masquerade as fresh publication"
        );
        let reply =
            crate::diagnostics::dns::query(&agent.path, "demo-sha.meshway.test", None, false)
                .await
                .unwrap();
        assert_eq!(reply.state, crate::diagnostics::dns::QueryState::Servfail);
    }
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut request = Message::new(4321, hickory_proto::op::MessageType::Query, OpCode::Query);
    request.add_query(Query::query(
        Name::from_ascii("demo-sha.meshway.test.").unwrap(),
        RecordType::A,
    ));
    socket
        .send_to(&request.to_vec().unwrap(), listen)
        .await
        .unwrap();
    let mut buffer = [0; 1232];
    let (length, _) = tokio::time::timeout(Duration::from_secs(2), socket.recv_from(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    let reply = Message::from_vec(&buffer[..length]).unwrap();
    assert!(reply.answers.is_empty());
    assert_eq!(reply.metadata.response_code, ResponseCode::ServFail);
    assert_eq!(std::fs::read(path(&agent.path)).unwrap(), cached);
    runtime.abort();
    assert!(runtime.await.unwrap_err().is_cancelled());
    #[cfg(unix)]
    {
        assert!(!crate::diagnostics::socket_path(&agent.path).exists());
        assert!(!agent.dns_guard.as_ref().unwrap().status().listener_bound);
    }
    task.abort();
}
