use std::{fmt, io};

/// Protocol errors never contain the UUID or attacker-supplied addon contents.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    InvalidUuid,
    InvalidPort,
    InvalidDomain,
    UnsupportedVersion(u8),
    /// All nonempty addons are unsupported, including malformed protobuf.
    UnsupportedAddons(u8),
    AuthenticationFailed,
    UnsupportedCommand(u8),
    UnsupportedAddressType(u8),
    TargetMismatch,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "VLESS I/O: {error}"),
            Self::InvalidUuid => f.write_str("VLESS UUID must be canonical hyphenated ASCII hex"),
            Self::InvalidPort => f.write_str("VLESS target port must be nonzero"),
            Self::InvalidDomain => f.write_str("VLESS target must be a valid ASCII DNS name"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported VLESS version {version}"),
            Self::UnsupportedAddons(length) => {
                write!(f, "VLESS addons are unsupported (declared length {length})")
            }
            Self::AuthenticationFailed => f.write_str("VLESS authentication failed"),
            Self::UnsupportedCommand(command) => write!(f, "unsupported VLESS command {command}"),
            Self::UnsupportedAddressType(kind) => {
                write!(f, "unsupported VLESS address type {kind}")
            }
            Self::TargetMismatch => {
                f.write_str("VLESS target does not match the configured endpoint")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl Error {
    pub(crate) fn into_io(self) -> io::Error {
        match self {
            Self::Io(error) => error,
            error => io::Error::new(io::ErrorKind::InvalidData, error),
        }
    }
}
