//! Pure transport declarations shared by configuration and execution consumers.
//!
//! A declaration is not evidence of a compiled transport, a bound listener,
//! reachability, or authorization. Runtime implementations live elsewhere.
//!
//! This foundation deliberately has no listener/dialer defaults, capability
//! roles, credential generation, socket operations, or TLS implementation.
//! Validated values are immutable and validate on deserialization as well as
//! construction. Consumers must still enforce their resource and lease limits.

#![forbid(unsafe_code)]

mod duration;
mod error;
mod port;
mod protocol;
mod schema;
mod tls;
mod websocket;

pub use duration::PositiveDuration;
pub use error::ValidationError;
pub use port::Port;
pub use protocol::{OuterSocket, TransportProtocol, VlessStream};
pub use tls::{TlsVersion, TlsVersionRange, validate_websocket_alpn};
pub use websocket::{WebSocketHeaderName, WebSocketHeaderValue, WebSocketHeaders, WebSocketPath};
