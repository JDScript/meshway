//! Real App provider projection and private IPC across DNS guard transitions.
use super::{ProbeError, SCHEMA_VERSION, Server as Control, dns::*};
use hickory_proto::rr::Name;
use meshway_agent_dns::{Guard, Server, Zone};
use std::{
    collections::BTreeMap,
    fs,
    net::{IpAddr, SocketAddr},
    os::unix::fs::PermissionsExt,
    path::Path,
    time::Duration,
};
use tokio::time::Instant;

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
async fn paced_status(path: &Path) -> Result<Status, Error> {
    tokio::time::sleep(Duration::from_millis(150)).await;
    status(path).await
}
async fn paced_query(
    path: &Path,
    input: &str,
    kind: Option<QueryType>,
    tcp: bool,
) -> Result<Observation, Error> {
    tokio::time::sleep(Duration::from_millis(150)).await;
    query(path, input, kind, tcp).await
}

#[tokio::test]
async fn dns_only_private_ipc_and_real_udp_tcp_queries_follow_guard_transitions() {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let state = dir.path().join("identity");
    let lock = meshway_protocol::local::lock(&dir.path().join("lock")).unwrap();
    let control = Control::bind(&state, &lock).await.unwrap();
    let temporary = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address: SocketAddr = temporary.local_addr().unwrap();
    drop(temporary);
    let server = Server::bind(address).await.unwrap();
    let guard = Guard::default();
    guard.listener_bound(address, Some("real-node".into()));
    let task_guard = guard.clone();
    let dns_task = tokio::spawn(async move { server.serve(task_guard).await });
    let task_guard = guard.clone();
    let control_task =
        tokio::spawn(async move { control.serve_with_dns(None, Some(task_guard)).await });
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
    assert_eq!(
        serde_json::to_value(&active).unwrap(),
        serde_json::json!({
            "schema_version": SCHEMA_VERSION, "state": "active", "generation": active.generation,
            "revision_floor": 2, "active_revision": 2, "zone": "meshway.test.", "ttl": 30,
            "lease_remaining_ms": active.lease_remaining_ms, "listener": address.to_string(),
            "listener_bound": true, "network_id": "real-network", "node_id": "real-node", "evidence": "current_dns_guard",
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
    let changed = paced_status(&state).await.unwrap();
    assert_eq!(changed.zone.as_deref(), Some("changed.test."));
    assert_eq!(changed.network_id, None);
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
        super::live_status(&state, false, 0, 64).await.unwrap_err(),
        ProbeError::Unavailable
    );
    control_task.abort();
    dns_task.abort();
    let _ = control_task.await;
    let _ = dns_task.await;
    assert!(status(&state).await.is_err());
}
