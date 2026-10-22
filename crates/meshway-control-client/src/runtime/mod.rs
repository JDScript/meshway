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

/// A transport failure keeps only its class. The debug event names that
/// class and the innermost I/O error kind, never the URL, headers or body.
fn network<P>(error: &(dyn std::error::Error + 'static), status: Option<u16>) -> ClientError<P> {
    let (stage, io, os_error, tls) = transport_failure(error);
    tracing::debug!(stage, io = ?io, os_error, tls, status, "Control HTTP transport failed");
    ClientError::Network { status }
}

fn transport_failure(
    error: &(dyn std::error::Error + 'static),
) -> (
    &'static str,
    Option<std::io::ErrorKind>,
    Option<i32>,
    Option<String>,
) {
    let mut stage = "other";
    let mut io = None;
    let mut os_error = None;
    let mut tls = None;
    let mut current = Some(error);
    while let Some(error) = current {
        // The transparent wrapper's source() skips reqwest itself, which owns
        // the timeout/connect classification (and the private request URL).
        if let Some(reqwest_middleware::Error::Reqwest(inner)) =
            error.downcast_ref::<reqwest_middleware::Error>()
        {
            current = Some(inner);
            continue;
        }
        if let Some(error) = error.downcast_ref::<reqwest::Error>() {
            stage = if error.is_timeout() {
                "timeout"
            } else if error.is_connect() {
                "connect"
            } else if error.is_body() {
                "body"
            } else if error.is_request() {
                "request"
            } else {
                stage
            };
        }
        if let Some(error) = error.downcast_ref::<std::io::Error>() {
            io = Some(error.kind());
            os_error = error.raw_os_error();
            if let Some(inner) = error.get_ref() {
                current = Some(inner);
                continue;
            }
        }
        if let Some(error) = error.downcast_ref::<rustls::Error>() {
            // Only protocol enums/numbers: General, Other and certificate
            // verifier errors can hold arbitrary text and are never formatted.
            tls = Some(match error {
                rustls::Error::InvalidMessage(value) => format!("invalid_message:{value:?}"),
                rustls::Error::PeerMisbehaved(value) => format!("peer_misbehaved:{value:?}"),
                rustls::Error::PeerIncompatible(_) => "peer_incompatible".into(),
                rustls::Error::AlertReceived(value) => format!("alert:{value:?}"),
                rustls::Error::DecryptError => "decrypt".into(),
                rustls::Error::EncryptError => "encrypt".into(),
                rustls::Error::InvalidCertificate(_) => "invalid_certificate".into(),
                rustls::Error::PeerSentOversizedRecord => "oversized_record".into(),
                rustls::Error::NoApplicationProtocol => "no_application_protocol".into(),
                _ => "other".into(),
            });
        }
        current = error.source();
    }
    (stage, io, os_error, tls)
}

/// Start a caller-owned HTTP builder with Meshway's explicit ring provider and
/// platform certificate verification. Socket protection, proxy policy and
/// deadlines remain the caller's responsibility. For managed certificate trust,
/// supply `meshway_transport_stream::tls::Client::configuration()` through
/// `reqwest::ClientBuilder::tls_backend_preconfigured` instead.
pub fn http_client_builder() -> Result<reqwest::ClientBuilder, ClientError<std::convert::Infallible>>
{
    let config = meshway_transport_stream::tls::platform_client_config()
        .map_err(|_| ClientError::Configuration)?;
    Ok(reqwest::Client::builder().tls_backend_preconfigured(config))
}

/// Caller supplies the HTTP builder with protected sockets/resolver and timeout.
/// TLS must be explicitly configured, for example with `http_client_builder`.
/// The SDK disables redirects before building that exact client. Middleware can
/// be attached with `with_middleware` without replacing the protected client.
/// Dropping an operation future cancels its pending send/read; no detached tasks.
#[derive(Clone)]
pub struct Client {
    base: Url,
    http: ClientWithMiddleware,
    limit: usize,
    request_timeout: Option<std::time::Duration>,
    interrupt: Option<std::sync::Arc<dyn RequestInterrupt>>,
}

/// Host-owned transport changes, not control authority. Each finite HTTP
/// operation obtains a new waiter; completion cancels its send/body future.
/// SSE streams retain their separate caller-owned cancellation lifecycle.
pub trait RequestInterrupt: Send + Sync + 'static {
    fn interrupted(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;
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
            request_timeout: None,
            interrupt: None,
        })
    }

    pub fn with_middleware<M: Middleware>(mut self, middleware: M) -> Self {
        self.http = ClientBuilder::from_client(self.http)
            .with(middleware)
            .build();
        self
    }

    /// Total budget for ordinary operations only. Streaming callers can instead
    /// configure connect/read-idle timeouts on the same protected HTTP builder.
    pub fn with_request_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.request_timeout = Some(timeout);
        self
    }
    pub fn with_request_interrupt(
        mut self,
        interrupt: Option<std::sync::Arc<dyn RequestInterrupt>>,
    ) -> Self {
        self.interrupt = interrupt;
        self
    }
    async fn finite<T, P>(
        &self,
        operation: impl std::future::Future<Output = Result<T, ClientError<P>>>,
    ) -> Result<T, ClientError<P>> {
        let Some(interrupt) = &self.interrupt else {
            return operation.await;
        };
        match futures_util::future::select(Box::pin(operation), interrupt.interrupted()).await {
            futures_util::future::Either::Left((result, _)) => result,
            futures_util::future::Either::Right(((), _)) => {
                tracing::debug!(
                    stage = "host_interrupt",
                    "Control HTTP transport interrupted"
                );
                Err(ClientError::Network { status: None })
            }
        }
    }

    // Path/query values have already passed generated scalar validation. This
    // second boundary binds every placeholder and encodes decoded UTF-8 once.
    pub(crate) fn parameter_url<P>(
        &self,
        template: &str,
        path: &[(&str, String)],
        query: &[(&str, Option<String>)],
    ) -> Result<Url, ClientError<P>> {
        if !template.starts_with('/') {
            return Err(ClientError::Configuration);
        }
        let mut bound = std::collections::BTreeSet::new();
        let mut encoded = String::new();
        for part in template.split('/').skip(1) {
            encoded.push('/');
            if let Some(name) = part.strip_prefix('{').and_then(|p| p.strip_suffix('}')) {
                if !parameter_name(name) || !bound.insert(name) {
                    return Err(ClientError::Configuration);
                }
                let mut values = path.iter().filter(|(key, _)| *key == name);
                let value = &values.next().ok_or(ClientError::Request)?.1;
                if values.next().is_some()
                    || value.is_empty()
                    || matches!(value.as_str(), "." | "..")
                {
                    return Err(ClientError::Request);
                }
                encoded.push_str(&encode_parameter(value));
            } else if !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            {
                encoded.push_str(part);
            } else {
                return Err(ClientError::Configuration);
            }
        }
        if path.iter().any(|(name, _)| !bound.contains(name)) {
            return Err(ClientError::Request);
        }
        let mut query = query.to_vec();
        query.sort_by(|a, b| a.0.cmp(b.0));
        if query.windows(2).any(|pair| pair[0].0 == pair[1].0)
            || query.iter().any(|(name, _)| !parameter_name(name))
        {
            return Err(ClientError::Configuration);
        }
        let mut separator = '?';
        for (name, value) in query {
            if let Some(value) = value {
                encoded.push(separator);
                separator = '&';
                encoded.push_str(&encode_parameter(name));
                encoded.push('=');
                encoded.push_str(&encode_parameter(&value));
            }
        }
        self.base
            .join(&encoded)
            .map_err(|_| ClientError::Configuration)
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
        let url = self.parameter_url(path, &[], &[])?;
        self.send_url(method, url, token, body).await
    }

    pub(crate) async fn send_url<B: Serialize, P>(
        &self,
        method: Method,
        url: Url,
        token: Option<&str>,
        body: Option<&B>,
    ) -> Result<Response, ClientError<P>> {
        let mut request = self.request_builder(method, url, token, "application/json")?;
        if let Some(timeout) = self.request_timeout {
            request = request.timeout(timeout);
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
        self.finite(async {
            let response = request
                .send()
                .await
                .map_err(|error| network(&error, None))?;
            Response::read(response, self.limit).await
        })
        .await
    }

    /// A body-less request whose success body is raw bytes of `media`.
    /// Error responses still carry the JSON problem envelope.
    pub(crate) async fn send_binary_url<P>(
        &self,
        method: Method,
        url: Url,
        token: Option<&str>,
        media: &'static str,
    ) -> Result<Response, ClientError<P>> {
        let mut request = self.request_builder(method, url, token, media)?;
        if let Some(timeout) = self.request_timeout {
            request = request.timeout(timeout);
        }
        self.finite(async {
            let response = request
                .send()
                .await
                .map_err(|error| network(&error, None))?;
            Response::read_media(response, self.limit, Some(media)).await
        })
        .await
    }

    fn request_builder<P>(
        &self,
        method: Method,
        url: Url,
        token: Option<&str>,
        accept: &'static str,
    ) -> Result<reqwest_middleware::RequestBuilder, ClientError<P>> {
        if url.origin() != self.base.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(ClientError::Configuration);
        }
        let mut request = self
            .http
            .request(method, url)
            .header(header::ACCEPT, accept);
        if let Some(token) = token {
            if token.is_empty() || !token.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
                return Err(ClientError::Configuration);
            }
            let mut value = header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| ClientError::Configuration)?;
            value.set_sensitive(true);
            request = request.header(header::AUTHORIZATION, value);
        }
        Ok(request)
    }

    /// Transport entry used by generated SSE methods. The caller owns retries
    /// and supplies only an already-applied cursor on each new connection.
    pub async fn stream<T: DeserializeOwned, P>(
        &self,
        url: Url,
        token: Option<&str>,
        options: &crate::StreamOptions<'_>,
        contract: crate::StreamResponse<'_, P>,
    ) -> Result<crate::EventStream<T, P>, ClientError<P>> {
        let limit = options.max_frame_bytes.unwrap_or(self.limit);
        if !(1..=16 * 1024 * 1024).contains(&limit) || contract.success_status != 200 {
            return Err(ClientError::Configuration);
        }
        let mut request = self.request_builder(Method::GET, url, token, "text/event-stream")?;
        if let Some(cursor) = options.applied_cursor {
            if cursor.len() > 4096 || !cursor.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
                return Err(ClientError::Configuration);
            }
            if !cursor.is_empty() {
                let mut value = header::HeaderValue::from_str(cursor)
                    .map_err(|_| ClientError::Configuration)?;
                value.set_sensitive(true);
                request = request.header("last-event-id", value);
            }
        }
        let response = request
            .send()
            .await
            .map_err(|error| network(&error, None))?;
        let status = response.status().as_u16();
        if status != contract.success_status {
            return Err(if contract.error_statuses.contains(&status) {
                let response = Response::read(response, self.limit).await?;
                ClientError::Http {
                    status,
                    problem: contract.problem.and_then(|decode| {
                        response.json.then(|| decode(&response.bytes)).flatten()
                    }),
                }
            } else {
                ClientError::Http {
                    status,
                    problem: None,
                }
            });
        }
        if !media_type(&response, "text/event-stream") {
            return Err(ClientError::Decode { status });
        }
        Ok(crate::EventStream::new(response, limit))
    }
}

fn media_type(response: &reqwest::Response, expected: &str) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case(expected))
}

impl Response {
    async fn read<P>(response: reqwest::Response, limit: usize) -> Result<Self, ClientError<P>> {
        Self::read_media(response, limit, None).await
    }

    async fn read_media<P>(
        response: reqwest::Response,
        limit: usize,
        binary: Option<&'static str>,
    ) -> Result<Self, ClientError<P>> {
        let status = response.status().as_u16();
        // Redirects are never followed by the underlying client. Reject their
        // response immediately too, even if its body never reaches EOF.
        if response.status().is_redirection() {
            return Err(ClientError::Http {
                status,
                problem: None,
            });
        }
        let json = media_type(&response, "application/json");
        let raw = response.status().is_success()
            && binary.is_some_and(|media| media_type(&response, media));
        if status == 204 {
            return Ok(Response {
                status,
                bytes: Vec::new(),
                json: false,
                raw: false,
            });
        }
        if !json && !raw {
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
            .is_some_and(|size| size > limit as u64)
        {
            return Err(limit_error(status));
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| network(&error, Some(status)))?;
            if chunk.len() > limit.saturating_sub(bytes.len()) {
                return Err(limit_error(status));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Response {
            status,
            bytes,
            json,
            raw,
        })
    }
}

fn parameter_name(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn encode_parameter(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    encoded
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
    /// A success body of the operation's declared binary media type.
    raw: bool,
}
impl Response {
    pub(crate) fn bytes<P>(self) -> Result<Vec<u8>, ClientError<P>> {
        if !self.raw {
            return Err(ClientError::Decode {
                status: self.status,
            });
        }
        Ok(self.bytes)
    }
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
mod tests;
