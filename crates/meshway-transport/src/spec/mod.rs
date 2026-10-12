//! Pure transport declarations shared by configuration and execution consumers.
//!
//! A declaration is not evidence of a compiled transport, a bound listener,
//! reachability, or authorization. Runtime implementations live elsewhere.
//!
//! Declared defaults do not imply runtime capability support. There is no
//! credential generation, socket operation, or TLS implementation.
//! Validated values are immutable and validate on deserialization as well as
//! construction. Consumers must still enforce their resource and lease limits.

mod duration;
mod error;
mod object;
mod port;
mod prepared;
mod schema;
mod secret;
mod settings;
mod tls;
mod validation;
mod websocket;

pub use duration::PositiveDuration;
pub use error::ValidationError;
pub use port::Port;
pub use prepared::{ClientTls, DialParameters};
pub use secret::{ListenerSecret, SecretString};
pub use settings::*;
pub use tls::{TlsVersion, validate_websocket_alpn};
pub use validation::{
    DeclarationError, MaterialRequirement, PreparedListener, normalize_host, validate_vless_uuid,
};
pub use websocket::{WebSocketHeaderName, WebSocketHeaderValue, WebSocketHeaders, WebSocketPath};
