//! Key-pinning enrollment proxy in front of a real Server.
use crate::sync::server_key;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{Response, StatusCode, header},
};
use std::sync::{Arc, Mutex};
use tokio::{sync::oneshot, task::JoinHandle};

const BODY_LIMIT: usize = 64 * 1024 * 1024;

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
