use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::ValidationError;

/// Supported TLS protocol version bound, encoded as `1.2` or `1.3`.
#[derive(
    Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash, ToSchema,
)]
#[serde(try_from = "String")]
pub enum TlsVersion {
    /// Permit TLS 1.2 at this version bound.
    #[serde(rename = "1.2")]
    Tls12,
    /// Permit TLS 1.3 at this version bound.
    #[serde(rename = "1.3")]
    Tls13,
}

impl TryFrom<String> for TlsVersion {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "1.2" => Ok(Self::Tls12),
            "1.3" => Ok(Self::Tls13),
            _ => Err(ValidationError::TlsVersion),
        }
    }
}

impl TlsVersion {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tls12 => "1.2",
            Self::Tls13 => "1.3",
        }
    }
}

pub(crate) fn validate_tls_versions(
    min_version: TlsVersion,
    max_version: TlsVersion,
) -> Result<(), ValidationError> {
    if min_version > max_version {
        Err(ValidationError::TlsVersionOrder)
    } else {
        Ok(())
    }
}

/// Validate the pure WebSocket/ALPN combination before execution.
/// Empty ALPN is permitted, but does not assert runtime HTTP/1.1 support.
/// All advertised names must be exactly `http/1.1`; merely including it next
/// to `h2` is not sufficient. Order is preserved and duplicates are harmless.
pub fn validate_websocket_alpn(alpn: &[String]) -> Result<(), ValidationError> {
    if alpn.iter().all(|protocol| protocol == "http/1.1") {
        Ok(())
    } else {
        Err(ValidationError::WebSocketAlpn)
    }
}
