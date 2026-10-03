use futures_util::StreamExt;
use reqwest::{Method, Url, header};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware, Middleware};
use serde::{Serialize, de::DeserializeOwned};
use std::{fmt, io::Write};

/// Errors never retain raw HTTP bodies, URLs, headers or transport exceptions.
pub enum ClientError<Problem> {
    Configuration,
    Request,
    Network {
        status: Option<u16>,
    },
    BodyLimit {
        status: u16,
    },
    Decode {
        status: u16,
    },
    Http {
        status: u16,
        problem: Option<Problem>,
    },
}

impl<P> fmt::Debug for ClientError<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration => f.write_str("Configuration"),
            Self::Request => f.write_str("Request"),
            Self::Network { status } => f.debug_struct("Network").field("status", status).finish(),
            Self::BodyLimit { status } => {
                f.debug_struct("BodyLimit").field("status", status).finish()
            }
            Self::Decode { status } => f.debug_struct("Decode").field("status", status).finish(),
            Self::Http { status, .. } => f
                .debug_struct("Http")
                .field("status", status)
                .finish_non_exhaustive(),
        }
    }
}
impl<P> fmt::Display for ClientError<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}
impl<P> std::error::Error for ClientError<P> {}

/// Caller supplies the HTTP builder with protected sockets/resolver and timeout.
/// The SDK disables redirects before building that exact client. Middleware can
/// be attached with `with_middleware` without replacing the protected client.
/// Dropping an operation future cancels its pending send/read; no detached tasks.
#[derive(Clone)]
pub struct Client {
    base: Url,
    http: ClientWithMiddleware,
    limit: usize,
}

impl Client {
    pub fn new(
        base: &str,
        http: reqwest::ClientBuilder,
        max_body_bytes: usize,
    ) -> Result<Self, ClientError<std::convert::Infallible>> {
        let base = Url::parse(base).map_err(|_| ClientError::Configuration)?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
            || !(1..=16 * 1024 * 1024).contains(&max_body_bytes)
        {
            return Err(ClientError::Configuration);
        }
        let http = http
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| ClientError::Configuration)?;
        Ok(Self {
            base,
            http: ClientBuilder::new(http).build(),
            limit: max_body_bytes,
        })
    }

    pub fn with_middleware<M: Middleware>(mut self, middleware: M) -> Self {
        self.http = ClientBuilder::from_client(self.http)
            .with(middleware)
            .build();
        self
    }

    // Kept crate-private: only generated operations provide paths and wire types.
    pub(crate) async fn send<B: Serialize, P>(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<&B>,
    ) -> Result<Response, ClientError<P>> {
        if !path.starts_with('/')
            || path.starts_with("//")
            || path.split('/').skip(1).any(|part| {
                part.is_empty()
                    || !part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
        {
            return Err(ClientError::Configuration);
        }
        let url = self
            .base
            .join(path)
            .map_err(|_| ClientError::Configuration)?;
        let mut request = self
            .http
            .request(method, url)
            .header(header::ACCEPT, "application/json");
        if let Some(token) = token {
            if token.is_empty() || !token.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
                return Err(ClientError::Configuration);
            }
            let mut value = header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| ClientError::Configuration)?;
            value.set_sensitive(true);
            request = request.header(header::AUTHORIZATION, value);
        }
        if let Some(body) = body {
            let mut buffer = BoundedBuffer {
                bytes: Vec::new(),
                limit: self.limit,
            };
            serde_json::to_writer(&mut buffer, body).map_err(|_| ClientError::Request)?;
            request = request
                .header(header::CONTENT_TYPE, "application/json")
                .body(buffer.bytes);
        }
        let response = request
            .send()
            .await
            .map_err(|_| ClientError::Network { status: None })?;
        let status = response.status().as_u16();
        // Redirects are never followed by the underlying client. Reject their
        // response immediately too, even if its body never reaches EOF.
        if response.status().is_redirection() {
            return Err(ClientError::Http {
                status,
                problem: None,
            });
        }
        let json = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"));
        if status == 204 {
            return Ok(Response {
                status,
                bytes: Vec::new(),
                json: false,
            });
        }
        if !json {
            return Err(if response.status().is_success() {
                ClientError::Decode { status }
            } else {
                ClientError::Http {
                    status,
                    problem: None,
                }
            });
        }
        if response
            .content_length()
            .is_some_and(|size| size > self.limit as u64)
        {
            return Err(limit_error(status));
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| ClientError::Network {
                status: Some(status),
            })?;
            if chunk.len() > self.limit.saturating_sub(bytes.len()) {
                return Err(limit_error(status));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Response {
            status,
            bytes,
            json,
        })
    }
}

fn limit_error<P>(status: u16) -> ClientError<P> {
    if status >= 400 {
        ClientError::Http {
            status,
            problem: None,
        }
    } else {
        ClientError::BodyLimit { status }
    }
}

pub(crate) struct Response {
    pub status: u16,
    bytes: Vec<u8>,
    json: bool,
}
impl Response {
    pub(crate) fn json<T: DeserializeOwned, P>(self) -> Result<T, ClientError<P>> {
        if !self.json {
            return Err(ClientError::Decode {
                status: self.status,
            });
        }
        serde_json::from_slice(&self.bytes).map_err(|_| ClientError::Decode {
            status: self.status,
        })
    }
    pub(crate) fn empty<P>(self) -> Result<(), ClientError<P>> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(ClientError::Decode {
                status: self.status,
            })
        }
    }
    pub(crate) fn problem<P: DeserializeOwned>(self) -> ClientError<P> {
        let problem = if self.json {
            serde_json::from_slice(&self.bytes).ok()
        } else {
            None
        };
        ClientError::Http {
            status: self.status,
            problem,
        }
    }
    pub(crate) fn unexpected<P>(self) -> ClientError<P> {
        ClientError::Http {
            status: self.status,
            problem: None,
        }
    }
}

struct BoundedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("request body limit"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
        let http = reqwest::Client::builder().timeout(Duration::from_secs(2));
        (Client::new(&base, http, limit).unwrap(), server)
    }
    async fn fetch(client: &Client, path: &str) -> Result<Response, ClientError<Problem>> {
        client.send(Method::GET, path, None, None::<&()>).await
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
        let address: std::net::SocketAddr =
            source.strip_prefix("http://").unwrap().parse().unwrap();
        // This hostname cannot resolve without the caller's override. Even an
        // explicitly permissive redirect policy must be replaced by the SDK.
        let client = Client::new(
            &format!("http://protected.invalid:{}", address.port()),
            reqwest::Client::builder()
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
            let result =
                tokio::time::timeout(Duration::from_millis(250), fetch(&client, "/pending"))
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
}
