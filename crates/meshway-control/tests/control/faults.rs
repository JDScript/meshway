//! Forward real Server operations, then perturb delivery at the HTTP boundary.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{Response, StatusCode, header},
};
use std::sync::{Arc, Mutex};
use tokio::{
    sync::{Notify, oneshot},
    task::JoinHandle,
};

#[derive(Clone, Copy)]
pub enum Fault {
    RejectBefore,
    Lost,
    Hold,
    WrongController,
    WrongInstance,
    WrongExitRoleInstance,
    WrongDnsNetwork,
    WrongDnsSource,
    WrongDnsName,
    MissingDnsRecord,
    ShortSession,
    ShortInstance,
    /// The enrollment response pins the fixture's netmap signing key.
    ServerKey,
    UntypedConflict,
    WrongStateScopeStatus,
    StateScopeChanged,
    ReceiptConflict(&'static str, u16),
}
#[derive(Clone)]
struct Forward {
    origin: String,
    http: reqwest::Client,
    fault: Arc<Mutex<Option<(String, Fault)>>>,
    paths: Arc<Mutex<Vec<String>>>,
    committed: Arc<Notify>,
    release: Arc<Notify>,
}
pub struct Proxy {
    pub origin: String,
    state: Forward,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}
impl Proxy {
    pub async fn start(origin: &str) -> Self {
        let state = Forward {
            origin: origin.into(),
            http: meshway_control_client::http_client_builder()
                .unwrap()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            fault: Arc::default(),
            paths: Arc::default(),
            committed: Arc::default(),
            release: Arc::default(),
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
    pub fn fault(&self, suffix: &str, fault: Fault) {
        *self.state.fault.lock().unwrap() = Some((suffix.into(), fault));
    }
    pub async fn committed(&self) {
        self.state.committed.notified().await;
    }
    pub fn release(&self) {
        self.state.release.notify_one();
    }
    pub fn count(&self, suffix: &str) -> usize {
        self.state
            .paths
            .lock()
            .unwrap()
            .iter()
            .filter(|path| path.ends_with(suffix))
            .count()
    }
    pub async fn close(self) {
        self.release();
        self.stop.send(()).unwrap();
        self.task.await.unwrap();
    }
}
async fn forward(State(state): State<Forward>, request: Request) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let path = parts.uri.path().to_owned();
    state.paths.lock().unwrap().push(path.clone());
    let fault = {
        let mut pending = state.fault.lock().unwrap();
        if pending
            .as_ref()
            .is_some_and(|(suffix, _)| path.ends_with(suffix))
        {
            pending.take().map(|(_, fault)| fault)
        } else {
            None
        }
    };
    let mut headers = parts.headers;
    if let Some(Fault::ReceiptConflict(code, body_status)) = fault {
        let body = serde_json::json!({
            "requestId": super::support::request(), "status": body_status,
            "problem": { "code": code, "params": {}, "message": "sensitive receipt body" }
        });
        return Response::builder()
            .status(StatusCode::CONFLICT)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
    }
    if matches!(fault, Some(Fault::RejectBefore)) {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::empty())
            .unwrap();
    }
    headers.remove(header::HOST);
    headers.remove(header::CONTENT_LENGTH);
    let body = to_bytes(body, 16 * 1024 * 1024).await.unwrap();
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
    let bytes = response.bytes().await.unwrap();
    if let Some(
        fault @ (Fault::UntypedConflict | Fault::WrongStateScopeStatus | Fault::StateScopeChanged),
    ) = fault
    {
        let body = if matches!(fault, Fault::UntypedConflict) {
            b"{}".to_vec()
        } else {
            serde_json::to_vec(&meshway_protocol::ErrorResponse {
                request_id: Some(super::support::request()),
                status: if matches!(fault, Fault::WrongStateScopeStatus) {
                    400
                } else {
                    409
                },
                problem: meshway_protocol::Problem::StateScopeChanged {
                    params: meshway_protocol::EmptyParams {},
                    message: "State scope changed.".into(),
                },
            })
            .unwrap()
        };
        return Response::builder()
            .status(StatusCode::CONFLICT)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();
    }
    let bytes = if status.is_success() {
        match fault {
            Some(Fault::Lost) => {
                return Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Body::empty())
                    .unwrap();
            }
            Some(Fault::Hold) => {
                state.committed.notify_one();
                state.release.notified().await;
                bytes.to_vec()
            }
            Some(fault) => {
                let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let id = serde_json::to_value(super::support::request()).unwrap();
                match fault {
                    Fault::WrongController => value["controllerId"] = id,
                    Fault::WrongInstance => value["lease"]["instanceId"] = id,
                    Fault::WrongExitRoleInstance => {
                        let mut response: meshway_protocol::NodeConfigurationResponse =
                            serde_json::from_value(value).unwrap();
                        let full = response.full.as_mut().expect("initial full target");
                        full.configuration
                            .exit_role
                            .as_mut()
                            .expect("exit role")
                            .instance_id = "00000000-0000-4000-8000-000000000001".parse().unwrap();
                        full.digest = meshway_configuration::digest(&full.configuration).unwrap();
                        response.lease.target.as_mut().unwrap().digest = full.digest;
                        value = serde_json::to_value(response).unwrap();
                    }
                    Fault::WrongDnsNetwork => value["networkId"] = id,
                    Fault::WrongDnsSource => value["sourceNodeId"] = id,
                    Fault::WrongDnsName => value["record"]["name"] = "other.mesh.example".into(),
                    Fault::MissingDnsRecord => {
                        value.as_object_mut().unwrap().remove("record");
                    }
                    Fault::ShortSession => {
                        value["expiresAt"] = (chrono::Utc::now() + chrono::Duration::seconds(2))
                            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                            .into()
                    }
                    Fault::ShortInstance => {
                        value["expiresAt"] = (chrono::Utc::now() + chrono::Duration::seconds(2))
                            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                            .into();
                        value["executionExpiresAt"] = serde_json::Value::Null;
                    }
                    Fault::ServerKey => {
                        value["serverSigningKey"] =
                            serde_json::to_value(super::netmap::support::server_key().1).unwrap();
                    }
                    Fault::Lost | Fault::Hold => unreachable!("delivery fault handled above"),
                    Fault::RejectBefore => unreachable!("rejected before forwarding"),
                    Fault::ReceiptConflict(..) => unreachable!("rejected before forwarding"),
                    Fault::UntypedConflict
                    | Fault::WrongStateScopeStatus
                    | Fault::StateScopeChanged => unreachable!("conflict fault handled above"),
                }
                serde_json::to_vec(&value).unwrap()
            }
            None => bytes.to_vec(),
        }
    } else {
        bytes.to_vec()
    };
    headers.remove(header::CONTENT_LENGTH);
    headers.remove(header::TRANSFER_ENCODING);
    let mut response = Response::builder()
        .status(status)
        .body(Body::from(bytes))
        .unwrap();
    *response.headers_mut() = headers;
    response
}
