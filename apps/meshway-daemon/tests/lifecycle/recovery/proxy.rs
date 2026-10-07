//! Bounded HTTP delivery faults around a real Server. No grants are fabricated.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{Response, StatusCode, header},
};
use meshway_protocol::{ErrorResponse, NodeChallenge, NodeSessionResponse, Problem};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};
use tokio::sync::oneshot;

pub const HEALTHY: u8 = 0;
pub const GATEWAY: u8 = 1;
pub const HTML: u8 = 2;
pub const WRONG_CONTROLLER: u8 = 3;

#[derive(Default)]
pub struct Faults {
    pub mode: AtomicU8,
    pub invalidate_instance: AtomicBool,
    pub session_expiry: AtomicU64,
    pub challenges: AtomicU64,
    pub blocked_requests: AtomicU64,
}
#[derive(Clone)]
struct Forward {
    origin: Arc<Mutex<String>>,
    http: reqwest::Client,
    faults: Arc<Faults>,
}
pub struct Proxy {
    pub origin: String,
    pub faults: Arc<Faults>,
    backend: Arc<Mutex<String>>,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}
impl Proxy {
    pub async fn start(origin: &str) -> Self {
        let faults = Arc::new(Faults::default());
        let backend = Arc::new(Mutex::new(origin.to_owned()));
        let state = Forward {
            origin: backend.clone(),
            faults: faults.clone(),
            http: meshway_control_client::http_client_builder()
                .unwrap()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap(),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new().fallback(forward).with_state(state);
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(async {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        Self {
            origin,
            faults,
            backend,
            stop,
            task,
        }
    }
    pub fn mode(&self, mode: u8) {
        self.faults.mode.store(mode, Ordering::SeqCst);
    }
    pub fn backend(&self, origin: &str) {
        *self.backend.lock().unwrap() = origin.to_owned();
    }
    pub async fn close(self) {
        self.stop.send(()).unwrap();
        self.task.await.unwrap();
    }
}

async fn forward(State(state): State<Forward>, request: Request) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let path = parts.uri.path();
    let mode = state.faults.mode.load(Ordering::SeqCst);
    if path.ends_with("/auth/challenges") {
        state.faults.challenges.fetch_add(1, Ordering::SeqCst);
    }
    if mode == GATEWAY || mode == HTML {
        state.faults.blocked_requests.fetch_add(1, Ordering::SeqCst);
        return Response::builder()
            .status(if mode == GATEWAY { 521 } else { 200 })
            .header(header::CONTENT_TYPE, "text/html")
            .body(Body::from("<html>isolated gateway unavailable</html>"))
            .unwrap();
    }
    if path.contains("/configuration/")
        && state
            .faults
            .invalidate_instance
            .swap(false, Ordering::SeqCst)
    {
        // Simulate a received stale-instance rejection, not a valid response.
        let error = ErrorResponse {
            request_id: None,
            status: 409,
            problem: Problem::RequestConflict {
                params: serde_json::from_value(serde_json::json!({})).unwrap(),
                message: "stale instance".into(),
            },
        };
        return Response::builder()
            .status(409)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&error).unwrap()))
            .unwrap();
    }
    let mut headers = parts.headers;
    headers.remove(header::HOST);
    headers.remove(header::CONTENT_LENGTH);
    let bytes = to_bytes(body, 16 * 1024 * 1024).await.unwrap();
    let origin = state.origin.lock().unwrap().clone();
    let response = state
        .http
        .request(parts.method, format!("{}{}", origin, parts.uri))
        .headers(headers)
        .body(bytes)
        .send()
        .await
        .unwrap();
    let status = response.status();
    assert!(
        response
            .content_length()
            .is_none_or(|length| length <= 16 * 1024 * 1024)
    );
    let mut bytes = response.bytes().await.unwrap().to_vec();
    if status == StatusCode::OK && path.ends_with("/auth/session") {
        let session: NodeSessionResponse = serde_json::from_slice(&bytes).unwrap();
        // Only the expiry is retained; the actual bearer stays on its HTTP wire.
        state
            .faults
            .session_expiry
            .store(session.expires_at_ms, Ordering::SeqCst);
    }
    if status == StatusCode::OK && path.ends_with("/auth/challenges") && mode == WRONG_CONTROLLER {
        let mut challenge: NodeChallenge = serde_json::from_slice(&bytes).unwrap();
        challenge.controller_id = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        bytes = serde_json::to_vec(&challenge).unwrap();
    }
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(bytes))
        .unwrap()
}
