use std::{fmt, io};

/// Errors contain no password, hash, private key, certificate or peer bytes.
/// I/O errors retain only their kind, including TLS verification errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Io(io::ErrorKind),
    TlsHandshake(io::ErrorKind),
    InvalidPassword,
    InvalidPort,
    InvalidDomain,
    AuthenticationFailed,
    InvalidCrlf,
    UnsupportedCommand(u8),
    UnsupportedAddressType(u8),
    TargetMismatch,
    EmptyTrustStore,
    InvalidServerName,
    TlsConfig,
    Timeout,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(kind) => write!(f, "Trojan I/O: {kind}"),
            Self::TlsHandshake(kind) => write!(f, "Trojan TLS handshake: {kind}"),
            Self::InvalidPassword => f.write_str("Trojan password must contain 1..=4096 bytes"),
            Self::InvalidPort => f.write_str("Trojan target port must be nonzero"),
            Self::InvalidDomain => f.write_str("Trojan target must be a valid ASCII DNS name"),
            Self::AuthenticationFailed => f.write_str("Trojan authentication failed"),
            Self::InvalidCrlf => f.write_str("invalid Trojan header delimiter"),
            Self::UnsupportedCommand(command) => write!(f, "unsupported Trojan command {command}"),
            Self::UnsupportedAddressType(kind) => {
                write!(f, "unsupported Trojan address type {kind}")
            }
            Self::TargetMismatch => {
                f.write_str("Trojan target does not match the configured endpoint")
            }
            Self::EmptyTrustStore => f.write_str("Trojan TLS requires trusted CA certificates"),
            Self::InvalidServerName => f.write_str("invalid Trojan TLS server name"),
            Self::TlsConfig => f.write_str("invalid Trojan TLS certificate or key configuration"),
            Self::Timeout => f.write_str("Trojan connection handshake timed out"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}
