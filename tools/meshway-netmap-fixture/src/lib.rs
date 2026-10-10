//! Test-only netmap authority for in-process fixtures.
//!
//! Fixtures that compile their own netmaps cannot obtain the real Server's
//! sealed signing key. They enroll their production `Control` through
//! [`PinningProxy`], which forwards every request to the real Server and only
//! replaces the `serverSigningKey` pinned by the enrollment response with the
//! fixed [`server_key`]. Scripted sync replies are then signed with that key
//! ([`signed`]), so the Control verifies them exactly as it verifies the real
//! Server. Nothing here is reachable from production code.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{Response, StatusCode, header},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_configuration::netmap::CompiledNetmap;
use meshway_control::{FactsRejected, PeerFactsSink, SyncFuture, SyncTransport};
use meshway_control_client::ClientError;
use meshway_model::{
    KeyAlgorithm, NetmapLease, NetmapRef, NetworkId, NodeId, NodePublicKey, Version,
};
use meshway_protocol::{
    ErrorResponse, NetmapSignature, NetmapSigner, PeerFactsDelta, ServerSigningKey, SyncNetmap,
    SyncRequest, SyncResponse, netmap_lease_transcript,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::{sync::oneshot, task::JoinHandle};

mod configuration;
pub use configuration::single_node;

const BODY_LIMIT: usize = 64 * 1024 * 1024;

/// Register a fixture user while respecting the Server's process-wide KDF
/// capacity. Only a busy response is retried, within a bounded setup budget.
pub async fn register(
    client: &meshway_control_client::Client,
    request: &meshway_protocol::RegisterRequest,
) -> Result<meshway_protocol::SessionIssuedResponse, ClientError<ErrorResponse>> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match client.register(request).await {
            Err(ClientError::Http {
                status: 503,
                problem: Some(ref body),
            }) if matches!(body.problem, meshway_protocol::Problem::Busy { .. })
                && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            result => return result,
        }
    }
}

/// The fixed fixture signing key and its public binding.
pub fn server_key() -> (SigningKey, ServerSigningKey) {
    let secret = SigningKey::from_bytes(&[42; 32]);
    let public = ServerSigningKey {
        key_id: "00000000-0000-4009-8000-000000000001"
            .parse()
            .expect("fixture key ID"),
        key_generation: Version::MIN,
        public_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: secret.verifying_key().to_bytes().into(),
        },
    };
    (secret, public)
}

/// Signature of `lease` for one Node by an explicit key.
pub fn sign_with(
    secret: &SigningKey,
    key: &ServerSigningKey,
    network: NetworkId,
    node: NodeId,
    lease: &NetmapLease,
) -> NetmapSignature {
    let message = netmap_lease_transcript(
        network,
        node,
        lease,
        NetmapSigner::Server,
        key.key_id,
        key.key_generation,
    );
    NetmapSignature {
        key_id: key.key_id,
        key_generation: key.key_generation,
        signature: URL_SAFE_NO_PAD.encode(secret.sign(&message).to_bytes()),
    }
}

/// Signature of `lease` for one Node by the fixture key.
pub fn sign(network: NetworkId, node: NodeId, lease: &NetmapLease) -> NetmapSignature {
    let (secret, key) = server_key();
    sign_with(&secret, &key, network, node, lease)
}

/// Current wall time in Unix milliseconds.
pub fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).expect("time after 1970")
}

/// A lease for `netmap` starting now.
pub fn lease(netmap: NetmapRef, validity_ms: u64) -> NetmapLease {
    let now = now_ms();
    NetmapLease::new(netmap, now, now + validity_ms).expect("valid fixture lease")
}

/// A fixture-signed response without peer facts.
pub fn signed(
    network: NetworkId,
    node: NodeId,
    lease: NetmapLease,
    netmap: SyncNetmap,
) -> SyncResponse {
    SyncResponse {
        signature: sign(network, node, &lease),
        lease,
        netmap,
        peer_facts: None,
    }
}

/// A fixture-signed full (`full`) or unchanged reply for `netmap`'s Node.
pub fn reply(netmap: &CompiledNetmap, validity_ms: u64, full: bool) -> SyncResponse {
    signed(
        netmap.netmap.network_id,
        netmap.netmap.node_id,
        lease(netmap.reference(), validity_ms),
        if full {
            SyncNetmap::Full {
                netmap: Box::new(netmap.clone()),
            }
        } else {
            SyncNetmap::Unchanged
        },
    )
}

type Reply = Result<SyncResponse, ClientError<ErrorResponse>>;

/// Sync replies in order; records each request. An empty script behaves like
/// an unreachable Server.
#[derive(Default)]
pub struct Script {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<SyncRequest>>,
}

impl Script {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    pub fn push(&self, reply: Reply) {
        self.replies.lock().expect("script lock").push_back(reply);
    }

    pub fn requests(&self) -> Vec<SyncRequest> {
        self.requests.lock().expect("script lock").clone()
    }

    pub fn calls(&self) -> usize {
        self.requests.lock().expect("script lock").len()
    }
}

impl SyncTransport for Script {
    fn sync<'a>(&'a self, request: &'a SyncRequest, token: &'a str) -> SyncFuture<'a> {
        assert!(!token.is_empty(), "sync carries the Node session");
        self.requests
            .lock()
            .expect("script lock")
            .push(request.clone());
        let reply = self.replies.lock().expect("script lock").pop_front();
        Box::pin(async move { reply.unwrap_or(Err(ClientError::Network { status: None })) })
    }
}

/// A peer-fact sink that records accepted deltas, or refuses all of them.
#[derive(Default)]
pub struct Facts {
    pub applied: Vec<(NetmapRef, PeerFactsDelta)>,
    pub reject: bool,
}

impl PeerFactsSink for Facts {
    fn apply(&mut self, netmap: NetmapRef, facts: PeerFactsDelta) -> Result<(), FactsRejected> {
        if self.reject {
            return Err(FactsRejected);
        }
        self.applied.push((netmap, facts));
        Ok(())
    }
}

#[derive(Clone)]
struct Forward {
    origin: String,
    http: reqwest::Client,
    pin: Arc<Mutex<Option<serde_json::Value>>>,
}

/// Forwards to a real Server and pins [`server_key`] in enrollment responses.
pub struct PinningProxy {
    pub origin: String,
    state: Forward,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl PinningProxy {
    pub async fn start(server: &str) -> Self {
        let state = Forward {
            origin: server.into(),
            http: meshway_control_client::http_client_builder()
                .expect("TLS configuration")
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("proxy client"),
            pin: Arc::new(Mutex::new(Some(
                serde_json::to_value(server_key().1).expect("key JSON"),
            ))),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("proxy listener");
        let origin = format!("http://{}", listener.local_addr().expect("proxy address"));
        let router = Router::new().fallback(forward).with_state(state.clone());
        let (stop, done) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = done.await;
                })
                .await
                .expect("proxy serve");
        });
        Self {
            origin,
            state,
            stop,
            task,
        }
    }

    /// Stop rewriting: later enrollments pin the real Server key.
    pub fn pass_through(&self) {
        *self.state.pin.lock().expect("pin lock") = None;
    }

    pub async fn close(self) {
        let _ = self.stop.send(());
        let _ = self.task.await;
    }
}

async fn forward(State(state): State<Forward>, request: Request) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let mut headers = parts.headers;
    headers.remove(header::HOST);
    headers.remove(header::CONTENT_LENGTH);
    let Ok(body) = to_bytes(body, BODY_LIMIT).await else {
        return status(StatusCode::PAYLOAD_TOO_LARGE);
    };
    let Ok(response) = state
        .http
        .request(parts.method, format!("{}{}", state.origin, parts.uri))
        .headers(headers)
        .body(body)
        .send()
        .await
    else {
        return status(StatusCode::BAD_GATEWAY);
    };
    let code = response.status();
    let mut headers = response.headers().clone();
    let Ok(bytes) = response.bytes().await else {
        return status(StatusCode::BAD_GATEWAY);
    };
    let pin = state.pin.lock().expect("pin lock").clone();
    let bytes = match pin {
        Some(pin) if code.is_success() && contains(&bytes, b"\"serverSigningKey\"") => {
            match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Ok(mut value) if value.get("serverSigningKey").is_some() => {
                    value["serverSigningKey"] = pin;
                    serde_json::to_vec(&value).expect("JSON")
                }
                _ => bytes.to_vec(),
            }
        }
        _ => bytes.to_vec(),
    };
    headers.remove(header::CONTENT_LENGTH);
    headers.remove(header::TRANSFER_ENCODING);
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = code;
    *response.headers_mut() = headers;
    response
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn status(code: StatusCode) -> Response<Body> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = code;
    response
}
