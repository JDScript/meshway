use std::{fmt, net::IpAddr, num::NonZeroU16, str::FromStr};

use zeroize::Zeroizing;

use crate::Error;

/// VLESS's shared bearer credential, **not** a Meshway node identity.
///
/// Parsing requires exactly `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` (ASCII hex,
/// either case). Arbitrary strings, UUID URNs, braces, and whitespace are rejected.
/// Debug output is redacted; this type deliberately has no Display or serialization.
#[derive(Clone)]
pub struct SecretUuid(pub(crate) Zeroizing<[u8; 16]>);

impl fmt::Debug for SecretUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretUuid([REDACTED])")
    }
}

impl FromStr for SecretUuid {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let input = value.as_bytes();
        if input.len() != 36 {
            return Err(Error::InvalidUuid);
        }
        let mut bytes = Zeroizing::new([0; 16]);
        let mut offset = 0;
        for (index, byte) in bytes.iter_mut().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                if input[offset] != b'-' {
                    return Err(Error::InvalidUuid);
                }
                offset += 1;
            }
            let high = hex_digit(input[offset]).ok_or(Error::InvalidUuid)?;
            let low = hex_digit(input[offset + 1]).ok_or(Error::InvalidUuid)?;
            *byte = high << 4 | low;
            offset += 2;
        }
        Ok(Self(bytes))
    }
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

/// Address kind is significant: a domain does not match an IP by resolving it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetHost {
    Ip(IpAddr),
    Domain(String),
}

/// A validated CONNECT destination with a nonzero port.
///
/// Domain names are ASCII DNS names (IDNs must already be A-labels). They are
/// lowercased and one trailing root dot is removed. No DNS lookup is performed.
/// IPv4 and IPv6 remain distinct, including IPv4-mapped IPv6 addresses.
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
    pub uuid: SecretUuid,
    /// For an external proxy, this must be the private/loopback Meshway endpoint
    /// reachable from that proxy. It is not the proxy's public listening address.
    pub target: Target,
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub uuid: SecretUuid,
    /// Exactly one allowed logical Meshway endpoint. There is no wildcard mode.
    /// The embedded adapter never dials this address or forwards to the Internet.
    pub expected_target: Target,
}
