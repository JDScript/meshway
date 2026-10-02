//! Real controller + normal managed_tick + two authenticated TCP stream peers.
//! Delay only the runtime HTTP request, leaving the real five-second probes running.
use super::*;
use meshway_protocol::*;
use serde::{Serialize, de::DeserializeOwned};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

async fn json<T: DeserializeOwned>(request: reqwest::RequestBuilder) -> T {
    let response = request.send().await.unwrap().error_for_status().unwrap();
    serde_json::from_slice(&response.bytes().await.unwrap()).unwrap()
}

fn body(request: reqwest::RequestBuilder, value: &impl Serialize) -> reqwest::RequestBuilder {
    request
        .header("content-type", "application/json")
        .body(serde_json::to_vec(value).unwrap())
}

async fn serve(
    path: &std::path::Path,
    address: std::net::SocketAddr,
) -> tokio::task::JoinHandle<()> {
    let state = meshway_controller::AppState::open(path).unwrap();
    let app = meshway_controller::router(state);
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() })
}

// Delay request delivery on the wire, not merely Controller handler completion:
// an HTTP future cancelled before delivery must not accidentally commit later.
async fn proxy(
    upstream: std::net::SocketAddr,
    delay: Arc<AtomicU64>,
    cancelled: Arc<AtomicU64>,
    delivered: Arc<AtomicU64>,
) -> (String, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            let (mut downstream, _) = listener.accept().await.unwrap();
            let delay = delay.clone();
            let cancelled = cancelled.clone();
            let delivered = delivered.clone();
            tasks.spawn(async move {
                let mut bytes = Vec::new();
                let mut buffer = [0; 8192];
                let end = loop {
                    let read = downstream.read(&mut buffer).await.unwrap();
                    if read == 0 { return; }
                    bytes.extend_from_slice(&buffer[..read]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") { break end + 4; }
                    assert!(bytes.len() < 256 * 1024);
                };
                let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                let report = headers.starts_with("POST /api/v1/agent/connections/runtime HTTP/");
                let length = headers.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                }).unwrap_or(0);
                while bytes.len() < end + length {
                    let read = downstream.read(&mut buffer).await.unwrap();
                    if read == 0 { return; }
                    bytes.extend_from_slice(&buffer[..read]);
                }
                if report {
                    tokio::select! {
                        _ = time::sleep(Duration::from_millis(delay.load(Ordering::SeqCst))) => {},
                        read = downstream.read(&mut buffer) => {
                            assert_eq!(read.unwrap(), 0, "no pipelined request before first response");
                            cancelled.fetch_add(1, Ordering::SeqCst);
                            return;
                        },
                    }
                }
                let Ok(mut upstream) = tokio::net::TcpStream::connect(upstream).await else { return; };
                if upstream.write_all(&bytes).await.is_err() { return; }
                if report { delivered.fetch_add(1, Ordering::SeqCst); }
                let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
            });
            while tasks.try_join_next().is_some() {}
        }
    });
    (base, task)
}

#[tokio::test]
async fn runtime_report_periodic_two_stream_peers_recovers_after_controller_restart() {
    let dir = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let controller = dir.path().join("controller");
    let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let delay = Arc::new(AtomicU64::new(0));
    let cancelled = Arc::new(AtomicU64::new(0));
    let delivered = Arc::new(AtomicU64::new(0));
    let mut server = serve(&controller, address).await;
    let (base, proxy_task) =
        proxy(address, delay.clone(), cancelled.clone(), delivered.clone()).await;
    let admin = meshway_controller::admin_token(&controller).unwrap();
    let http = reqwest::Client::new();
    let networks: NetworkList = json(
        http.get(format!("{base}/api/v1/networks"))
            .bearer_auth(&admin),
    )
    .await;
    let network = &networks.networks[0].network_id;
    let mut agents = Vec::new();
    let mut managers = Vec::new();
    let mut leases = Vec::new();
    for index in 0..3 {
        let token: CreateEnrollmentTokenResponse = json(body(
            http.post(format!(
                "{base}/api/v1/networks/{network}/enrollment-tokens"
            ))
            .bearer_auth(&admin),
            &CreateEnrollmentTokenRequest {
                expires_in_seconds: 3600,
            },
        ))
        .await;
        let mut agent = Agent::prepare_enrollment(
            dir.path().join(format!("agent-{index}.json")),
            &base,
            &token.token,
            &format!("report-{index}"),
        )
        .unwrap();
        agent.enroll(&token.token).await.unwrap();
        agent.publish_connection_identity().await.unwrap();
        agents.push(agent);
        managers.push(Manager::new(None, Duration::from_secs(30)));
        leases.push(RuntimeLease::new().unwrap());
    }
    // Remote generations remain alive through the >60s stale-window experiment.
    managers[1] = Manager::new(None, Duration::from_secs(300));
    managers[2] = Manager::new(None, Duration::from_secs(300));
    let port = crate::managed_connections::tests::listener_port(ConnectionTransport::NativeTcp);
    let endpoint = ConnectionEndpoint {
        host: "127.0.0.1".into(),
        port,
    };
    let configuration: ConnectionsSnapshot = json(
        http.get(format!("{base}/api/v1/networks/{network}/connections"))
            .bearer_auth(&admin),
    )
    .await;
    let _: ConnectionsSnapshot = json(body(
        http.post(format!("{base}/api/v1/networks/{network}/connections"))
            .bearer_auth(&admin),
        &SaveConnectionsRequest {
            expected_revision: configuration.revision,
            configuration: ConnectionPlan {
                listeners: vec![ConnectionListener {
                    id: "report-stream".into(),
                    node_id: agents[0].identity.node_id.clone().unwrap(),
                    transport: ConnectionTransport::NativeTcp,
                    bind: format!("127.0.0.1:{port}"),
                    advertised_endpoints: vec![endpoint.clone()],
                    credential_ref: None,
                }],
                candidates: (1..3)
                    .map(|index| ConnectionCandidate {
                        id: format!("report-dial-{index}"),
                        dialer_node_id: agents[index].identity.node_id.clone().unwrap(),
                        listener_id: "report-stream".into(),
                        endpoint: endpoint.clone(),
                        priority: 0,
                        credential_ref: None,
                    })
                    .collect(),
            },
        },
    ))
    .await;
    for index in 0..3 {
        let result = agents[index]
            .managed_tick(
                &mut managers[index],
                &mut leases[index],
                &mut false,
                &mut None,
            )
            .await;
        assert!(
            matches!(result, Ok(()) | Err(AgentError::RuntimeObservationChanged)),
            "initial tick {index}: {result:?}"
        );
    }
    time::timeout(Duration::from_secs(8), async {
        loop {
            let observed = managers[0].observation();
            if observed.peers.len() == 2 && observed.peers.iter().all(|peer| peer.probe_fresh) {
                break;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    agents[0]
        .managed_tick(&mut managers[0], &mut leases[0], &mut false, &mut None)
        .await
        .unwrap();
    let runtime_url = format!("{base}/api/v1/networks/{network}/connections/runtime");
    let initial: ConnectionRuntimesSnapshot =
        json(http.get(&runtime_url).bearer_auth(&admin)).await;
    let node = agents[0].identity.node_id.clone().unwrap();
    let initial_at = initial
        .nodes
        .iter()
        .find(|row| row.node_id == node)
        .unwrap()
        .reported_at;
    delay.store(350, Ordering::SeqCst);
    // The independent baseline run had 13 aligned cancellations and a >60s
    // stale public report. Every aligned fixed tick must now advance reported_at.
    let mut previous_at = initial_at;
    for cycle in 0..3 {
        let (_, guard) = managers[0].report_snapshot();
        guard.unwrap().changed().await;
        time::sleep(Duration::from_millis(4800)).await;
        let before_cancel = cancelled.load(Ordering::SeqCst);
        let before_delivery = delivered.load(Ordering::SeqCst);
        let mut synced = false;
        let result = agents[0]
            .managed_tick(&mut managers[0], &mut leases[0], &mut synced, &mut None)
            .await;
        assert!(synced);
        assert!(result.is_ok(), "aligned cycle {cycle}: {result:?}");
        let runtime: ConnectionRuntimesSnapshot =
            json(http.get(&runtime_url).bearer_auth(&admin)).await;
        let report = runtime
            .nodes
            .iter()
            .find(|row| row.node_id == node)
            .unwrap();
        assert!(report.reported_at > previous_at && !report.stale && report.current);
        previous_at = report.reported_at;
        assert_eq!(report.peers.len(), 2);
        assert!(report.peers.iter().all(|peer| peer.probe_fresh));
        assert!(
            cancelled.load(Ordering::SeqCst) > before_cancel,
            "aligned cycle must really cancel a pre-delivery old request"
        );
        assert!(delivered.load(Ordering::SeqCst) > before_delivery);
        eprintln!(
            "RR12 REAL CYCLE {cycle}: epoch {} sequence {} reported_at {:?} cancelled={} delivered={}",
            report.epoch,
            report.sequence,
            report.reported_at,
            cancelled.load(Ordering::SeqCst),
            delivered.load(Ordering::SeqCst)
        );
    }
    server.abort();
    let _ = server.await;
    let outage = agents[0]
        .managed_tick(&mut managers[0], &mut leases[0], &mut false, &mut None)
        .await;
    assert!(outage.is_err());
    time::sleep(Duration::from_secs(31)).await;
    assert_eq!(
        managers[0].observation().state,
        ConnectionRuntimeState::Withdrawn
    );
    server = serve(&controller, address).await;
    for index in 0..3 {
        let result = agents[index]
            .managed_tick(
                &mut managers[index],
                &mut leases[index],
                &mut false,
                &mut None,
            )
            .await;
        assert!(
            matches!(result, Ok(()) | Err(AgentError::RuntimeObservationChanged)),
            "restart {index}: {result:?}"
        );
    }
    time::sleep(Duration::from_secs(5)).await;
    let (_, guard) = managers[0].report_snapshot();
    guard.unwrap().changed().await;
    time::sleep(Duration::from_millis(4800)).await;
    let result = agents[0]
        .managed_tick(&mut managers[0], &mut leases[0], &mut false, &mut None)
        .await;
    assert!(
        result.is_ok(),
        "post-controller-restart aligned tick: {result:?}"
    );
    let runtime: ConnectionRuntimesSnapshot =
        json(http.get(&runtime_url).bearer_auth(&admin)).await;
    let report = runtime
        .nodes
        .iter()
        .find(|row| row.node_id == node)
        .unwrap();
    assert!(report.reported_at > previous_at && !report.stale && report.current);
    assert_eq!(report.epoch, leases[0].epoch.unwrap());
    assert_eq!(report.peers.len(), 2);
    assert!(report.peers.iter().all(|peer| peer.probe_fresh));
    eprintln!(
        "RR12 REAL RECOVERY: epoch {} sequence {} reported_at {:?} fresh={} current={}",
        report.epoch, report.sequence, report.reported_at, !report.stale, report.current
    );
    for manager in &mut managers {
        manager.withdraw(Reason::ShuttingDown).await;
    }
    server.abort();
    proxy_task.abort();
}
