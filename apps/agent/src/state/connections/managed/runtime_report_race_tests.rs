//! Deterministic generated-transport injection for races and bounded retry.
//! The sibling recovery test independently exercises real Controller/TCP peers.
use super::*;
use crate::{
    generated::client::{HttpResult, HttpTransport},
    managed_connections::{
        Observation,
        tests::{fixture, report_generation, snapshot},
    },
};
use meshway_protocol::{
    ConnectionNeighborObservation, ConnectionNeighborState, ConnectionTransport,
};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

struct Transport {
    requests: Arc<Mutex<Vec<ReportConnectionRuntimeRequest>>>,
    wait: Duration,
    status: u16,
    race: Option<watch::Sender<Observation>>,
    retire: Option<watch::Sender<Option<Instant>>>,
}

impl HttpTransport for Transport {
    fn execute(
        &self,
        request: reqwest::Request,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = HttpResult<reqwest::Response>> + Send + '_>,
    > {
        Box::pin(async move {
            let report: ReportConnectionRuntimeRequest =
                serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
            self.requests.lock().unwrap().push(report.clone());
            time::sleep(self.wait).await;
            if let Some(observed) = &self.race {
                observed.send_modify(|_| {});
            }
            if let Some(deadline) = &self.retire {
                deadline.send_replace(None);
            }
            let bytes = serde_json::to_vec(&ReportConnectionRuntimeResponse {
                epoch: report.epoch,
                sequence: report.sequence,
                reported_at: chrono::Utc::now(),
            })
            .unwrap();
            Ok(http::Response::builder()
                .status(self.status)
                .body(bytes)
                .unwrap()
                .into())
        })
    }
}

fn setup(
    lease_duration: Duration,
) -> (
    tempfile::TempDir,
    Agent,
    Manager,
    RuntimeLease,
    watch::Sender<Observation>,
    watch::Sender<Option<Instant>>,
) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let agent = Agent::prepare_enrollment(
        dir.path().join("agent.json"),
        "http://127.0.0.1:8420",
        &"a".repeat(64),
        "report-race",
    )
    .unwrap();
    let observation = Observation {
        state: meshway_protocol::ConnectionRuntimeState::Applied,
        reason: Reason::None,
        listeners: vec![],
        peers: vec![ConnectionNeighborObservation {
            peer_node_id: "b".repeat(64),
            transport: ConnectionTransport::NativeTcp,
            initiator_node_id: "a".repeat(64),
            listener_id: Some("listener".into()),
            candidate_id: Some("candidate".into()),
            state: ConnectionNeighborState::Authenticated,
            reason: Reason::None,
            probe_fresh: true,
            probe_rtt_ms: Some(200),
        }],
    };
    let (mut manager, observed, deadline) = report_generation(observation, lease_duration);
    let (intent, _, _, _) = fixture(ConnectionTransport::NativeTcp);
    manager.snapshot = Some(snapshot(intent));
    let mut lease = RuntimeLease::new().unwrap();
    lease.epoch = Some(1);
    (dir, agent, manager, lease, observed, deadline)
}

fn client(transport: Transport) -> HttpClient {
    HttpClient::new().with_transport(Arc::new(transport))
}

#[tokio::test(start_paused = true)]
async fn runtime_report_retry_shares_original_http_budget_and_does_not_renew_lease() {
    let (_dir, agent, mut manager, mut lease, observed, deadline) = setup(Duration::from_secs(30));
    let original = *deadline.borrow();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let http = client(Transport {
        requests: requests.clone(),
        wait: Duration::from_secs(60),
        status: 200,
        race: None,
        retire: None,
    });
    let publication = tokio::spawn(async move {
        time::sleep(Duration::from_secs(8)).await;
        observed.send_modify(|value| value.peers[0].probe_rtt_ms = Some(201));
        time::sleep(Duration::from_secs(60)).await;
    });
    let started = Instant::now();
    let result = agent.report_runtime(&mut manager, &mut lease, &http).await;
    assert!(matches!(
        result,
        Err(AgentError::Transport { retryable: true })
    ));
    assert_eq!(started.elapsed(), Duration::from_secs(10));
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(*deadline.borrow(), original);
    assert_eq!(lease.epoch, Some(1));
    assert_eq!(lease.sequence, 2);
    publication.abort();
    manager.withdraw(Reason::ShuttingDown).await;
}

#[tokio::test(start_paused = true)]
async fn runtime_report_continuous_changes_have_three_attempt_bound() {
    let (_dir, agent, mut manager, mut lease, observed, deadline) = setup(Duration::from_secs(30));
    let original = *deadline.borrow();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let http = client(Transport {
        requests: requests.clone(),
        wait: Duration::from_secs(60),
        status: 200,
        race: None,
        retire: None,
    });
    let publication = tokio::spawn(async move {
        loop {
            time::sleep(Duration::from_secs(1)).await;
            observed.send_modify(|_| {});
        }
    });
    let started = Instant::now();
    let result = agent.report_runtime(&mut manager, &mut lease, &http).await;
    assert!(matches!(result, Err(AgentError::RuntimeObservationChanged)));
    assert_eq!(requests.lock().unwrap().len(), 3);
    assert_eq!(started.elapsed(), Duration::from_secs(3));
    assert_eq!(*deadline.borrow(), original);
    publication.abort();
    manager.withdraw(Reason::ShuttingDown).await;
}

#[tokio::test(start_paused = true)]
async fn runtime_report_failed_peer_cancels_old_healthy_body_and_recaptures_negative() {
    let (_dir, agent, mut manager, mut lease, observed, _) = setup(Duration::from_secs(30));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let http = client(Transport {
        requests: requests.clone(),
        wait: Duration::from_secs(1),
        status: 200,
        race: None,
        retire: None,
    });
    let publication = tokio::spawn(async move {
        time::sleep(Duration::from_millis(100)).await;
        observed.send_modify(|value| {
            let peer = &mut value.peers[0];
            peer.state = ConnectionNeighborState::Unreachable;
            peer.reason = Reason::ProbeFailed;
            peer.probe_fresh = false;
            peer.probe_rtt_ms = None;
        });
        time::sleep(Duration::from_secs(60)).await;
    });
    let (request, _) = agent
        .report_runtime(&mut manager, &mut lease, &http)
        .await
        .unwrap();
    assert_eq!(request.sequence, 2);
    assert_eq!(request.peers[0].state, ConnectionNeighborState::Unreachable);
    assert!(!request.peers[0].probe_fresh && request.peers[0].probe_rtt_ms.is_none());
    assert_eq!(requests.lock().unwrap().len(), 2);
    publication.abort();
    manager.withdraw(Reason::ShuttingDown).await;
}

#[tokio::test(start_paused = true)]
async fn runtime_report_withdraw_source_fence_and_lease_expiry_do_not_retry_old_generation() {
    for mode in 0..3 {
        let duration = if mode == 2 {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(30)
        };
        let (_dir, agent, mut manager, mut lease, observed, deadline) = setup(duration);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let http = client(Transport {
            requests: requests.clone(),
            wait: Duration::from_secs(60),
            status: 200,
            race: None,
            retire: None,
        });
        let publication = tokio::spawn(async move {
            time::sleep(Duration::from_millis(100)).await;
            match mode {
                0 => {
                    observed.send_modify(|value| {
                        value.state = meshway_protocol::ConnectionRuntimeState::Withdrawn
                    });
                }
                1 => {
                    deadline.send_replace(None);
                }
                _ => {}
            }
            time::sleep(Duration::from_secs(60)).await;
        });
        let result = agent.report_runtime(&mut manager, &mut lease, &http).await;
        assert!(
            matches!(result, Err(AgentError::RuntimeObservationChanged)),
            "mode {mode}: {result:?}"
        );
        assert_eq!(requests.lock().unwrap().len(), 1);
        publication.abort();
        manager.withdraw(Reason::ShuttingDown).await;
    }
}

#[tokio::test(start_paused = true)]
async fn runtime_report_received_late_403_409_win_over_observation_and_retirement_races() {
    for status in [403, 409] {
        for retirement in [false, true] {
            let (_dir, agent, mut manager, mut lease, observed, deadline) =
                setup(Duration::from_secs(30));
            let requests = Arc::new(Mutex::new(Vec::new()));
            // Publish exactly when the delayed transport returns the response;
            // both branches are ready. The generated status must be handled first.
            let http = client(Transport {
                requests: requests.clone(),
                wait: Duration::from_secs(1),
                status,
                race: Some(observed),
                retire: retirement.then_some(deadline),
            });
            let result = agent.report_runtime(&mut manager, &mut lease, &http).await;
            if status == 403 {
                assert!(matches!(
                    result,
                    Err(AgentError::Http {
                        status: 403,
                        retryable: false
                    })
                ));
            } else {
                assert!(matches!(
                    result,
                    Err(AgentError::Invalid(
                        "runtime lease superseded; refusing to reacquire"
                    ))
                ));
                assert_eq!(manager.observation().reason, Reason::LeaseSuperseded);
            }
            assert_eq!(requests.lock().unwrap().len(), 1);
            assert_eq!(lease.epoch, Some(1));
            manager.withdraw(Reason::ShuttingDown).await;
        }
    }
}
