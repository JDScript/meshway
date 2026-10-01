use std::{
    fmt,
    net::{IpAddr, SocketAddr},
    num::NonZeroU16,
};

use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::Error;

/// RFC 1929 credentials, not a Meshway peer identity. Both fields are opaque
/// octets of length 1..=255 (UTF-8, if used, is measured in bytes).
/// Debug is redacted; no Display, serialization, or public secret getters exist.
#[derive(Clone)]
pub struct Credentials {
    // Length plus zero-padded value: compare all bytes, including both lengths,
    // without early-returning on a username mismatch or different secret length.
    pub(crate) username: Box<Zeroizing<[u8; 256]>>,
    pub(crate) password: Box<Zeroizing<[u8; 256]>>,
}

impl Credentials {
    pub fn new(username: &[u8], password: &[u8]) -> Result<Self, Error> {
        if !(1..=255).contains(&username.len()) || !(1..=255).contains(&password.len()) {
            return Err(Error::InvalidCredentialLength);
        }
        let mut value = Self::empty();
        value.username[0] = username.len() as u8;
        value.username[1..=username.len()].copy_from_slice(username);
        value.password[0] = password.len() as u8;
        value.password[1..=password.len()].copy_from_slice(password);
        Ok(value)
    }

    pub(crate) fn empty() -> Self {
        Self {
            username: Box::new(Zeroizing::new([0; 256])),
            password: Box::new(Zeroizing::new([0; 256])),
        }
    }

    pub(crate) fn matches(&self, other: &Self) -> bool {
        bool::from(
            self.username[..].ct_eq(&other.username[..])
                & self.password[..].ct_eq(&other.password[..]),
        )
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credentials([REDACTED])")
    }
}

/// Exactly one permitted method on either side; there is no implicit fallback.
#[derive(Clone, Debug)]
pub enum Auth {
    NoAuth,
    UsernamePassword(Credentials),
}

impl Auth {
    pub(crate) fn method(&self) -> u8 {
        match self {
            Self::NoAuth => 0,
            Self::UsernamePassword(_) => 2,
        }
    }
}

/// Address kind is significant. Domains are never resolved for target matching;
/// IPv4 and IPv4-mapped IPv6 addresses remain different.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetHost {
    Ip(IpAddr),
    Domain(String),
}

/// Validated destination with a nonzero port. Domains are ASCII DNS names
/// (IDNs must be A-labels), lowercased with one trailing root dot removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    host: TargetHost,
    port: NonZeroU16,
}

impl Target {
    pub fn ip(address: IpAddr, port: u16) -> Result<Self, Error> {
        Self::new(TargetHost::Ip(address), port)
    }

    pub fn domain(name: &str, port: u16) -> Result<Self, Error> {
        Self::new(TargetHost::Domain(domain(name)?), port)
    }

    pub(crate) fn new(host: TargetHost, port: u16) -> Result<Self, Error> {
        Ok(Self {
            host,
            port: NonZeroU16::new(port).ok_or(Error::InvalidPort)?,
        })
    }

    pub fn host(&self) -> &TargetHost {
        &self.host
    }
    pub fn port(&self) -> u16 {
        self.port.get()
    }
}

/// Server-reported address. Unlike a CONNECT target, BND.PORT may be zero and
/// BND.ADDR may be unspecified (as used by several upstream implementations).
/// This is metadata, not a verified/reachable endpoint or peer identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundAddress {
    pub(crate) host: TargetHost,
    pub(crate) port: u16,
}

impl BoundAddress {
    pub fn host(&self) -> &TargetHost {
        &self.host
    }
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl From<SocketAddr> for BoundAddress {
    fn from(address: SocketAddr) -> Self {
        Self {
            host: TargetHost::Ip(address.ip()),
            port: address.port(),
        }
    }
}

pub(crate) fn domain(name: &str) -> Result<String, Error> {
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.is_empty()
        || name.len() > 253
        || !name.is_ascii()
        || !name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(Error::InvalidDomain);
    }
    Ok(name.to_ascii_lowercase())
}

#[derive(Clone, Debug)]
pub struct ClientConfig {
    pub auth: Auth,
    /// Private Meshway destination reachable from an external proxy, or the
    /// logical fixed target of the embedded acceptor; not the proxy's address.
    pub target: Target,
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub auth: Auth,
    /// The only accepted destination, including address kind and port. No DNS
    /// resolution, wildcard matching, or outbound dialing occurs here.
    pub expected_target: Target,
    /// Address reported in the standard success reply. Use the accepted socket's
    /// local address, or an explicit unspecified address when it is unavailable.
    /// There is no outbound proxy socket: this adapter terminates the carrier.
    pub bound_address: SocketAddr,
}
