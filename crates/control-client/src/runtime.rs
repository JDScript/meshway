use futures_util::StreamExt;
use reqwest::{Method, Url, header};
use reqwest_middleware::ClientWithMiddleware;
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

/// Caller supplies the HTTP client, including protected sockets/middleware.
/// Configure redirects off and a timeout on that client. Dropping an operation
/// future cancels its pending send/read; there are no detached request tasks.
#[derive(Clone)]
pub struct Client {
    base: Url,
    http: ClientWithMiddleware,
    limit: usize,
}

impl Client {
    pub fn new(
        base: &str,
        http: ClientWithMiddleware,
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
        Ok(Self {
            base,
            http,
            limit: max_body_bytes,
        })
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
            .request(method, url.clone())
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
        if response.url() != &url {
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
        routing::get,
    };
    use serde::Deserialize;
    use std::{
        convert::Infallible,
        sync::{
            Arc,
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
    async fn server(router: Router, limit: usize) -> (Client, Server) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        (
            Client::new(
                &base,
                reqwest_middleware::ClientBuilder::new(http).build(),
                limit,
            )
            .unwrap(),
            Server(handle),
        )
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
