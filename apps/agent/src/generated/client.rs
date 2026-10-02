//! Generated HTTP client for regular API requests
//!
//! This file contains the HTTP client implementation for GET, POST, etc.
//! Do not edit manually - regenerate using the appropriate script.
#![allow(clippy::format_in_format_args)]
#![allow(clippy::let_unit_value)]
use super::types::*;
use thiserror::Error;
/// The generated validation-problem profile based on RFC 9457.
/// The distinctive namespace avoids collisions with user schemas.
pub mod openapi_to_rust_problem {
    #[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
    pub struct ProblemDetails {
        #[serde(rename = "type")]
        pub type_uri: String,
        pub title: String,
        pub status: u16,
        pub code: String,
        #[serde(default)]
        pub errors: Vec<InvalidParameter>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub detail: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub instance: Option<String>,
    }
    #[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
    pub struct InvalidParameter {
        pub code: String,
        pub location: String,
        pub message: String,
    }
}
/// Request/transport failures, including failures while reading a 2xx body.
/// Non-2xx responses return [`ApiOpError::Api`] immediately with their status
/// and headers; their bodies are never read or classified as transport failures.
#[derive(Error, Debug)]
pub enum HttpError {
    /// Failure from an injected internal transport.
    #[error("Injected transport error: {0}")]
    Io(#[from] std::io::Error),
    /// Network or connection error (from reqwest)
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
    /// Middleware error (from reqwest-middleware)
    #[error("Middleware error: {0}")]
    Middleware(#[from] reqwest_middleware::Error),
    /// Request serialization error
    #[error("Failed to serialize request: {0}")]
    Serialization(String),
    /// Authentication error
    #[error("Authentication error: {0}")]
    Auth(String),
    /// Request timeout
    #[error("Request timeout")]
    Timeout,
    /// A response body exceeded the configured in-memory limit
    #[error("Response body exceeded configured limit of {limit} bytes")]
    ResponseTooLarge { limit: usize },
    /// Invalid configuration
    #[error("Configuration error: {0}")]
    Config(String),
    /// Generic error
    #[error("{0}")]
    Other(String),
}
impl HttpError {
    /// Create a serialization error
    pub fn serialization_error(error: impl std::fmt::Display) -> Self {
        Self::Serialization(error.to_string())
    }
    /// Check if this transport error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Network(_) | Self::Middleware(_) | Self::Timeout)
            || matches!(self, Self::Io(error) if !matches!(error.kind(), std::io::ErrorKind::InvalidInput | std::io::ErrorKind::Unsupported | std::io::ErrorKind::PermissionDenied))
    }
}
/// An HTTP rejection or a 2xx response that could not become a typed result.
/// Status and headers are preserved. Meshway returns non-2xx responses before
/// body I/O: `body` and `raw_body` are empty, `typed` and `parse_error` are `None`.
/// Skipping an error body is not a deserialization failure. For 2xx decoding
/// failures, raw bodies and parse values are discarded; only a safe category remains.
#[derive(Clone)]
pub struct ApiError<E> {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: String,
    /// Always empty: response bytes must not escape through exceptions.
    pub raw_body: Vec<u8>,
    pub typed: Option<E>,
    pub parse_error: Option<String>,
}
impl<E> ApiError<E> {
    /// Stable error classification without raw body, parse values or headers.
    pub fn category(&self) -> &'static str {
        if !(200..300).contains(&self.status) {
            "http_rejection"
        } else if self.parse_error.as_deref() == Some("success_response_decode") {
            "success_response_decode"
        } else {
            "unexpected_success_status"
        }
    }

    pub fn is_client_error(&self) -> bool {
        (400..500).contains(&self.status)
    }
    pub fn is_server_error(&self) -> bool {
        (500..600).contains(&self.status)
    }
    /// Retry guidance for the response. Mirrors the previous
    /// HttpError logic for backwards-compatible retry middleware.
    pub fn is_retryable(&self) -> bool {
        matches!(self.status, 429 | 500 | 502 | 503 | 504)
    }
    /// Decode the generated RFC 9457 validation-problem profile
    /// without replacing a documented per-operation error in `typed`.
    ///
    /// Returns `None` unless the response's `Content-Type` is
    /// `application/problem+json`, which is how RFC 9457 identifies
    /// a problem document. Most third-party APIs return their
    /// errors as plain `application/json`, so this yields `None`
    /// against them by design — use `typed` for a documented
    /// per-operation error body, or `body` for the raw payload.
    /// Meshway skips non-2xx bodies, so their envelopes return `None`.
    pub fn problem_details(&self) -> Option<openapi_to_rust_problem::ProblemDetails> {
        let content_type = self
            .headers
            .get(reqwest::header::CONTENT_TYPE)?
            .to_str()
            .ok()?;
        let media_type = content_type.split(';').next()?.trim();
        if !media_type.eq_ignore_ascii_case("application/problem+json") {
            return None;
        }
        serde_json::from_str(&self.body).ok()
    }
}
impl<E> std::fmt::Debug for ApiError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiError")
            .field("status", &self.status)
            .field("category", &self.category())
            .finish_non_exhaustive()
    }
}
impl<E> std::fmt::Display for ApiError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "API error {} ({})", self.status, self.category())
    }
}
impl<E: std::fmt::Debug> std::error::Error for ApiError<E> {}
/// Result error type returned by every generated operation method.
/// `Transport` covers request failures and failures reading a 2xx body. `Api`
/// preserves status and headers for all non-2xx responses immediately, without
/// reading their bodies, and for 2xx responses that fail typed decoding.
#[derive(Debug, Error)]
pub enum ApiOpError<E: std::fmt::Debug> {
    #[error(transparent)]
    Transport(#[from] HttpError),
    #[error(transparent)]
    Api(ApiError<E>),
}
impl<E: std::fmt::Debug> ApiOpError<E> {
    /// Returns the API envelope when this is an `Api` variant.
    pub fn api(&self) -> Option<&ApiError<E>> {
        match self {
            Self::Api(e) => Some(e),
            Self::Transport(_) => None,
        }
    }
    /// True when the underlying error came from the server (i.e.
    /// any `Api` variant) rather than the transport layer.
    pub fn is_api_error(&self) -> bool {
        matches!(self, Self::Api(_))
    }
}
impl<E: std::fmt::Debug> From<reqwest::Error> for ApiOpError<E> {
    fn from(e: reqwest::Error) -> Self {
        Self::Transport(HttpError::Network(e))
    }
}
impl<E: std::fmt::Debug> From<reqwest_middleware::Error> for ApiOpError<E> {
    fn from(e: reqwest_middleware::Error) -> Self {
        Self::Transport(HttpError::Middleware(e))
    }
}
/// Result alias for transport-only error paths (e.g. helpers that
/// don't have a per-operation error type). Generated operation
/// methods use [`ApiOpError`] directly.
pub type HttpResult<T> = Result<T, HttpError>;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use std::collections::BTreeMap;
/// Default upper bound for any response body buffered in memory.
pub const DEFAULT_MAX_RESPONSE_BODY_BYTES: usize = 8 * 1024 * 1024;
/// HTTP client for making API requests
/// Transport injection for internal socket protection. Implementations must
/// enforce connection, TLS, proxy, timeout and retirement semantics; generated
/// code never falls back to reqwest after an injected transport fails.
pub trait HttpTransport: Send + Sync {
    fn execute(
        &self,
        request: reqwest::Request,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = HttpResult<reqwest::Response>> + Send + '_>,
    >;
}
#[derive(Clone)]
pub struct HttpClient {
    base_url: String,
    api_key: Option<String>,
    http_client: ClientWithMiddleware,
    transport: Option<std::sync::Arc<dyn HttpTransport>>,
    custom_headers: BTreeMap<String, String>,
    max_response_body_bytes: usize,
}
async fn __read_bounded_response_body(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, HttpError> {
    use futures_util::StreamExt;
    let mut body = Vec::new();
    let mut stream = std::pin::pin!(response.bytes_stream());
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(HttpError::Network)?;
        let next_len = body.len().checked_add(chunk.len());
        if next_len.is_none_or(|next_len| next_len > limit) {
            return Err(HttpError::ResponseTooLarge { limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
impl HttpClient {
    /// Create a new HTTP client with default configuration
    pub fn new() -> Self {
        let reqwest_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("valid static HTTP client configuration");
        let client_builder = ClientBuilder::new(reqwest_client);
        let http_client = client_builder.build();
        Self {
            base_url: "http://127.0.0.1:8420".to_string(),
            api_key: None,
            http_client,
            transport: None,
            custom_headers: BTreeMap::new(),
            max_response_body_bytes: 2097152usize,
        }
    }
    /// Inject a transport. All generated operations still build their own
    /// authoritative URL, authentication, headers and JSON body, and decode
    /// responses through the same bounded, status-first implementation.
    pub fn with_transport(mut self, transport: std::sync::Arc<dyn HttpTransport>) -> Self {
        self.transport = Some(transport);
        self
    }
    /// Set the base URL for all requests
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
    /// Set the API key for authentication
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }
    /// Set the maximum number of response-body bytes buffered in memory.
    pub fn with_max_response_body_bytes(mut self, limit: usize) -> Self {
        self.max_response_body_bytes = limit;
        self
    }
    /// Add a custom header to all requests
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.custom_headers.insert(name.into(), value.into());
        self
    }
    /// Add multiple custom headers
    pub fn with_headers(mut self, headers: BTreeMap<String, String>) -> Self {
        self.custom_headers.extend(headers);
        self
    }
}
impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}
fn __pct_encode_path_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}
///Typed error responses for `acknowledgeAgentConnections`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum AcknowledgeAgentConnectionsApiError {
    Status400(ErrorResponse),
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status409(ErrorResponse),
    Status413(ErrorResponse),
    Status415(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `acknowledgeConfiguration`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum AcknowledgeConfigurationApiError {
    Status400(ErrorResponse),
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status409(ErrorResponse),
    Status413(ErrorResponse),
    Status415(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `beginConnectionRuntime`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum BeginConnectionRuntimeApiError {
    Status400(ErrorResponse),
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status409(ErrorResponse),
    Status413(ErrorResponse),
    Status415(ErrorResponse),
    Status422(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `enrollAgent`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum EnrollAgentApiError {
    Status400(ErrorResponse),
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status409(ErrorResponse),
    Status413(ErrorResponse),
    Status415(ErrorResponse),
    Status422(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `exchangeDevice`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum ExchangeDeviceApiError {
    Status400(ErrorResponse),
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status409(ErrorResponse),
    Status413(ErrorResponse),
    Status415(ErrorResponse),
    Status422(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `getAgentConfiguration`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum GetAgentConfigurationApiError {
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `getAgentConnections`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum GetAgentConnectionsApiError {
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `getAgentDataplane`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum GetAgentDataplaneApiError {
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status422(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `getAgentDns`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum GetAgentDnsApiError {
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status422(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `getAgentManagedConnections`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum GetAgentManagedConnectionsApiError {
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `getAgentManagedDataplane`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum GetAgentManagedDataplaneApiError {
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status422(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `getConnectionRuntimeEpoch`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum GetConnectionRuntimeEpochApiError {
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status422(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `heartbeatAgent`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum HeartbeatAgentApiError {
    Status400(ErrorResponse),
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status413(ErrorResponse),
    Status415(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `publishConnectionIdentity`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum PublishConnectionIdentityApiError {
    Status400(ErrorResponse),
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status409(ErrorResponse),
    Status413(ErrorResponse),
    Status415(ErrorResponse),
    Status422(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `publishManagedTransportCapabilities`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum PublishManagedTransportCapabilitiesApiError {
    Status400(ErrorResponse),
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status413(ErrorResponse),
    Status415(ErrorResponse),
    Status422(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `reportConnectionRuntime`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum ReportConnectionRuntimeApiError {
    Status400(ErrorResponse),
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status409(ErrorResponse),
    Status413(ErrorResponse),
    Status415(ErrorResponse),
    Status422(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `reportEndpointDiscovery`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum ReportEndpointDiscoveryApiError {
    Status400(ErrorResponse),
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status409(ErrorResponse),
    Status413(ErrorResponse),
    Status415(ErrorResponse),
    Status422(ErrorResponse),
    Status500(ErrorResponse),
}
///Typed error responses for `reportLatencySamples`. One variant per declared non-2xx response.
#[derive(Debug, Clone)]
pub enum ReportLatencySamplesApiError {
    Status400(ErrorResponse),
    Status401(ErrorResponse),
    Status403(ErrorResponse),
    Status409(ErrorResponse),
    Status413(ErrorResponse),
    Status415(ErrorResponse),
    Status422(ErrorResponse),
    Status500(ErrorResponse),
}
impl HttpClient {
    /// `POST /api/v1/agent/connections/ack`
    pub async fn acknowledge_agent_connections(
        &self,
        request: AcknowledgeAgentConnectionsRequest,
    ) -> Result<AcknowledgeAgentConnectionsResponse, ApiOpError<AcknowledgeAgentConnectionsApiError>>
    {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agent/connections/ack");
        let mut req = self.http_client.post(request_url);
        req = req
            .body(serde_json::to_vec(&request).map_err(HttpError::serialization_error)?)
            .header("content-type", "application/json");
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<AcknowledgeAgentConnectionsApiError>;
            let parse_error: Option<String>;
            match status_code {
                400u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(AcknowledgeAgentConnectionsApiError::Status400(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(AcknowledgeAgentConnectionsApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(AcknowledgeAgentConnectionsApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                409u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(AcknowledgeAgentConnectionsApiError::Status409(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                413u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(AcknowledgeAgentConnectionsApiError::Status413(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                415u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(AcknowledgeAgentConnectionsApiError::Status415(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(AcknowledgeAgentConnectionsApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `POST /api/v1/agents/configuration/ack`
    pub async fn acknowledge_configuration(
        &self,
        request: AcknowledgeConfigurationRequest,
    ) -> Result<AcknowledgeConfigurationResponse, ApiOpError<AcknowledgeConfigurationApiError>>
    {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agents/configuration/ack");
        let mut req = self.http_client.post(request_url);
        req = req
            .body(serde_json::to_vec(&request).map_err(HttpError::serialization_error)?)
            .header("content-type", "application/json");
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<AcknowledgeConfigurationApiError>;
            let parse_error: Option<String>;
            match status_code {
                400u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(AcknowledgeConfigurationApiError::Status400(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(AcknowledgeConfigurationApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(AcknowledgeConfigurationApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                409u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(AcknowledgeConfigurationApiError::Status409(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                413u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(AcknowledgeConfigurationApiError::Status413(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                415u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(AcknowledgeConfigurationApiError::Status415(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(AcknowledgeConfigurationApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `POST /api/v1/agent/connections/runtime/begin`
    pub async fn begin_connection_runtime(
        &self,
        request: BeginConnectionRuntimeRequest,
    ) -> Result<BeginConnectionRuntimeResponse, ApiOpError<BeginConnectionRuntimeApiError>> {
        let request_url = format!(
            "{}{}",
            self.base_url, "/api/v1/agent/connections/runtime/begin"
        );
        let mut req = self.http_client.post(request_url);
        req = req
            .body(serde_json::to_vec(&request).map_err(HttpError::serialization_error)?)
            .header("content-type", "application/json");
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<BeginConnectionRuntimeApiError>;
            let parse_error: Option<String>;
            match status_code {
                400u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(BeginConnectionRuntimeApiError::Status400(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(BeginConnectionRuntimeApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(BeginConnectionRuntimeApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                409u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(BeginConnectionRuntimeApiError::Status409(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                413u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(BeginConnectionRuntimeApiError::Status413(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                415u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(BeginConnectionRuntimeApiError::Status415(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                422u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(BeginConnectionRuntimeApiError::Status422(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(BeginConnectionRuntimeApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `POST /api/v1/agents/enroll`
    pub async fn enroll_agent(
        &self,
        request: EnrollAgentRequest,
    ) -> Result<AgentNodeResponse, ApiOpError<EnrollAgentApiError>> {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agents/enroll");
        let mut req = self.http_client.post(request_url);
        req = req
            .body(serde_json::to_vec(&request).map_err(HttpError::serialization_error)?)
            .header("content-type", "application/json");
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<EnrollAgentApiError>;
            let parse_error: Option<String>;
            match status_code {
                400u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(EnrollAgentApiError::Status400(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(EnrollAgentApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(EnrollAgentApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                409u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(EnrollAgentApiError::Status409(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                413u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(EnrollAgentApiError::Status413(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                415u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(EnrollAgentApiError::Status415(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                422u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(EnrollAgentApiError::Status422(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(EnrollAgentApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `POST /api/v1/devices/exchange`
    pub async fn exchange_device(
        &self,
        request: ExchangeDeviceRequest,
    ) -> Result<AgentNodeResponse, ApiOpError<ExchangeDeviceApiError>> {
        let request_url = format!("{}{}", self.base_url, "/api/v1/devices/exchange");
        let mut req = self.http_client.post(request_url);
        req = req
            .body(serde_json::to_vec(&request).map_err(HttpError::serialization_error)?)
            .header("content-type", "application/json");
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<ExchangeDeviceApiError>;
            let parse_error: Option<String>;
            match status_code {
                400u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ExchangeDeviceApiError::Status400(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ExchangeDeviceApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ExchangeDeviceApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                409u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ExchangeDeviceApiError::Status409(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                413u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ExchangeDeviceApiError::Status413(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                415u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ExchangeDeviceApiError::Status415(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                422u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ExchangeDeviceApiError::Status422(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ExchangeDeviceApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `GET /api/v1/agents/configuration`
    pub async fn get_agent_configuration(
        &self,
    ) -> Result<ConfigurationSnapshot, ApiOpError<GetAgentConfigurationApiError>> {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agents/configuration");
        let mut req = self.http_client.get(request_url);
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<GetAgentConfigurationApiError>;
            let parse_error: Option<String>;
            match status_code {
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentConfigurationApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentConfigurationApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentConfigurationApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `GET /api/v1/agent/connections`
    pub async fn get_agent_connections(
        &self,
    ) -> Result<AgentConnectionsSnapshot, ApiOpError<GetAgentConnectionsApiError>> {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agent/connections");
        let mut req = self.http_client.get(request_url);
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<GetAgentConnectionsApiError>;
            let parse_error: Option<String>;
            match status_code {
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentConnectionsApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentConnectionsApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentConnectionsApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `GET /api/v1/agent/dataplane`
    pub async fn get_agent_dataplane(
        &self,
    ) -> Result<AgentDataplaneSnapshot, ApiOpError<GetAgentDataplaneApiError>> {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agent/dataplane");
        let mut req = self.http_client.get(request_url);
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<GetAgentDataplaneApiError>;
            let parse_error: Option<String>;
            match status_code {
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentDataplaneApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentDataplaneApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                422u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentDataplaneApiError::Status422(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentDataplaneApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `GET /api/v1/agent/dns`
    pub async fn get_agent_dns(&self) -> Result<AgentDnsSnapshot, ApiOpError<GetAgentDnsApiError>> {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agent/dns");
        let mut req = self.http_client.get(request_url);
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<GetAgentDnsApiError>;
            let parse_error: Option<String>;
            match status_code {
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentDnsApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentDnsApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                422u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentDnsApiError::Status422(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentDnsApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `GET /api/v1/agent/managed-connections`
    pub async fn get_agent_managed_connections(
        &self,
    ) -> Result<AgentManagedConnectionsSnapshot, ApiOpError<GetAgentManagedConnectionsApiError>>
    {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agent/managed-connections");
        let mut req = self.http_client.get(request_url);
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<GetAgentManagedConnectionsApiError>;
            let parse_error: Option<String>;
            match status_code {
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentManagedConnectionsApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentManagedConnectionsApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentManagedConnectionsApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `GET /api/v1/agent/managed-dataplane`
    pub async fn get_agent_managed_dataplane(
        &self,
    ) -> Result<AgentManagedDataplaneSnapshot, ApiOpError<GetAgentManagedDataplaneApiError>> {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agent/managed-dataplane");
        let mut req = self.http_client.get(request_url);
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<GetAgentManagedDataplaneApiError>;
            let parse_error: Option<String>;
            match status_code {
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentManagedDataplaneApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentManagedDataplaneApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                422u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentManagedDataplaneApiError::Status422(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetAgentManagedDataplaneApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `GET /api/v1/agent/connections/runtime`
    pub async fn get_connection_runtime_epoch(
        &self,
    ) -> Result<GetConnectionRuntimeEpochResponse, ApiOpError<GetConnectionRuntimeEpochApiError>>
    {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agent/connections/runtime");
        let mut req = self.http_client.get(request_url);
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<GetConnectionRuntimeEpochApiError>;
            let parse_error: Option<String>;
            match status_code {
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetConnectionRuntimeEpochApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetConnectionRuntimeEpochApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                422u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetConnectionRuntimeEpochApiError::Status422(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(GetConnectionRuntimeEpochApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `POST /api/v1/agents/heartbeat`
    pub async fn heartbeat_agent(
        &self,
        request: HeartbeatRequest,
    ) -> Result<AgentNodeResponse, ApiOpError<HeartbeatAgentApiError>> {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agents/heartbeat");
        let mut req = self.http_client.post(request_url);
        req = req
            .body(serde_json::to_vec(&request).map_err(HttpError::serialization_error)?)
            .header("content-type", "application/json");
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<HeartbeatAgentApiError>;
            let parse_error: Option<String>;
            match status_code {
                400u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(HeartbeatAgentApiError::Status400(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(HeartbeatAgentApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(HeartbeatAgentApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                413u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(HeartbeatAgentApiError::Status413(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                415u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(HeartbeatAgentApiError::Status415(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(HeartbeatAgentApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `POST /api/v1/agent/connection-identity`
    pub async fn publish_connection_identity(
        &self,
        request: PublishConnectionIdentityRequest,
    ) -> Result<PublishConnectionIdentityResponse, ApiOpError<PublishConnectionIdentityApiError>>
    {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agent/connection-identity");
        let mut req = self.http_client.post(request_url);
        req = req
            .body(serde_json::to_vec(&request).map_err(HttpError::serialization_error)?)
            .header("content-type", "application/json");
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<PublishConnectionIdentityApiError>;
            let parse_error: Option<String>;
            match status_code {
                400u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishConnectionIdentityApiError::Status400(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishConnectionIdentityApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishConnectionIdentityApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                409u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishConnectionIdentityApiError::Status409(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                413u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishConnectionIdentityApiError::Status413(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                415u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishConnectionIdentityApiError::Status415(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                422u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishConnectionIdentityApiError::Status422(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishConnectionIdentityApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `POST /api/v1/agent/managed-transport-capabilities`
    pub async fn publish_managed_transport_capabilities(
        &self,
        request: PublishManagedTransportCapabilitiesRequest,
    ) -> Result<
        PublishManagedTransportCapabilitiesResponse,
        ApiOpError<PublishManagedTransportCapabilitiesApiError>,
    > {
        let request_url = format!(
            "{}{}",
            self.base_url, "/api/v1/agent/managed-transport-capabilities"
        );
        let mut req = self.http_client.post(request_url);
        req = req
            .body(serde_json::to_vec(&request).map_err(HttpError::serialization_error)?)
            .header("content-type", "application/json");
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<PublishManagedTransportCapabilitiesApiError>;
            let parse_error: Option<String>;
            match status_code {
                400u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishManagedTransportCapabilitiesApiError::Status400(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishManagedTransportCapabilitiesApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishManagedTransportCapabilitiesApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                413u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishManagedTransportCapabilitiesApiError::Status413(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                415u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishManagedTransportCapabilitiesApiError::Status415(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                422u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishManagedTransportCapabilitiesApiError::Status422(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(PublishManagedTransportCapabilitiesApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `POST /api/v1/agent/connections/runtime`
    pub async fn report_connection_runtime(
        &self,
        request: ReportConnectionRuntimeRequest,
    ) -> Result<ReportConnectionRuntimeResponse, ApiOpError<ReportConnectionRuntimeApiError>> {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agent/connections/runtime");
        let mut req = self.http_client.post(request_url);
        req = req
            .body(serde_json::to_vec(&request).map_err(HttpError::serialization_error)?)
            .header("content-type", "application/json");
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<ReportConnectionRuntimeApiError>;
            let parse_error: Option<String>;
            match status_code {
                400u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportConnectionRuntimeApiError::Status400(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportConnectionRuntimeApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportConnectionRuntimeApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                409u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportConnectionRuntimeApiError::Status409(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                413u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportConnectionRuntimeApiError::Status413(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                415u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportConnectionRuntimeApiError::Status415(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                422u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportConnectionRuntimeApiError::Status422(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportConnectionRuntimeApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `POST /api/v1/agent/connections/discovery`
    pub async fn report_endpoint_discovery(
        &self,
        request: ReportEndpointDiscoveryRequest,
    ) -> Result<ReportEndpointDiscoveryResponse, ApiOpError<ReportEndpointDiscoveryApiError>> {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agent/connections/discovery");
        let mut req = self.http_client.post(request_url);
        req = req
            .body(serde_json::to_vec(&request).map_err(HttpError::serialization_error)?)
            .header("content-type", "application/json");
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<ReportEndpointDiscoveryApiError>;
            let parse_error: Option<String>;
            match status_code {
                400u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportEndpointDiscoveryApiError::Status400(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportEndpointDiscoveryApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportEndpointDiscoveryApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                409u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportEndpointDiscoveryApiError::Status409(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                413u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportEndpointDiscoveryApiError::Status413(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                415u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportEndpointDiscoveryApiError::Status415(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                422u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportEndpointDiscoveryApiError::Status422(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportEndpointDiscoveryApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
    /// `POST /api/v1/agent/latency/samples`
    pub async fn report_latency_samples(
        &self,
        request: ReportLatencySamplesRequest,
    ) -> Result<ReportLatencySamplesResponse, ApiOpError<ReportLatencySamplesApiError>> {
        let request_url = format!("{}{}", self.base_url, "/api/v1/agent/latency/samples");
        let mut req = self.http_client.post(request_url);
        req = req
            .body(serde_json::to_vec(&request).map_err(HttpError::serialization_error)?)
            .header("content-type", "application/json");
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }
        for (name, value) in &self.custom_headers {
            if !name.eq_ignore_ascii_case("accept") {
                req = req.header(name, value);
            }
        }
        req = req.header(reqwest::header::ACCEPT, "application/json");
        let response = if let Some(transport) = &self.transport {
            transport.execute(req.build()?).await?
        } else {
            req.send().await?
        };
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response.headers().clone();
        // Meshway acts on HTTP rejection immediately; error bodies are not consumed.
        if !status.is_success() {
            return Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: None,
            }));
        }
        let body_bytes =
            __read_bounded_response_body(response, self.max_response_body_bytes).await?;
        let raw_body = body_bytes;
        let body_text = String::from_utf8_lossy(&raw_body).into_owned();
        if false || status_code == 200u16 {
            match serde_json::from_str(&body_text) {
                Ok(body) => Ok(body),
                Err(_e) => Err(ApiOpError::Api(ApiError {
                    status: status_code,
                    headers: headers,
                    body: String::new(),
                    raw_body: Vec::new(),
                    typed: None,
                    parse_error: Some("success_response_decode".to_owned()),
                })),
            }
        } else if status.is_success() {
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed: None,
                parse_error: Some(format!(
                    "unexpected successful status {}; generated return type selects `{}`",
                    status_code, "200",
                )),
            }))
        } else {
            let typed: Option<ReportLatencySamplesApiError>;
            let parse_error: Option<String>;
            match status_code {
                400u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportLatencySamplesApiError::Status400(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                401u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportLatencySamplesApiError::Status401(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                403u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportLatencySamplesApiError::Status403(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                409u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportLatencySamplesApiError::Status409(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                413u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportLatencySamplesApiError::Status413(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                415u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportLatencySamplesApiError::Status415(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                422u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportLatencySamplesApiError::Status422(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                500u16 => match serde_json::from_str::<ErrorResponse>(&body_text) {
                    Ok(v) => {
                        typed = Some(ReportLatencySamplesApiError::Status500(v));
                        parse_error = None;
                    }
                    Err(e) => {
                        typed = None;
                        parse_error = Some(e.to_string());
                    }
                },
                _ => {
                    typed = None;
                    parse_error = None;
                }
            }
            Err(ApiOpError::Api(ApiError {
                status: status_code,
                headers,
                body: String::new(),
                raw_body: Vec::new(),
                typed,
                parse_error,
            }))
        }
    }
}
