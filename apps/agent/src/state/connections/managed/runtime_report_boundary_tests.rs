//! RR1/RR2 regressions adapted from the independent review's preserved probes.
use super::runtime_report_race_tests::{Transport, client, setup};
use super::*;
use crate::{
    generated::client::{HttpResult, HttpTransport},
    managed_connections::Observation,
};
use meshway_protocol::ConnectionNeighborState;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::watch;

#[tokio::test(start_paused = true)]
async fn pending_deadline_renewal_fences_report_without_second_attempt() {
    let (_dir, agent, mut manager, mut lease, observed, deadline) = setup(Duration::from_secs(30));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let http = client(Transport {
        requests: requests.clone(),
        wait: Duration::from_secs(2),
        status: 200,
        race: None,
        retire: None,
    });
    let update = tokio::spawn(async move {
        time::sleep(Duration::from_secs(1)).await;
        deadline.send_replace(Some(Instant::now() + Duration::from_secs(60)));
        time::sleep(Duration::from_secs(20)).await;
        drop(observed);
    });
    let result = agent.report_runtime(&mut manager, &mut lease, &http).await;
    assert!(matches!(result, Err(AgentError::RuntimeObservationChanged)));
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(lease.sequence, 1);
    // Observation cancellation is not itself data-plane withdrawal. The owner
    // still must independently fence actual grants/source/lease changes.
    assert_eq!(
        manager.observation().state,
        meshway_protocol::ConnectionRuntimeState::Applied
    );
    update.abort();
    manager.withdraw(Reason::ShuttingDown).await;
}

struct LateIo {
    wait: Duration,
    progressed: Arc<AtomicUsize>,
}
impl HttpTransport for LateIo {
    fn execute(
        &self,
        _request: reqwest::Request,
    ) -> std::pin::Pin<Box<dyn Future<Output = HttpResult<reqwest::Response>> + Send + '_>> {
        Box::pin(async move {
            time::sleep(self.wait).await;
            self.progressed.fetch_add(1, Ordering::SeqCst);
            std::future::pending().await
        })
    }
}

#[tokio::test(start_paused = true)]
async fn natural_lease_and_http_expiry_deny_pending_io_without_status() {
    for http_budget in [false, true] {
        let wait = Duration::from_secs(if http_budget { 10 } else { 1 });
        let (_dir, agent, mut manager, mut lease, _observed, _) =
            setup(Duration::from_secs(if http_budget { 30 } else { 1 }));
        let progressed = Arc::new(AtomicUsize::new(0));
        let http = HttpClient::new().with_transport(Arc::new(LateIo {
            wait,
            progressed: progressed.clone(),
        }));
        let result = agent.report_runtime(&mut manager, &mut lease, &http).await;
        if http_budget {
            assert!(matches!(
                result,
                Err(AgentError::Transport { retryable: true })
            ));
        } else {
            assert!(matches!(result, Err(AgentError::RuntimeObservationChanged)));
        }
        assert_eq!(progressed.load(Ordering::SeqCst), 0);
        manager.withdraw(Reason::ShuttingDown).await;
    }
}

struct WireAfterFence {
    wire: Mutex<std::net::TcpStream>,
    deadline: watch::Sender<Option<Instant>>,
}
impl HttpTransport for WireAfterFence {
    fn execute(
        &self,
        request: reqwest::Request,
    ) -> std::pin::Pin<Box<dyn Future<Output = HttpResult<reqwest::Response>> + Send + '_>> {
        Box::pin(async move {
            let until = (*self.deadline.borrow()).unwrap();
            time::sleep_until(until).await;
            assert!(
                self.deadline
                    .borrow()
                    .is_some_and(|until| until <= Instant::now())
            );
            std::io::Write::write_all(
                &mut *self.wire.lock().unwrap(),
                request.body().unwrap().as_bytes().unwrap(),
            )
            .unwrap();
            // A passive TCP sink never creates a status or response.
            std::future::pending().await
        })
    }
}

#[tokio::test]
async fn natural_expiry_delivers_zero_old_healthy_bytes_on_real_tcp() {
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let wire = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut receiver, _) = listener.accept().await.unwrap();
    let (_dir, agent, mut manager, mut lease, _observed, deadline) =
        setup(Duration::from_millis(100));
    let http = HttpClient::new().with_transport(Arc::new(WireAfterFence {
        wire: Mutex::new(wire.into_std().unwrap()),
        deadline,
    }));
    let result = agent.report_runtime(&mut manager, &mut lease, &http).await;
    assert!(matches!(result, Err(AgentError::RuntimeObservationChanged)));
    drop(http);
    let mut bytes = Vec::new();
    let count = time::timeout(Duration::from_secs(1), receiver.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        count, 0,
        "expired pending operation must never deliver the old Applied/healthy body"
    );
    assert!(bytes.is_empty());
    eprintln!(
        "RR2 FIXED WIRE: natural lease expiry, real TCP received 0 old report bytes; no HTTP response existed"
    );
    manager.withdraw(Reason::ShuttingDown).await;
}

struct DropCount(Arc<AtomicUsize>);
impl Drop for DropCount {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
struct BudgetTransport {
    started: Arc<AtomicUsize>,
    completed: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}
impl HttpTransport for BudgetTransport {
    fn execute(
        &self,
        request: reqwest::Request,
    ) -> std::pin::Pin<Box<dyn Future<Output = HttpResult<reqwest::Response>> + Send + '_>> {
        Box::pin(async move {
            let _drop = DropCount(self.dropped.clone());
            let index = self.started.fetch_add(1, Ordering::SeqCst);
            let report: ReportConnectionRuntimeRequest =
                serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
            time::sleep(Duration::from_secs(if index == 0 { 60 } else { 2 })).await;
            self.completed.fetch_add(1, Ordering::SeqCst);
            Ok(http::Response::builder()
                .status(200)
                .body(
                    serde_json::to_vec(&ReportConnectionRuntimeResponse {
                        epoch: report.epoch,
                        sequence: report.sequence,
                        reported_at: chrono::Utc::now(),
                    })
                    .unwrap(),
                )
                .unwrap()
                .into())
        })
    }
}

#[tokio::test(start_paused = true)]
async fn retry_preparation_keeps_remaining_budget_and_drops_both_futures() {
    let (_dir, agent, mut manager, mut lease, observed, deadline) = setup(Duration::from_secs(30));
    let original = *deadline.borrow();
    let started = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let http = HttpClient::new().with_transport(Arc::new(BudgetTransport {
        started: started.clone(),
        completed: completed.clone(),
        dropped: dropped.clone(),
    }));
    let publication = tokio::spawn(async move {
        time::sleep(Duration::from_secs(9)).await;
        observed.send_modify(|_| {});
        time::sleep(Duration::from_secs(20)).await;
    });
    let beginning = Instant::now();
    let result = agent.report_runtime(&mut manager, &mut lease, &http).await;
    assert!(matches!(
        result,
        Err(AgentError::Transport { retryable: true })
    ));
    assert_eq!(beginning.elapsed(), Duration::from_secs(10));
    assert_eq!(started.load(Ordering::SeqCst), 2);
    assert_eq!(dropped.load(Ordering::SeqCst), 2);
    assert_eq!(completed.load(Ordering::SeqCst), 0);
    assert_eq!(*deadline.borrow(), original);
    assert_eq!((lease.epoch, lease.sequence), (Some(1), 2));
    publication.abort();
    manager.withdraw(Reason::ShuttingDown).await;
}

struct CancellationPublication {
    observed: watch::Sender<Observation>,
    retire: Option<watch::Sender<Option<Instant>>>,
    dropped: Arc<AtomicUsize>,
}
impl Drop for CancellationPublication {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
        self.observed.send_modify(|value| {
            let peer = &mut value.peers[0];
            peer.state = ConnectionNeighborState::Unreachable;
            peer.probe_fresh = false;
            peer.probe_rtt_ms = None;
            peer.reason = Reason::ProbeFailed;
        });
        if let Some(deadline) = &self.retire {
            deadline.send_replace(None);
        }
    }
}
struct CancellationTransport {
    requests: Arc<Mutex<Vec<ReportConnectionRuntimeRequest>>>,
    observed: watch::Sender<Observation>,
    retire: Option<watch::Sender<Option<Instant>>>,
    dropped: Arc<AtomicUsize>,
}
impl HttpTransport for CancellationTransport {
    fn execute(
        &self,
        request: reqwest::Request,
    ) -> std::pin::Pin<Box<dyn Future<Output = HttpResult<reqwest::Response>> + Send + '_>> {
        Box::pin(async move {
            let report: ReportConnectionRuntimeRequest =
                serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
            let first = {
                let mut requests = self.requests.lock().unwrap();
                requests.push(report.clone());
                requests.len() == 1
            };
            if first {
                let _on_cancel = CancellationPublication {
                    observed: self.observed.clone(),
                    retire: self.retire.clone(),
                    dropped: self.dropped.clone(),
                };
                return std::future::pending().await;
            }
            Ok(http::Response::builder()
                .status(200)
                .body(
                    serde_json::to_vec(&ReportConnectionRuntimeResponse {
                        epoch: report.epoch,
                        sequence: report.sequence,
                        reported_at: chrono::Utc::now(),
                    })
                    .unwrap(),
                )
                .unwrap()
                .into())
        })
    }
}

#[tokio::test(start_paused = true)]
async fn drop_callback_precedes_negative_recapture_and_withdraw_forbids_attempt() {
    for retire in [false, true] {
        let (_dir, agent, mut manager, mut lease, observed, deadline) =
            setup(Duration::from_secs(30));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicUsize::new(0));
        let http = HttpClient::new().with_transport(Arc::new(CancellationTransport {
            requests: requests.clone(),
            observed: observed.clone(),
            retire: retire.then_some(deadline),
            dropped: dropped.clone(),
        }));
        let publication = tokio::spawn(async move {
            time::sleep(Duration::from_millis(100)).await;
            observed.send_modify(|_| {});
        });
        let result = agent.report_runtime(&mut manager, &mut lease, &http).await;
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        {
            let requests = requests.lock().unwrap();
            if retire {
                assert!(matches!(result, Err(AgentError::RuntimeObservationChanged)));
                assert_eq!(requests.len(), 1);
            } else {
                let (latest, _) = result.unwrap();
                assert_eq!(requests.len(), 2);
                assert!(requests[0].peers[0].probe_fresh && !latest.peers[0].probe_fresh);
                assert_eq!(latest.peers[0].state, ConnectionNeighborState::Unreachable);
                assert_eq!(latest.sequence, requests[0].sequence + 1);
                assert_eq!(
                    (latest.epoch, latest.revision, &latest.digest),
                    (requests[0].epoch, requests[0].revision, &requests[0].digest)
                );
            }
        }
        publication.await.unwrap();
        manager.withdraw(Reason::ShuttingDown).await;
    }
}

#[tokio::test(start_paused = true)]
async fn sequence_exhaustion_and_actual_withdrawn_report_never_wrap() {
    for initial in [u32::MAX, u32::MAX - 1] {
        let (_dir, agent, mut manager, mut lease, observed, _deadline) =
            setup(Duration::from_secs(30));
        lease.sequence = initial;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let http = client(Transport {
            requests: requests.clone(),
            wait: Duration::from_secs(60),
            status: 200,
            race: None,
            retire: None,
        });
        let publication = tokio::spawn(async move {
            time::sleep(Duration::from_secs(1)).await;
            observed.send_modify(|_| {});
            time::sleep(Duration::from_secs(20)).await;
        });
        let result = agent.report_runtime(&mut manager, &mut lease, &http).await;
        assert!(matches!(
            result,
            Err(AgentError::Invalid("runtime report sequence exhausted"))
        ));
        assert_eq!(lease.sequence, u32::MAX);
        assert_eq!(
            requests.lock().unwrap().len(),
            usize::from(initial != u32::MAX)
        );
        publication.abort();
        manager.withdraw(Reason::ShuttingDown).await;
        lease.sequence = 0;
        let http = client(Transport {
            requests: requests.clone(),
            wait: Duration::ZERO,
            status: 200,
            race: None,
            retire: None,
        });
        let (request, _) = agent
            .report_runtime(&mut manager, &mut lease, &http)
            .await
            .unwrap();
        assert_eq!(
            request.state,
            meshway_protocol::ConnectionRuntimeState::Withdrawn
        );
        assert!(request.peers.is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn received_422_and_closed_watch_keep_terminal_and_proof_boundaries() {
    let (_dir, agent, mut manager, mut lease, observed, _deadline) = setup(Duration::from_secs(30));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let http = client(Transport {
        requests: requests.clone(),
        wait: Duration::ZERO,
        status: 422,
        race: Some(observed.clone()),
        retire: None,
    });
    let result = agent.report_runtime(&mut manager, &mut lease, &http).await;
    assert!(matches!(
        result,
        Err(AgentError::Http {
            status: 422,
            retryable: true
        })
    ));
    assert_eq!(requests.lock().unwrap().len(), 1);
    let (_, guard) = manager.report_snapshot();
    let guard = guard.unwrap();
    drop(http);
    drop(observed);
    assert!(!guard.generation_current() && !guard.current());
    manager.withdraw(Reason::ShuttingDown).await;
}
