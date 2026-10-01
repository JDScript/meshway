use std::{fmt, net::IpAddr, num::NonZeroU16};

use sha2::{Digest, Sha224};
use zeroize::Zeroizing;

use crate::Error;

/// Trojan's password-derived bearer credential, **not** a Meshway identity.
///
/// Stores only lowercase SHA-224 hex; this is itself a secret. No Display or
/// serialization is provided. The caller owns and must protect the source bytes.
#[derive(Clone)]
pub struct SecretPassword(pub(crate) Zeroizing<[u8; 56]>);

impl SecretPassword {
    /// Hash exact password bytes, without whitespace or Unicode normalization.
    /// Local configuration rejects empty passwords and inputs over 4096 bytes.
    pub fn new(password: &[u8]) -> Result<Self, Error> {
        if password.is_empty() || password.len() > 4096 {
            return Err(Error::InvalidPassword);
        }
        let digest = Zeroizing::new(<[u8; 28]>::from(Sha224::digest(password)));
        let mut hex = Zeroizing::new([0; 56]);
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        for (index, byte) in digest.iter().enumerate() {
            hex[index * 2] = DIGITS[usize::from(byte >> 4)];
            hex[index * 2 + 1] = DIGITS[usize::from(byte & 15)];
        }
        Ok(Self(hex))
    }
}

impl fmt::Debug for SecretPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretPassword([REDACTED])")
    }
}

/// Address kind is significant: domains are never resolved to compare targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetHost {
    Ip(IpAddr),
    Domain(String),
}

/// Exact CONNECT destination, with nonzero port. ASCII DNS names are lowercased
/// and one trailing root dot is removed. IDNs must already be A-labels. IPv4 and
/// IPv6 remain distinct, including IPv4-mapped IPv6 addresses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    host: TargetHost,
    port: NonZeroU16,
}

impl Target {
    pub fn ip(address: IpAddr, port: u16) -> Result<Self, Error> {
        Ok(Self {
            host: TargetHost::Ip(address),
            port: NonZeroU16::new(port).ok_or(Error::InvalidPort)?,
        })
    }

    pub fn domain(name: &str, port: u16) -> Result<Self, Error> {
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
        Ok(Self {
            host: TargetHost::Domain(name.to_ascii_lowercase()),
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

#[derive(Clone, Debug)]
pub struct ClientConfig {
    pub password: SecretPassword,
    /// For external proxies, the private Meshway endpoint reachable from the
    /// proxy, rather than the proxy's public listener address.
    pub target: Target,
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub password: SecretPassword,
    /// Exactly one logical endpoint. No wildcard, DNS lookup or outbound dial.
    pub expected_target: Target,
}
