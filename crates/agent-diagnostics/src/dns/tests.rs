use super::*;
use crate::Server as Control;
use meshway_agent_dns::{Guard, Server, Zone};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};

struct DnsProvider(Guard);
impl crate::Provider for DnsProvider {
    fn live_status(&self, _: bool, _: u32, _: u32) -> Result<crate::LiveStatus, ProbeError> {
        Err(ProbeError::Unavailable)
    }
    fn whois(&self, _: &str) -> Result<crate::WhoisObservation, ProbeError> {
        Err(ProbeError::Unavailable)
    }
    fn probe_neighbor_observed(
        &self,
        _: crate::ProbeRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<crate::ProbeObservation, ProbeError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async { Err(ProbeError::Unavailable) })
    }
    fn dns_status(&self) -> Status {
        let snapshot = self.0.status();
        let mut result = Status::unavailable();
        result.state = match snapshot.state {
            meshway_agent_dns::StatusState::Active => State::Active,
            meshway_agent_dns::StatusState::Disabled => State::Disabled,
            meshway_agent_dns::StatusState::Expired => State::Expired,
            meshway_agent_dns::StatusState::Unavailable => State::Unavailable,
        };
        result.generation = snapshot.generation;
        result.revision_floor = snapshot.revision_floor;
        result.active_revision = snapshot.active_revision;
        result.zone = snapshot.zone;
        result.ttl = snapshot.ttl;
        result.lease_remaining_ms = snapshot.lease_remaining_ms;
        result.listener = snapshot.listener;
        result.listener_bound = snapshot.listener_bound;
        result.network_id = snapshot.network_id;
        result.node_id = snapshot.node_id;
        result
    }
}

fn zone() -> Zone {
    let owner = Name::from_ascii("node.meshway.test.").unwrap();
    Zone {
        name: Name::from_ascii("meshway.test.").unwrap(),
        ttl: 30,
        forward: BTreeMap::from([(
            owner.clone(),
            ("100.64.0.2".parse().unwrap(), "fd00::2".parse().unwrap()),
        )]),
        reverse: BTreeMap::from([(Name::from("100.64.0.2".parse::<IpAddr>().unwrap()), owner)]),
    }
}

async fn listener() -> (SocketAddr, Server) {
    // DNS requires explicit unprivileged loopback ports; reserve a fresh port.
    let temporary = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = temporary.local_addr().unwrap();
    drop(temporary);
    (address, Server::bind(address).await.unwrap())
}

async fn paced_status(state: &Path) -> Result<Status, Error> {
    tokio::time::sleep(Duration::from_millis(150)).await;
    status(state).await
}

async fn paced_query(
    state: &Path,
    input: &str,
    kind: Option<QueryType>,
    tcp: bool,
) -> Result<Observation, Error> {
    tokio::time::sleep(Duration::from_millis(150)).await;
    query(state, input, kind, tcp).await
}

#[tokio::test]
async fn dns_only_private_ipc_and_real_udp_tcp_queries_follow_guard_transitions() {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let state = dir.path().join("identity");
    let lock = meshway_protocol::local::lock(&dir.path().join("lock")).unwrap();
    let control = Control::bind(&state, &lock).await.unwrap();
    let (address, server) = listener().await;
    let guard = Guard::default();
    guard.listener_bound(address, Some("real-node".into()));
    let task_guard = guard.clone();
    let dns_task = tokio::spawn(async move { server.serve(task_guard).await });
    let task_guard = guard.clone();
    let control_task = tokio::spawn(async move {
        control
            .serve(std::sync::Arc::new(DnsProvider(task_guard)))
            .await
    });
    let waiting = paced_status(&state).await.unwrap();
    assert_eq!(waiting.state, State::Unavailable);
    assert!(waiting.listener_bound);
    guard.publish_snapshot(
        zone(),
        Instant::now() + Duration::from_secs(20),
        1,
        false,
        Some("real-network".into()),
        Some("real-node".into()),
    );
    assert_eq!(paced_status(&state).await.unwrap().state, State::Disabled);
    assert_eq!(
        paced_query(&state, "node.meshway.test", None, false)
            .await
            .unwrap()
            .state,
        QueryState::Refused
    );
    guard.publish_snapshot(
        zone(),
        Instant::now() + Duration::from_secs(20),
        2,
        true,
        Some("real-network".into()),
        Some("real-node".into()),
    );
    let active = paced_status(&state).await.unwrap();
    assert_eq!(active.state, State::Active);
    assert_eq!(active.network_id.as_deref(), Some("real-network"));
    assert_eq!(active.active_revision, Some(2));
    assert!(active.generation > waiting.generation);
    let value = serde_json::to_value(&active).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "state": "active",
            "generation": active.generation,
            "revision_floor": 2,
            "active_revision": 2,
            "zone": "meshway.test.",
            "ttl": 30,
            "lease_remaining_ms": active.lease_remaining_ms,
            "listener": address.to_string(),
            "listener_bound": true,
            "network_id": "real-network",
            "node_id": "real-node",
            "evidence": "current_dns_guard",
        })
    );
    for tcp in [false, true] {
        for (input, kind, expected) in [
            ("node.meshway.test", Some(QueryType::A), "100.64.0.2"),
            ("node.meshway.test", Some(QueryType::AAAA), "fd00::2"),
            ("100.64.0.2", None, "node.meshway.test."),
        ] {
            let observed = paced_query(&state, input, kind, tcp).await.unwrap();
            assert_eq!(observed.state, QueryState::Positive);
            assert_eq!(observed.answers[0].data, expected);
        }
        assert_eq!(
            paced_query(&state, "missing.meshway.test", None, tcp)
                .await
                .unwrap()
                .state,
            QueryState::Nxdomain
        );
        assert_eq!(
            paced_query(&state, "meshway.test", None, tcp)
                .await
                .unwrap()
                .state,
            QueryState::Nodata
        );
    }
    let mut changed = zone();
    changed.name = Name::from_ascii("changed.test.").unwrap();
    guard.publish_snapshot(
        changed,
        Instant::now() + Duration::from_secs(10),
        3,
        true,
        None,
        Some("real-node".into()),
    );
    assert_eq!(
        paced_status(&state).await.unwrap().zone.as_deref(),
        Some("changed.test.")
    );
    guard.expire(Instant::now() + Duration::from_secs(11));
    assert_eq!(paced_status(&state).await.unwrap().state, State::Expired);
    assert_eq!(
        paced_query(&state, "node.changed.test", None, false)
            .await
            .unwrap()
            .state,
        QueryState::Servfail
    );
    guard.withdraw();
    assert_eq!(
        paced_status(&state).await.unwrap().state,
        State::Unavailable
    );
    guard.shutdown();
    assert_eq!(paced_status(&state).await.unwrap().active_revision, None);
    assert_eq!(
        crate::live_status(&state, false, 0, 64).await.unwrap_err(),
        ProbeError::Unavailable
    );
    control_task.abort();
    dns_task.abort();
    let _ = control_task.await;
    let _ = dns_task.await;
    assert!(status(&state).await.is_err());
}

#[test]
fn rejects_wrong_transaction_question_type_trailing_and_malformed_response() {
    let (name, kind) = question("node.meshway.test", None).unwrap();
    let mut request = Message::new(123, MessageType::Query, OpCode::Query);
    request.add_query(Query::query(name, kind.record_type()));
    let mut reply = Message::response(123, OpCode::Query);
    reply.queries = request.queries.clone();
    assert!(validate_response(&request, &reply.to_vec().unwrap()).is_ok());
    let mut wrong = reply.clone();
    wrong.metadata.id = 124;
    assert!(validate_response(&request, &wrong.to_vec().unwrap()).is_err());
    let mut wrong = reply.clone();
    wrong.queries[0] = Query::query(Name::from_ascii("evil.test").unwrap(), RecordType::A);
    assert!(validate_response(&request, &wrong.to_vec().unwrap()).is_err());
    let mut wrong = reply.clone();
    wrong.queries[0] = Query::query(request.queries[0].name().clone(), RecordType::AAAA);
    assert!(validate_response(&request, &wrong.to_vec().unwrap()).is_err());
    let mut bytes = reply.to_vec().unwrap();
    bytes.push(0);
    assert!(validate_response(&request, &bytes).is_err());
    assert!(validate_response(&request, &[0; 5]).is_err());
    assert!(question("100.64.0.2", Some(QueryType::A)).is_err());
}

#[tokio::test]
async fn malicious_udp_oversize_wrong_endpoint_and_timeout_are_failures() {
    for behavior in [0, 1, 2, 3, 4] {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let endpoint = socket.local_addr().unwrap();
        let mut metadata = Status::unavailable();
        metadata.listener = Some(endpoint);
        metadata.listener_bound = true;
        let task = tokio::spawn(async move {
            let mut bytes = [0; 512];
            let (size, peer) = socket.recv_from(&mut bytes).await.unwrap();
            let request = Message::from_vec(&bytes[..size]).unwrap();
            let mut reply = Message::response(request.metadata.id, OpCode::Query);
            reply.queries = request.queries;
            match behavior {
                0 => reply.metadata.id = reply.metadata.id.wrapping_add(1),
                1 => {
                    reply.queries[0] =
                        Query::query(Name::from_ascii("wrong.test.").unwrap(), RecordType::A)
                }
                2 => {
                    socket.send_to(&[0; 513], peer).await.unwrap();
                    return;
                }
                3 => {
                    let other = UdpSocket::bind("127.0.0.1:0").await.unwrap();
                    other.send_to(&reply.to_vec().unwrap(), peer).await.unwrap();
                    return;
                }
                _ => {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    return;
                }
            }
            socket
                .send_to(&reply.to_vec().unwrap(), peer)
                .await
                .unwrap();
        });
        let (name, kind) = question("node.meshway.test", None).unwrap();
        let result = query_listener(metadata, name, kind, false).await;
        assert!(matches!(
            result,
            Err(Error::InvalidResponse | Error::Timeout)
        ));
        task.abort();
        let _ = task.await;
    }
}

#[tokio::test]
async fn malicious_tcp_framing_oversize_partial_extra_and_timeout_are_failures() {
    for behavior in [0, 1, 2, 3] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap();
        let mut metadata = Status::unavailable();
        metadata.listener = Some(endpoint);
        metadata.listener_bound = true;
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let length = socket.read_u16().await.unwrap();
            let mut request = vec![0; length as usize];
            socket.read_exact(&mut request).await.unwrap();
            match behavior {
                0 => socket.write_u16(4097).await.unwrap(),
                1 => {
                    socket.write_u16(100).await.unwrap();
                    socket.write_all(&[0; 12]).await.unwrap();
                }
                2 => {
                    let request = Message::from_vec(&request).unwrap();
                    let mut reply = Message::response(request.metadata.id, OpCode::Query);
                    reply.queries = request.queries;
                    let bytes = reply.to_vec().unwrap();
                    socket.write_u16(bytes.len() as u16).await.unwrap();
                    socket.write_all(&bytes).await.unwrap();
                    socket.write_all(&[0]).await.unwrap();
                }
                _ => tokio::time::sleep(Duration::from_secs(4)).await,
            }
        });
        let (name, kind) = question("node.meshway.test", None).unwrap();
        let result = query_listener(metadata, name, kind, true).await;
        assert!(matches!(
            result,
            Err(Error::InvalidResponse | Error::Timeout)
        ));
        task.abort();
        let _ = task.await;
    }
}
