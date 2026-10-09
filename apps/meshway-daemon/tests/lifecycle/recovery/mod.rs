mod proxy;
use super::support::{self, Process, Server, request_id};
use meshway_control_client::{ApproveDeviceAuthorizationPath, GetNodeStatePath, RevokeNodeKeyPath};
use meshway_ipc::{Command, Phase, Problem, Status};
use meshway_protocol::{
    ApproveEnrollmentRequest, LoginRequest, RevokeNodeKeyRequest, WireSecret, state::Freshness,
};
use std::{sync::atomic::Ordering, time::Duration};
use tokio::time::Instant;

struct Fixture {
    daemon: Process,
    server: Server,
    proxy: proxy::Proxy,
    token: WireSecret,
    ready: Status,
    _temp: tempfile::TempDir,
}
impl Fixture {
    async fn start() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let server = Server::start(root.join("server")).await;
        let (token, network) = server.owner().await;
        let proxy = proxy::Proxy::start(&server.origin).await;
        let daemon = Process::start(&root.join("node"), &proxy.origin);
        let initial = support::wait(&daemon.socket, Phase::Down).await;
        support::call(
            &daemon.socket,
            Command::Up {
                enrollment_key: None,
                expected_revision: initial.revision,
            },
            request_id(),
        )
        .await
        .unwrap();
        let pending = support::wait(&daemon.socket, Phase::AwaitingApproval).await;
        server
            .client
            .approve_device_authorization(
                &ApproveDeviceAuthorizationPath {
                    enrollment_request_id: pending.enrollment_request_id.unwrap(),
                },
                &ApproveEnrollmentRequest {
                    network_id: network,
                },
                token.expose(),
            )
            .await
            .unwrap();
        let ready = support::wait(&daemon.socket, Phase::ControlReady).await;
        Self {
            daemon,
            server,
            proxy,
            token,
            ready,
            _temp: temp,
        }
    }
    async fn status(&self, predicate: impl Fn(&Status) -> bool) -> Status {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let status = support::status(&self.daemon.socket).await.unwrap();
            assert_eq!(
                status.revision, self.ready.revision,
                "recovery must not manufacture an operator revision"
            );
            assert_eq!(status.identity, self.ready.identity);
            assert_eq!(status.process_id, self.ready.process_id);
            assert!(status.desired_up);
            if predicate(&status) {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "automatic convergence timed out: {:?} {:?}",
                status.phase,
                status.last_error
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    async fn current_report(&self, status: &Status) {
        let binding = status.identity.as_ref().unwrap();
        let expected = status.instance.as_ref().unwrap().instance_id;
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let view = self
                .server
                .client
                .get_node_state(
                    &GetNodeStatePath {
                        network_id: binding.network_id,
                        node_id: binding.node_id,
                    },
                    self.token.expose(),
                )
                .await
                .unwrap();
            let state = &view.nodes[&binding.node_id];
            if state.freshness == Freshness::Current
                && state.instance_id == Some(expected)
                && state.report.is_some()
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "fresh instance did not report State"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    async fn close(mut self) {
        self.proxy.mode(proxy::HEALTHY);
        self.daemon.exit("-TERM").await;
        self.proxy.close().await;
        self.server.close().await;
    }
}

#[tokio::test]
async fn gateway_html_and_stale_instance_recover_without_operator_up() {
    // C: faults are discovered by real 20 s control cycles and retry timers;
    // wait on observed phases/receipts while the daemon owns lease renewal.
    let fixture = Fixture::start().await;
    fixture.current_report(&fixture.ready).await;
    for mode in [proxy::GATEWAY, proxy::HTML] {
        fixture.proxy.mode(mode);
        let failed = fixture.status(|s| s.phase == Phase::Retrying).await;
        assert!(failed.retry_at.is_some());
        if mode == proxy::HTML {
            assert!(
                !failed.authenticated && failed.instance.is_none(),
                "malformed responses never grant authority"
            );
        }
        fixture.proxy.mode(proxy::HEALTHY);
        let restored = fixture
            .status(|s| s.phase == Phase::ControlReady && s.authenticated)
            .await;
        fixture.current_report(&restored).await;
    }
    let before = support::status(&fixture.daemon.socket)
        .await
        .unwrap()
        .instance
        .unwrap();
    fixture
        .proxy
        .faults
        .invalidate_instance
        .store(true, Ordering::SeqCst);
    let restored = fixture
        .status(|s| {
            s.phase == Phase::ControlReady
                && s.instance
                    .as_ref()
                    .is_some_and(|i| i.instance_id != before.instance_id)
        })
        .await;
    fixture.current_report(&restored).await;
    fixture.close().await;
}

#[tokio::test]
async fn server_restart_replaces_stale_session_and_instance_without_operator_up() {
    // C: the real control poll/retry cycle must discover the new Server epoch;
    // observed instance replacement, rather than a sleep, completes recovery.
    let mut fixture = Fixture::start().await;
    let original = fixture.ready.instance.as_ref().unwrap().instance_id;
    fixture.proxy.mode(proxy::GATEWAY);
    fixture.status(|s| s.phase == Phase::Retrying).await;
    fixture.server.close().await;
    // Same durable Server identity, a genuinely new boot/session/instance scope.
    fixture.server =
        Server::start(fixture._temp.path().canonicalize().unwrap().join("server")).await;
    fixture.token = fixture
        .server
        .client
        .login(&LoginRequest {
            email: "daemon@example.test".into(),
            password: WireSecret::new("daemon test password".into()),
        })
        .await
        .unwrap()
        .token;
    fixture.proxy.backend(&fixture.server.origin);
    fixture.proxy.mode(proxy::HEALTHY);
    let restored = fixture
        .status(|s| {
            s.phase == Phase::ControlReady
                && s.instance
                    .as_ref()
                    .is_some_and(|i| i.instance_id != original)
        })
        .await;
    fixture.current_report(&restored).await;
    fixture.close().await;
}

#[tokio::test]
async fn revoked_control_key_stays_blocked_after_transport_recovers() {
    // C: retain a real 6 s window across control's 1 s / 2 s initial backoff;
    // an immediate Blocked projection alone cannot prove no reauthentication.
    let fixture = Fixture::start().await;
    let binding = fixture.ready.identity.as_ref().unwrap();
    fixture
        .server
        .client
        .revoke_node_key(
            &RevokeNodeKeyPath {
                network_id: binding.network_id,
                node_id: binding.node_id,
            },
            &RevokeNodeKeyRequest {
                key_id: binding.keys.control.id,
                expected_generation: binding.keys.control.generation,
            },
            fixture.token.expose(),
        )
        .await
        .unwrap();
    let rejected = fixture.status(|s| s.phase == Phase::Blocked).await;
    assert_eq!(rejected.last_error, Some(Problem::Credentials));
    assert!(!rejected.authenticated && rejected.instance.is_none() && rejected.retry_at.is_none());
    let count = fixture.proxy.faults.challenges.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(
        fixture.proxy.faults.challenges.load(Ordering::SeqCst),
        count
    );
    fixture.close().await;
}

#[tokio::test]
async fn wrong_controller_response_is_fatal_not_transient_delivery() {
    // C: retain a real 6 s window across control's 1 s / 2 s initial backoff;
    // restoring HTTP must not schedule a new challenge for a fatal binding.
    let fixture = Fixture::start().await;
    fixture.proxy.mode(proxy::WRONG_CONTROLLER);
    fixture
        .proxy
        .faults
        .invalidate_instance
        .store(true, Ordering::SeqCst);
    let rejected = fixture.status(|s| s.phase == Phase::Blocked).await;
    assert_eq!(rejected.last_error, Some(Problem::Contract));
    assert!(!rejected.authenticated && rejected.instance.is_none());
    fixture.proxy.mode(proxy::HEALTHY);
    let count = fixture.proxy.faults.challenges.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(
        fixture.proxy.faults.challenges.load(Ordering::SeqCst),
        count
    );
    fixture.close().await;
}

#[tokio::test]
#[ignore = "real Server session expiry (five minutes); explicit outage conformance"]
async fn outage_past_real_session_and_instance_expiry_recovers_and_reports_without_up() {
    let fixture = Fixture::start().await;
    fixture.current_report(&fixture.ready).await;
    let original = fixture.ready.instance.as_ref().unwrap();
    let expiry = fixture
        .proxy
        .faults
        .session_expiry
        .load(Ordering::SeqCst)
        .max(original.expires_at_ms);
    assert!(expiry > 0);
    fixture.proxy.mode(proxy::GATEWAY);
    fixture.status(|s| s.phase == Phase::Retrying).await;
    let now = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap();
    // Use actual observed Server deadlines; no production TTL or response is shortened.
    tokio::time::sleep(Duration::from_millis(expiry.saturating_sub(now) + 500)).await;
    let expired = fixture
        .status(|s| !s.authenticated && s.instance.is_none())
        .await;
    assert_ne!(expired.phase, Phase::Blocked);
    assert!(expired.applied.is_none());
    fixture.proxy.mode(proxy::HEALTHY);
    let restored = fixture
        .status(|s| {
            s.phase == Phase::ControlReady
                && s.instance
                    .as_ref()
                    .is_some_and(|i| i.instance_id != original.instance_id)
        })
        .await;
    assert!(restored.instance.as_ref().unwrap().expires_at_ms > original.expires_at_ms);
    fixture.current_report(&restored).await;
    fixture.close().await;
}
