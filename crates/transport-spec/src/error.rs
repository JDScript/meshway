use thiserror::Error;

/// Invalid pure protocol values. Errors do not echo potentially sensitive input.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("unknown transport protocol key")]
    TransportProtocol,
    #[error("outer socket must be udp or tcp")]
    OuterSocket,
    #[error("VLESS stream must be tcp or websocket")]
    VlessStream,
    #[error("TLS version must be 1.2 or 1.3")]
    TlsVersion,
    #[error("port must be in 1..=65535")]
    Port,
    #[error(
        "duration must be a positive decimal integer followed by ms, s, m, or h, without leading zeros"
    )]
    DurationSyntax,
    #[error("duration exceeds u64::MAX milliseconds")]
    DurationRange,
    #[error(
        "WebSocket path must start with / and contain no query, fragment, whitespace, or control characters"
    )]
    WebSocketPath,
    #[error("header name must be a nonempty HTTP token")]
    HeaderName,
    #[error("custom headers cannot override WebSocket handshake or HTTP message framing fields")]
    ReservedHeader,
    #[error("header value cannot contain control characters other than horizontal tab")]
    HeaderValue,
    #[error("header names must be unique, ignoring ASCII case")]
    DuplicateHeader,
    #[error("TLS minimum version must not exceed maximum version")]
    TlsVersionOrder,
    #[error("WebSocket ALPN must be empty or contain only http/1.1")]
    WebSocketAlpn,
}
