//! Genuine held-key enrollment and proof sessions, with response-delivery faults.
use super::{
    faults::{Fault, Proxy},
    support::{Owner, Server, request},
    system,
};
use meshway_control::{Control, Error};
use meshway_control_client::{DeleteNodePath, GetNetworkPath};
use meshway_protocol::DeleteNodeRequest;
use std::time::Duration;

const SESSION: &str = "/auth/session";
const CHALLENGE: &str = "/auth/challenges";

async fn connected(server: &Server, owner: &Owner, origin: &str, name: &str) -> Control {
    let mut control = Control::open(
        &server.path(name),
        origin,
        reqwest::Client::builder().no_proxy(),
    )
    .unwrap();
    server.enroll(owner, &mut control, name).await;
    control.authenticate().await.unwrap();
    control.acquire_instance(Default::default()).await.unwrap();
    control.sync_configuration().await.unwrap();
    control
}

#[tokio::test]
async fn explicit_refresh_preserves_instance_tickets_pending_state_and_original_deadlines() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let wire = wire::Observer::start(&server.origin).await;
    let proxy = Proxy::start(&wire.origin).await;
    let mut control = connected(&server, &owner, &proxy.origin, "session-handoff").await;
    let ticket = control.begin_application().await.unwrap();
    proxy.fault("/agent-api/v1/state", Fault::Lost);
    assert!(control.report_control_state(system()).await.is_err());
    let old = control.execution_snapshot().unwrap();
    let deadline = old.expires_at();
    let before = control.status();
    control.authenticate().await.unwrap();
    assert_eq!(
        proxy.count(SESSION),
        1,
        "ordinary authentication reuses a long-lived session"
    );
    control.refresh_authentication().await.unwrap();
    assert_eq!(proxy.count(SESSION), 2);
    assert_eq!(
        control.status().instance,
        before.instance,
        "authentication must not renew either lease"
    );
    assert_eq!(control.status().cached, before.cached);
    assert_eq!(old.expires_at(), deadline);
    old.ensure_current().unwrap();
    let fresh = control.execution_snapshot().unwrap();
    assert!(fresh.same_execution(&old));
    fresh.continuation_from(&old).unwrap();
    let accepted = control.retry_state_report().await.unwrap();
    assert!(accepted.duplicate);
    assert_eq!(accepted.sequence.get(), 1);
    control
        .finish_application(&ticket, Ok(()), chrono::Utc::now())
        .await
        .unwrap();
    control.sync_configuration().await.unwrap();
    let sessions = wire.sessions();
    assert_eq!(sessions.len(), 2);
    assert_ne!(sessions[0].intent, sessions[1].intent);
    assert_ne!(sessions[0].token_digest, sessions[1].token_digest);
    assert_eq!(wire.last_query_token(), Some(sessions[1].token_digest));
    control.stop().await.unwrap();
    assert!(old.ensure_current().is_err() && fresh.ensure_current().is_err());
    // Restarting a stopped driver still reuses its valid parked session.
    control.authenticate().await.unwrap();
    assert_eq!(proxy.count(SESSION), 2);
    assert!(control.status().instance.is_none());
    control.shutdown().await.unwrap();
    proxy.close().await;
    wire.close().await;
    server.close().await;
}

#[tokio::test]
async fn lost_and_cancelled_refresh_retry_one_intent_and_one_server_session() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let wire = wire::Observer::start(&server.origin).await;
    let proxy = Proxy::start(&wire.origin).await;
    let mut control = connected(&server, &owner, &proxy.origin, "session-retry").await;
    let old = control.execution_snapshot().unwrap();
    let before = control.status().instance;
    proxy.fault(SESSION, Fault::Lost);
    assert!(control.refresh_authentication().await.is_err());
    old.ensure_current().unwrap();
    assert_eq!(control.status().instance, before);
    proxy.fault(SESSION, Fault::Hold);
    {
        // Even though the old session has >60s left, a pending explicit refresh
        // is retried by the normal host authenticate entry point.
        let retry = control.authenticate();
        tokio::pin!(retry);
        tokio::select! {
            result = &mut retry => panic!("session response was not held: {result:?}"),
            result = tokio::time::timeout(Duration::from_secs(2), proxy.committed()) => result.unwrap(),
        }
        old.ensure_current().unwrap();
    }
    proxy.release();
    old.ensure_current().unwrap();
    control.authenticate().await.unwrap();
    assert_eq!(control.status().instance, before);
    assert_eq!(proxy.count(SESSION), 4);
    let sessions = wire.sessions();
    assert_eq!(sessions.len(), 4);
    assert_ne!(sessions[0].intent, sessions[1].intent);
    for replay in &sessions[2..] {
        assert_eq!(replay.intent, sessions[1].intent);
        assert_eq!(
            replay.expires_at_ms, sessions[1].expires_at_ms,
            "an intent replay must retain its original Server expiry"
        );
        assert_eq!(
            replay.token_digest, sessions[1].token_digest,
            "an intent replay must not allocate another Server session/quota slot"
        );
    }
    control.sync_configuration().await.unwrap();
    assert_eq!(wire.last_query_token(), Some(sessions[1].token_digest));
    old.ensure_current().unwrap();
    control.shutdown().await.unwrap();
    assert!(old.ensure_current().is_err());
    proxy.close().await;
    wire.close().await;
    server.close().await;
}

#[tokio::test]
async fn authenticate_refreshes_before_expiry_without_extending_old_snapshot() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    proxy.fault(SESSION, Fault::ShortSession);
    let mut control = connected(&server, &owner, &proxy.origin, "session-proactive").await;
    let old = control.execution_snapshot().unwrap();
    let before = control.status().instance;
    let deadline = old.expires_at();
    control.authenticate().await.unwrap();
    assert_eq!(proxy.count(SESSION), 2);
    assert_eq!(control.status().instance, before);
    assert_eq!(old.expires_at(), deadline);
    old.ensure_current().unwrap();
    let fresh = control.execution_snapshot().unwrap();
    assert!(fresh.same_execution(&old));
    fresh.continuation_from(&old).unwrap();
    tokio::time::timeout(Duration::from_secs(4), old.invalidated())
        .await
        .unwrap();
    assert!(old.ensure_current().is_err());
    fresh.ensure_current().unwrap();
    assert!(
        fresh.continuation_from(&old).is_err(),
        "expired flows cannot borrow the fresh session's deadline"
    );
    assert_eq!(control.status().instance, before);
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn old_session_expiring_during_refresh_fences_instance_before_promoting_new_token() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    proxy.fault(SESSION, Fault::ShortSession);
    let mut control = connected(&server, &owner, &proxy.origin, "session-expired").await;
    let old = control.execution_snapshot().unwrap();
    let before = control.status();
    proxy.fault(SESSION, Fault::Hold);
    {
        let refresh = control.refresh_authentication();
        tokio::pin!(refresh);
        tokio::select! {
            result = &mut refresh => panic!("session response was not held: {result:?}"),
            result = tokio::time::timeout(Duration::from_secs(2), proxy.committed()) => result.unwrap(),
        }
        tokio::time::timeout(Duration::from_secs(4), old.invalidated())
            .await
            .unwrap();
        assert!(old.ensure_current().is_err());
        proxy.release();
        refresh.await.unwrap();
    }
    assert!(control.status().authenticated);
    assert!(control.status().instance.is_none());
    assert_eq!(control.status().cached, before.cached);
    assert!(control.execution_snapshot().is_err());
    let lease = control.acquire_instance(Default::default()).await.unwrap();
    assert_ne!(lease.instance_id, before.instance.unwrap().instance_id);
    control.sync_configuration().await.unwrap();
    let fresh = control.execution_snapshot().unwrap();
    assert!(!fresh.same_execution(&old));
    assert!(fresh.continuation_from(&old).is_err());
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn definitive_member_credential_rejection_fences_preexisting_execution() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = connected(&server, &owner, &proxy.origin, "session-deleted").await;
    let node = control.status().binding.unwrap().node_id;
    let old = control.execution_snapshot().unwrap();
    let current = server
        .client
        .get_network(
            &GetNetworkPath {
                network_id: owner.network,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    server
        .client
        .delete_node(
            &DeleteNodePath {
                network_id: owner.network,
                node_id: node,
            },
            &DeleteNodeRequest {
                request_id: request(),
                expected_revision: current.network.current_revision,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    assert!(control.refresh_authentication().await.is_err());
    assert!(!control.status().authenticated);
    assert!(control.status().instance.is_none());
    assert!(old.ensure_current().is_err());
    assert_eq!(
        proxy.count(SESSION),
        1,
        "rejected challenge must never be signed/submitted"
    );
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn foreign_challenge_and_session_bindings_fence_instead_of_preserving_old_authority() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = connected(&server, &owner, &proxy.origin, "session-binding").await;
    for (suffix, fault) in [
        (CHALLENGE, Fault::WrongController),
        (SESSION, Fault::WrongDnsNetwork),
    ] {
        let old = control.execution_snapshot().unwrap();
        let count = proxy.count(SESSION);
        proxy.fault(suffix, fault);
        assert!(matches!(
            control.refresh_authentication().await,
            Err(Error::Contract)
        ));
        assert!(old.ensure_current().is_err());
        assert!(!control.status().authenticated);
        assert!(control.status().instance.is_none());
        if suffix == CHALLENGE {
            assert_eq!(
                proxy.count(SESSION),
                count,
                "foreign Server challenge is not signed"
            );
        }
        let recovered = control.authenticate().await;
        assert!(
            recovered.is_ok(),
            "fresh proof after {suffix}: {recovered:?}; challenges={}, sessions={}",
            proxy.count(CHALLENGE),
            proxy.count(SESSION)
        );
        control.acquire_instance(Default::default()).await.unwrap();
        control.sync_configuration().await.unwrap();
    }
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn proven_expired_replacement_response_retires_only_its_intent_and_can_retry_fresh() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let wire = wire::Observer::start(&server.origin).await;
    let mut control = connected(&server, &owner, &wire.origin, "session-old-reply").await;
    let old = control.execution_snapshot().unwrap();
    let before = control.status().instance;
    wire.expire_next_session();
    assert!(matches!(
        control.refresh_authentication().await,
        Err(Error::Expired)
    ));
    old.ensure_current().unwrap();
    assert_eq!(control.status().instance, before);
    control.refresh_authentication().await.unwrap();
    let sessions = wire.sessions();
    assert_eq!(sessions.len(), 3);
    assert_ne!(sessions[1].intent, sessions[2].intent);
    assert_eq!(control.status().instance, before);
    old.ensure_current().unwrap();
    control.shutdown().await.unwrap();
    wire.close().await;
    server.close().await;
}

// Observe typed session identities ahead of the reusable delivery-fault proxy.
// Retain only public intent/expiry and SHA-256 token fingerprints, never tokens.
mod wire {
    use axum::{
        Router,
        body::{Body, to_bytes},
        extract::{Request, State},
        http::{Response, header},
    };
    use meshway_model::{Digest, RequestId};
    use meshway_protocol::{NodeSessionRequest, NodeSessionResponse};
    use sha2::{Digest as _, Sha256};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::{sync::oneshot, task::JoinHandle};

    #[derive(Clone)]
    pub struct SessionObservation {
        pub intent: RequestId,
        pub expires_at_ms: u64,
        pub token_digest: Digest,
    }
    #[derive(Clone)]
    struct Forward {
        origin: String,
        http: reqwest::Client,
        sessions: Arc<Mutex<Vec<SessionObservation>>>,
        query_token: Arc<Mutex<Option<Digest>>>,
        expire: Arc<AtomicBool>,
    }
    pub struct Observer {
        pub origin: String,
        state: Forward,
        stop: oneshot::Sender<()>,
        task: JoinHandle<()>,
    }
    impl Observer {
        pub async fn start(origin: &str) -> Self {
            let state = Forward {
                origin: origin.into(),
                http: reqwest::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(std::time::Duration::from_secs(10))
                    .build()
                    .unwrap(),
                sessions: Arc::default(),
                query_token: Arc::default(),
                expire: Arc::default(),
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let router = Router::new().fallback(forward).with_state(state.clone());
            let (stop, done) = oneshot::channel();
            let task = tokio::spawn(async move {
                axum::serve(listener, router)
                    .with_graceful_shutdown(async {
                        let _ = done.await;
                    })
                    .await
                    .unwrap();
            });
            Self {
                origin,
                state,
                stop,
                task,
            }
        }
        pub fn sessions(&self) -> Vec<SessionObservation> {
            self.state.sessions.lock().unwrap().clone()
        }
        pub fn last_query_token(&self) -> Option<Digest> {
            *self.state.query_token.lock().unwrap()
        }
        pub fn expire_next_session(&self) {
            self.state.expire.store(true, Ordering::Release);
        }
        pub async fn close(self) {
            self.stop.send(()).unwrap();
            self.task.await.unwrap();
        }
    }
    fn fingerprint(token: &str) -> Digest {
        Digest::from_bytes(Sha256::digest(token.as_bytes()).into())
    }
    async fn forward(State(state): State<Forward>, request: Request) -> Response<Body> {
        let (parts, body) = request.into_parts();
        let body = to_bytes(body, 16 * 1024 * 1024).await.unwrap();
        let intent = if parts.uri.path().ends_with(super::SESSION) {
            Some(
                serde_json::from_slice::<NodeSessionRequest>(&body)
                    .unwrap()
                    .intent
                    .request_id,
            )
        } else {
            None
        };
        let mut headers = parts.headers;
        if parts.uri.path().ends_with("/configuration/query") {
            let token = headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap()
                .strip_prefix("Bearer ")
                .unwrap();
            *state.query_token.lock().unwrap() = Some(fingerprint(token));
        }
        headers.remove(header::HOST);
        headers.remove(header::CONTENT_LENGTH);
        let response = state
            .http
            .request(parts.method, format!("{}{}", state.origin, parts.uri))
            .headers(headers)
            .body(body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let mut headers = response.headers().clone();
        let mut bytes = response.bytes().await.unwrap().to_vec();
        if let Some(intent) = intent
            && status.is_success()
        {
            let mut session: NodeSessionResponse = serde_json::from_slice(&bytes).unwrap();
            state.sessions.lock().unwrap().push(SessionObservation {
                intent,
                expires_at_ms: session.expires_at_ms,
                token_digest: fingerprint(session.token.expose()),
            });
            if state.expire.swap(false, Ordering::AcqRel) {
                // Conservative lifetime fault on a genuine bound response.
                session.expires_at_ms = u64::try_from(chrono::Utc::now().timestamp_millis())
                    .unwrap()
                    .saturating_sub(1);
                bytes = serde_json::to_vec(&session).unwrap();
            }
        }
        headers.remove(header::CONTENT_LENGTH);
        headers.remove(header::TRANSFER_ENCODING);
        let mut response = Response::builder()
            .status(status)
            .body(Body::from(bytes))
            .unwrap();
        *response.headers_mut() = headers;
        response
    }
}
