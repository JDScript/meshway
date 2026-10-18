use super::*;
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Response as HttpResponse},
    routing::{any, get, post},
};
use serde::Deserialize;
use std::{
    convert::Infallible,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Value {
    value: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Problem {
    code: String,
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn start_server(router: Router) -> (String, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (base, Server(handle))
}
async fn server(router: Router, limit: usize) -> (Client, Server) {
    let (base, server) = start_server(router).await;
    let http = crate::http_client_builder()
        .unwrap()
        .timeout(Duration::from_secs(2));
    (Client::new(&base, http, limit).unwrap(), server)
}
async fn fetch(client: &Client, path: &str) -> Result<Response, ClientError<Problem>> {
    client.send(Method::GET, path, None, None::<&()>).await
}

struct PathChanged(tokio::sync::watch::Receiver<u64>);
impl RequestInterrupt for PathChanged {
    fn interrupted(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let mut path = self.0.clone();
        path.borrow_and_update();
        Box::pin(async move {
            let _ = path.changed().await;
        })
    }
}

#[tokio::test]
async fn transport_change_cancels_headers_and_body_but_not_the_next_request() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let headers = entered.clone();
    let body = entered.clone();
    let router = Router::new()
        .route(
            "/headers",
            get(move || {
                let entered = headers.clone();
                async move {
                    entered.notify_one();
                    std::future::pending::<HttpResponse<Body>>().await
                }
            }),
        )
        .route(
            "/body",
            get(move || {
                let entered = body.clone();
                async move {
                    let stream = futures_util::stream::once(async move {
                        entered.notify_one();
                        Ok::<_, Infallible>(bytes::Bytes::from_static(b"{"))
                    })
                    .chain(futures_util::stream::pending());
                    HttpResponse::builder()
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from_stream(stream))
                        .unwrap()
                }
            }),
        )
        .route(
            "/ready",
            get(|| async { json(200, r#"{"value":"ready"}"#) }),
        );
    let (client, mut server) = server(router, 4096).await;
    let (path, received) = tokio::sync::watch::channel(0);
    let client = client.with_request_interrupt(Some(Arc::new(PathChanged(received))));
    for (next, route) in [(1, "/headers"), (2, "/body")] {
        let sending = client.clone();
        let task = tokio::spawn(async move { fetch(&sending, route).await });
        entered.notified().await;
        path.send_replace(next);
        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(500), task)
                .await
                .unwrap()
                .unwrap(),
            Err(ClientError::Network { .. })
        ));
        assert!(fetch(&client, "/ready").await.is_ok());
    }
    server.0.abort();
    let _ = (&mut server.0).await;
}
fn json(status: u16, body: &'static str) -> HttpResponse<Body> {
    HttpResponse::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn typed_decoding_and_debug_never_retain_invalid_bodies() {
    let router = Router::new()
        .route("/value", get(|| async { json(200, r#"{"value":"ok"}"#) }))
        .route(
            "/extra",
            get(|| async { json(200, r#"{"value":"ok","secret":"must-not-escape"}"#) }),
        )
        .route(
            "/problem",
            get(|| async { json(409, r#"{"code":"must-not-escape"}"#) }),
        )
        .route("/invalid", get(|| async { json(409, "must-not-escape") }))
        .route(
            "/empty",
            get(|| async {
                HttpResponse::builder()
                    .status(204)
                    .body(Body::empty())
                    .unwrap()
            }),
        );
    let (client, _server) = server(router, 1024).await;
    let value: Value = fetch(&client, "/value")
        .await
        .unwrap()
        .json::<_, Problem>()
        .unwrap();
    assert_eq!(value.value, "ok");
    let result = fetch(&client, "/extra")
        .await
        .unwrap()
        .json::<Value, Problem>();
    assert!(matches!(result, Err(ClientError::Decode { status: 200 })));
    let error: ClientError<Problem> = fetch(&client, "/problem").await.unwrap().problem();
    assert!(!format!("{error:?} {error}").contains("must-not-escape"));
    match error {
        ClientError::Http {
            status: 409,
            problem: Some(problem),
        } => assert_eq!(problem.code, "must-not-escape"),
        _ => panic!("expected validated problem"),
    }
    let error: ClientError<Problem> = fetch(&client, "/invalid").await.unwrap().problem();
    assert!(matches!(
        error,
        ClientError::Http {
            status: 409,
            problem: None
        }
    ));
    fetch(&client, "/empty")
        .await
        .unwrap()
        .empty::<Problem>()
        .unwrap();
    let unexpected = fetch(&client, "/value")
        .await
        .unwrap()
        .unexpected::<Problem>();
    assert!(matches!(
        unexpected,
        ClientError::Http {
            status: 200,
            problem: None
        }
    ));
}

#[tokio::test]
async fn budgets_apply_to_chunked_responses_and_serialized_requests() {
    let router = Router::new()
        .route(
            "/large",
            get(|| async {
                HttpResponse::builder()
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from_stream(futures_util::stream::iter([
                        Ok::<_, Infallible>("1234"),
                        Ok("5678"),
                    ])))
                    .unwrap()
            }),
        )
        .route(
            "/error",
            get(|| async { json(409, "secret response too long") }),
        );
    let (client, _server) = server(router, 5).await;
    assert!(matches!(
        fetch(&client, "/large").await,
        Err(ClientError::BodyLimit { status: 200 })
    ));
    assert!(matches!(
        fetch(&client, "/error").await,
        Err(ClientError::Http {
            status: 409,
            problem: None
        })
    ));
    let result = client
        .send::<_, Problem>(
            Method::POST,
            "/large",
            None,
            Some(&"request-secret-too-long"),
        )
        .await;
    assert!(matches!(result, Err(ClientError::Request)));
}

#[tokio::test]
async fn authorization_and_invalid_paths_remain_controlled() {
    let router = Router::new().route(
        "/headers",
        get(|headers: HeaderMap| async move {
            assert_eq!(headers[header::AUTHORIZATION], "Bearer private-token");
            json(200, r#"{"value":"ok"}"#)
        }),
    );
    let (client, _server) = server(router, 1024).await;
    client
        .send::<(), Problem>(Method::GET, "/headers", Some("private-token"), None)
        .await
        .unwrap();
    for path in [
        "//other-host/path",
        "/a/../b",
        "/query?token=secret",
        "/encoded%2fpath",
    ] {
        assert!(matches!(
            fetch(&client, path).await,
            Err(ClientError::Configuration)
        ));
    }
    assert!(matches!(
        client
            .send::<(), Problem>(Method::GET, "/headers", Some("token\r\nsecret"), None)
            .await,
        Err(ClientError::Configuration)
    ));
}

#[test]
fn parameter_binding_rejects_ambiguous_paths_and_preserves_origin() {
    let client = Client::new(
        "http://[::1]:1234",
        crate::http_client_builder().unwrap(),
        1024,
    )
    .unwrap();
    let url = client
        .parameter_url::<Problem>(
            "/fixture/{value}",
            &[("value", "https://evil.test/../?token=x#fragment".into())],
            &[
                ("z", None),
                ("cursor", Some("%2e&token=query-secret#x".into())),
            ],
        )
        .unwrap();
    assert_eq!(
        url.as_str(),
        "http://[::1]:1234/fixture/https%3A%2F%2Fevil.test%2F..%2F%3Ftoken%3Dx%23fragment?cursor=%252e%26token%3Dquery-secret%23x"
    );
    assert_eq!(url.origin(), client.base.origin());
    assert!(url.fragment().is_none());
    for (template, path) in [
        ("/fixture/{value}", vec![]),
        (
            "/fixture/{value}",
            vec![("value", "x".into()), ("value", "y".into())],
        ),
        (
            "/fixture/{value}",
            vec![("value", "x".into()), ("extra", "y".into())],
        ),
        ("/fixture/{value}/{value}", vec![("value", "x".into())]),
        ("/fixture/{value}", vec![("value", ".".into())]),
        ("/fixture/{value}", vec![("value", "..".into())]),
        ("/fixture/{value}", vec![("value", "".into())]),
        ("//evil/fixture", vec![]),
        ("/fixture?token=x", vec![]),
        ("/fixture#secret", vec![]),
        ("/fixture/%2e", vec![]),
        ("/fixture/..", vec![]),
    ] {
        assert!(
            client
                .parameter_url::<Problem>(template, &path, &[])
                .is_err()
        );
    }
    assert!(
        client
            .parameter_url::<Problem>("/fixture", &[], &[("cursor", None), ("cursor", None)])
            .is_err()
    );
    assert!(
        client
            .parameter_url::<Problem>("/fixture", &[], &[("cursor&token", Some("x".into()))])
            .is_err()
    );
}

fn marker_middleware<'a>(
    mut request: reqwest::Request,
    extensions: &'a mut axum::http::Extensions,
    next: reqwest_middleware::Next<'a>,
) -> futures_util::future::BoxFuture<'a, reqwest_middleware::Result<reqwest::Response>> {
    request.headers_mut().insert(
        "x-caller-middleware",
        header::HeaderValue::from_static("active"),
    );
    next.run(request, extensions)
}

#[tokio::test]
async fn sdk_blocks_307_and_308_before_forwarding_passwords_and_preserves_caller_stack() {
    let target_requests = Arc::new(Mutex::new(Vec::new()));
    let received = target_requests.clone();
    let (target, _target_server) = start_server(Router::new().route(
        "/sink",
        any(move |method: Method, body: String| {
            let received = received.clone();
            async move {
                received.lock().unwrap().push((method, body));
                json(200, r#"{"value":"redirected"}"#)
            }
        }),
    ))
    .await;
    let source_requests = Arc::new(Mutex::new(Vec::new()));
    let source_seen = source_requests.clone();
    let (source, _source_server) = start_server(Router::new().route(
        "/redirect/{status}",
        post(
            move |axum::extract::Path(status): axum::extract::Path<u16>,
                  headers: HeaderMap,
                  body: String| {
                let target = target.clone();
                let source_seen = source_seen.clone();
                async move {
                    assert_eq!(headers["x-caller-middleware"], "active");
                    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
                    source_seen.lock().unwrap().push(body);
                    HttpResponse::builder()
                        .status(status)
                        .header(header::LOCATION, format!("{target}/sink"))
                        .body(Body::empty())
                        .unwrap()
                }
            },
        ),
    ))
    .await;
    let address: std::net::SocketAddr = source.strip_prefix("http://").unwrap().parse().unwrap();
    // This hostname cannot resolve without the caller's override. Even an
    // explicitly permissive redirect policy must be replaced by the SDK.
    let client = Client::new(
        &format!("http://protected.invalid:{}", address.port()),
        crate::http_client_builder()
            .unwrap()
            .no_proxy()
            .resolve("protected.invalid", address)
            .redirect(reqwest::redirect::Policy::limited(10))
            .timeout(Duration::from_secs(2)),
        1024,
    )
    .unwrap()
    .with_middleware(marker_middleware);
    #[derive(Serialize)]
    struct Input<'a> {
        password: &'a str,
    }
    for status in [307, 308] {
        let result = client
            .send::<_, Problem>(
                Method::POST,
                &format!("/redirect/{status}"),
                None,
                Some(&Input {
                    password: "private-password",
                }),
            )
            .await;
        assert!(
            matches!(result, Err(ClientError::Http { status: actual, problem: None }) if actual == status)
        );
    }
    let sources = source_requests.lock().unwrap();
    assert_eq!(sources.len(), 2);
    assert!(
        sources
            .iter()
            .all(|body| body == r#"{"password":"private-password"}"#)
    );
    assert!(
        target_requests.lock().unwrap().is_empty(),
        "redirect target must receive neither POST nor body"
    );
}

struct PendingBody {
    dropped: Arc<AtomicBool>,
}
impl futures_util::Stream for PendingBody {
    type Item = Result<&'static str, Infallible>;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Pending
    }
}
impl Drop for PendingBody {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn wrong_media_rejects_headers_and_releases_never_ending_bodies() {
    for status in [200, 409] {
        let dropped = Arc::new(AtomicBool::new(false));
        let observed = dropped.clone();
        let router = Router::new().route(
            "/pending",
            get(move || {
                let dropped = observed.clone();
                async move {
                    HttpResponse::builder()
                        .status(status)
                        .header(header::CONTENT_TYPE, "text/event-stream")
                        .body(Body::from_stream(PendingBody { dropped }))
                        .unwrap()
                }
            }),
        );
        let (client, _server) = server(router, 1024).await;
        // The caller timeout is two seconds. Headers must reject promptly
        // rather than waiting for EOF or that timeout.
        let result = tokio::time::timeout(Duration::from_millis(250), fetch(&client, "/pending"))
            .await
            .expect("wrong media must reject at headers");
        match status {
            200 => assert!(matches!(result, Err(ClientError::Decode { status: 200 }))),
            409 => assert!(matches!(
                result,
                Err(ClientError::Http {
                    status: 409,
                    problem: None
                })
            )),
            _ => unreachable!(),
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while !dropped.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("header rejection must release the response stream");
    }
}

#[tokio::test]
async fn dropping_operation_cancels_pending_body_without_a_detached_task() {
    let dropped = Arc::new(AtomicBool::new(false));
    let observed = dropped.clone();
    let router = Router::new().route(
        "/pending",
        get(move || {
            let dropped = observed.clone();
            async move {
                HttpResponse::builder()
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from_stream(PendingBody { dropped }))
                    .unwrap()
            }
        }),
    );
    let (client, _server) = server(router, 1024).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), fetch(&client, "/pending"))
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("dropping the SDK future must release the pending response");
}
