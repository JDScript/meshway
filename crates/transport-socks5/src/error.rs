use std::{fmt, io};

/// Standard RFC 1928 CONNECT failure codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ReplyCode {
    GeneralFailure = 1,
    ConnectionNotAllowed = 2,
    NetworkUnreachable = 3,
    HostUnreachable = 4,
    ConnectionRefused = 5,
    TtlExpired = 6,
    CommandNotSupported = 7,
    AddressTypeNotSupported = 8,
}

impl TryFrom<u8> for ReplyCode {
    type Error = Error;
    fn try_from(code: u8) -> Result<Self, Error> {
        match code {
            1 => Ok(Self::GeneralFailure),
            2 => Ok(Self::ConnectionNotAllowed),
            3 => Ok(Self::NetworkUnreachable),
            4 => Ok(Self::HostUnreachable),
            5 => Ok(Self::ConnectionRefused),
            6 => Ok(Self::TtlExpired),
            7 => Ok(Self::CommandNotSupported),
            8 => Ok(Self::AddressTypeNotSupported),
            code => Err(Error::UnknownReplyCode(code)),
        }
    }
}

/// Errors never contain usernames, passwords, or raw authentication frames.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    InvalidCredentialLength,
    InvalidPort,
    InvalidDomain,
    UnsupportedVersion(u8),
    UnsupportedAuthVersion(u8),
    EmptyMethods,
    NoAcceptableMethod,
    UnofferedMethod(u8),
    AuthenticationFailed,
    UnsupportedCommand(u8),
    InvalidReserved(u8),
    UnsupportedAddressType(u8),
    TargetMismatch,
    Reply(ReplyCode),
    UnknownReplyCode(u8),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "SOCKS5 I/O: {error}"),
            Self::InvalidCredentialLength => {
                f.write_str("SOCKS5 credentials must each contain 1..=255 bytes")
            }
            Self::InvalidPort => f.write_str("SOCKS5 target port must be nonzero"),
            Self::InvalidDomain => f.write_str("SOCKS5 address must be a valid ASCII DNS name"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported SOCKS version {version}"),
            Self::UnsupportedAuthVersion(version) => {
                write!(f, "unsupported SOCKS5 authentication version {version}")
            }
            Self::EmptyMethods => f.write_str("SOCKS5 method list is empty"),
            Self::NoAcceptableMethod => f.write_str("no acceptable SOCKS5 authentication method"),
            Self::UnofferedMethod(method) => {
                write!(f, "server selected unoffered SOCKS5 method {method}")
            }
            Self::AuthenticationFailed => f.write_str("SOCKS5 authentication failed"),
            Self::UnsupportedCommand(command) => write!(f, "unsupported SOCKS5 command {command}"),
            Self::InvalidReserved(value) => {
                write!(f, "SOCKS5 reserved field must be zero (received {value})")
            }
            Self::UnsupportedAddressType(kind) => {
                write!(f, "unsupported SOCKS5 address type {kind}")
            }
            Self::TargetMismatch => {
                f.write_str("SOCKS5 target does not match the configured endpoint")
            }
            Self::Reply(code) => write!(f, "SOCKS5 CONNECT rejected: {code:?}"),
            Self::UnknownReplyCode(code) => write!(f, "unknown SOCKS5 reply code {code}"),
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
