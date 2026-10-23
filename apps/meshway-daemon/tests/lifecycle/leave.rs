//! `logout` is a durable intent: a lost answer completes on the Server's
//! later identity rejection, an unanswered one is retried, also after a
//! restart, and nothing rejoins meanwhile.
use super::*;
use axum::{body::Body, extract::Request, http::StatusCode, response::Response};
use meshway_control_client::GetNodePath;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Forward,
    /// The Server handles the leave; its answer is lost on the way back.
    LoseLeave,
    /// No Server answer at all.
    Unavailable,
}

struct Proxy {
    origin: String,
    mode: Arc<Mutex<Mode>>,
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl Proxy {
    async fn start(target: &str) -> Self {
        let mode = Arc::new(Mutex::new(Mode::Forward));
        let http = meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy()
            .build()
            .unwrap();
        let app = axum::Router::new().fallback({
            let (mode, target) = (mode.clone(), target.to_owned());
            move |request: Request| {
                let (mode, target, http) = (*mode.lock().unwrap(), target.clone(), http.clone());
                async move {
                    let path = request.uri().path().to_owned();
                    if mode == Mode::Unavailable {
                        return status(StatusCode::SERVICE_UNAVAILABLE);
                    }
                    let (parts, body) = request.into_parts();
                    let body = axum::body::to_bytes(body, 16 * 1024 * 1024).await.unwrap();
                    let mut headers = parts.headers;
                    headers.remove(axum::http::header::HOST);
                    headers.remove(axum::http::header::CONTENT_LENGTH);
                    let response = http
                        .request(parts.method, format!("{target}{}", parts.uri))
                        .headers(headers)
                        .body(body)
                        .send()
                        .await
                        .unwrap();
                    if mode == Mode::LoseLeave && path.ends_with("/leave") {
                        return status(StatusCode::BAD_GATEWAY);
                    }
                    let mut builder = Response::builder().status(response.status());
                    for (name, value) in response.headers() {
                        if name != axum::http::header::CONTENT_LENGTH
                            && name != axum::http::header::TRANSFER_ENCODING
                        {
                            builder = builder.header(name, value);
                        }
                    }
                    builder
                        .body(Body::from(response.bytes().await.unwrap()))
                        .unwrap()
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        Self {
            origin,
            mode,
            stop,
            task,
        }
    }
    fn set(&self, mode: Mode) {
        *self.mode.lock().unwrap() = mode;
    }
    async fn close(self) {
        self.stop.send(()).unwrap();
        self.task.await.unwrap();
    }
}

fn status(code: StatusCode) -> Response {
    Response::builder()
        .status(code)
        .body(Body::empty())
        .unwrap()
}

async fn enrolled(
    server: &Server,
    user: &meshway_protocol::WireSecret,
    network: meshway_model::NetworkId,
    socket: &std::path::Path,
) -> meshway_protocol::ipc::Status {
    let initial = wait(socket, Phase::Down).await;
    let up = Command::Up {
        enrollment_key: None,
        expected_revision: initial.revision,
        reenroll: false,
    };
    call(socket, up, request_id()).await.unwrap();
    let pending = wait(socket, Phase::AwaitingApproval).await;
    server
        .client
        .approve_device_authorization(
            &ApproveDeviceAuthorizationPath {
                enrollment_request_id: pending.enrollment_request_id.unwrap(),
            },
            &ApproveEnrollmentRequest {
                network_id: network,
            },
            user.expose(),
        )
        .await
        .unwrap();
    wait(socket, Phase::ControlReady).await
}

async fn logout(socket: &std::path::Path, revision: meshway_model::Version) -> ResultBody {
    call(
        socket,
        Command::Logout {
            expected_revision: revision,
        },
        request_id(),
    )
    .await
    .unwrap()
    .result
}

async fn node_exists(
    server: &Server,
    user: &meshway_protocol::WireSecret,
    network: meshway_model::NetworkId,
    node: meshway_model::NodeId,
) -> bool {
    server
        .client
        .get_node(
            &GetNodePath {
                network_id: network,
                node_id: node,
            },
            user.expose(),
        )
        .await
        .is_ok()
}

fn left(status: &meshway_protocol::ipc::Status) -> bool {
    status.phase == Phase::Down && status.identity.is_none() && !status.desired_up
}

/// The first Status in phase `blocked`.
async fn blocked(socket: &std::path::Path) -> meshway_protocol::ipc::Status {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(status) = support::status(socket).await
            && status.phase == Phase::Blocked
        {
            return status;
        }
        assert!(tokio::time::Instant::now() < deadline, "never blocked");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// N7 (PROD-1): after a confirmed `logout` the Node moves to another
/// Server by restarting with its URL and `up`, without touching the
/// identity directory. A leave its Server has not confirmed keeps the
/// identity there: the other URL is `blocked · storage` with a reason
/// naming both Servers, and the original URL completes the leave.
#[tokio::test]
async fn a_left_node_moves_to_another_server_and_a_leaving_one_says_why_not() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let old = Server::start(root.join("old")).await;
    let (old_user, old_network) = old.owner().await;
    let new = Server::start(root.join("new")).await;
    let (new_user, new_network) = new.owner().await;

    let moved = root.join("moved");
    let mut daemon = Process::start(&moved, &old.origin);
    let ready = enrolled(&old, &old_user, old_network, &daemon.socket).await;
    assert!(matches!(
        logout(&daemon.socket, ready.revision).await,
        ResultBody::Status { .. }
    ));
    wait_matching(&daemon.socket, left).await;
    daemon.exit("-TERM").await;
    let daemon = Process::start(&moved, &new.origin);
    let ready = enrolled(&new, &new_user, new_network, &daemon.socket).await;
    assert_eq!(ready.identity.unwrap().network_id, new_network);
    assert_eq!(ready.server.trim_end_matches('/'), new.origin);
    drop(daemon);

    let proxy = Proxy::start(&old.origin).await;
    let leaving = root.join("leaving");
    let mut daemon = Process::start(&leaving, &proxy.origin);
    let ready = enrolled(&old, &old_user, old_network, &daemon.socket).await;
    proxy.set(Mode::Unavailable);
    logout(&daemon.socket, ready.revision).await;
    wait(&daemon.socket, Phase::Leaving).await;
    daemon.exit("-TERM").await;
    let mut daemon = Process::start(&leaving, &new.origin);
    let status = blocked(&daemon.socket).await;
    assert_eq!(
        status.last_error,
        Some(meshway_protocol::ipc::Problem::Storage)
    );
    let detail = status
        .last_error_detail
        .expect("a blocked Status names its cause");
    for part in [
        "is leaving",
        &meshway_control::server_origin(&proxy.origin).unwrap(),
        &meshway_control::server_origin(&new.origin).unwrap(),
    ] {
        assert!(detail.contains(part), "{detail}");
    }
    daemon.exit("-TERM").await;
    proxy.set(Mode::Forward);
    let daemon = Process::start(&leaving, &proxy.origin);
    wait_matching(&daemon.socket, left).await;
    drop(daemon);
    proxy.close().await;
    new.close().await;
    old.close().await;
}

#[tokio::test]
async fn logout_converges_after_a_lost_answer_and_survives_an_unreachable_server() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let server = Server::start(root.join("server")).await;
    let (user, network) = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;

    // A lost answer: the Server deleted the Node; the retried request's
    // identity rejection completes the leave without operator action.
    let lost = root.join("lost");
    let mut daemon = Process::start(&lost, &proxy.origin);
    let ready = enrolled(&server, &user, network, &daemon.socket).await;
    let node = ready.identity.as_ref().unwrap().node_id;
    proxy.set(Mode::LoseLeave);
    let answer = logout(&daemon.socket, ready.revision).await;
    assert!(
        matches!(
            answer,
            ResultBody::Error {
                code: ErrorCode::Leaving
            }
        ),
        "{answer:?}"
    );
    assert!(!node_exists(&server, &user, network, node).await);
    proxy.set(Mode::Forward);
    let gone = wait_matching(&daemon.socket, left).await;
    assert!(gone.saved.is_none());
    assert!(!lost.join("identity/netmap.json").exists());

    // The left directory is an ordinary unenrolled installation: a restart
    // stays down with no identity and resumes no leave.
    daemon.exit("-TERM").await;
    let daemon = Process::start(&lost, &proxy.origin);
    let restarted = wait_matching(&daemon.socket, left).await;
    assert_eq!(restarted.revision, gone.revision);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(left(&support::status(&daemon.socket).await.unwrap()));
    drop(daemon);

    // No Server answer: the intent stays, `up` cannot rejoin, and a restart
    // resumes the leave until the Server confirms it.
    let unanswered = root.join("unanswered");
    let mut daemon = Process::start(&unanswered, &proxy.origin);
    let ready = enrolled(&server, &user, network, &daemon.socket).await;
    let node = ready.identity.as_ref().unwrap().node_id;
    proxy.set(Mode::Unavailable);
    let answer = logout(&daemon.socket, ready.revision).await;
    assert!(
        matches!(
            answer,
            ResultBody::Error {
                code: ErrorCode::Leaving
            }
        ),
        "{answer:?}"
    );
    let leaving = wait(&daemon.socket, Phase::Leaving).await;
    assert!(leaving.identity.is_some() && node_exists(&server, &user, network, node).await);
    let refused = call(
        &daemon.socket,
        Command::Up {
            enrollment_key: None,
            expected_revision: leaving.revision,
            reenroll: false,
        },
        request_id(),
    )
    .await
    .unwrap()
    .result;
    assert!(
        matches!(
            refused,
            ResultBody::Error {
                code: ErrorCode::Leaving
            }
        ),
        "{refused:?}"
    );
    daemon.exit("-TERM").await;
    let daemon = Process::start(&unanswered, &proxy.origin);
    let resumed = wait(&daemon.socket, Phase::Leaving).await;
    assert_eq!(resumed.identity, leaving.identity);
    proxy.set(Mode::Forward);
    wait_matching(&daemon.socket, left).await;
    assert!(!node_exists(&server, &user, network, node).await);
    drop(daemon);
    proxy.close().await;
    server.close().await;
}
