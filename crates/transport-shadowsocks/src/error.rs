use std::{fmt, io};

/// Errors never retain upstream messages, credentials, or untrusted target text.
#[derive(Debug)]
pub enum Error {
    UnsupportedCipher,
    InvalidPassword,
    InvalidPort,
    InvalidDomain,
    InvalidConfiguration,
    TargetMismatch,
    Io(io::ErrorKind),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedCipher => "unsupported Shadowsocks cipher",
            Self::InvalidPassword => "Shadowsocks password must contain 1 to 1024 UTF-8 bytes",
            Self::InvalidPort => "target port must be nonzero",
            Self::InvalidDomain => "target must be an ASCII DNS name of at most 253 bytes",
            Self::InvalidConfiguration => "invalid Shadowsocks configuration",
            Self::TargetMismatch => "Shadowsocks target does not match the configured endpoint",
            Self::Io(_) => "Shadowsocks carrier I/O or authentication failed",
        })
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}
