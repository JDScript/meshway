use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    num::NonZeroU16,
    str::FromStr,
    sync::Arc,
};

use shadowsocks::{
    config::{ReplayAttackPolicy, ServerConfig as BackendConfig, ServerType},
    context::{Context, SharedContext},
    crypto::CipherKind,
    relay::socks5::Address,
};
use zeroize::Zeroizing;

use crate::Error;

/// The complete supported allowlist. There is no fallback or alias parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cipher {
    Chacha20IetfPoly1305,
    Aes128Gcm,
    Aes256Gcm,
}

impl Cipher {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chacha20IetfPoly1305 => "chacha20-ietf-poly1305",
            Self::Aes128Gcm => "aes-128-gcm",
            Self::Aes256Gcm => "aes-256-gcm",
        }
    }

    pub(crate) fn backend(self) -> CipherKind {
        match self {
            Self::Chacha20IetfPoly1305 => CipherKind::CHACHA20_POLY1305,
            Self::Aes128Gcm => CipherKind::AES_128_GCM,
            Self::Aes256Gcm => CipherKind::AES_256_GCM,
        }
    }
}

impl FromStr for Cipher {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "chacha20-ietf-poly1305" => Ok(Self::Chacha20IetfPoly1305),
            "aes-128-gcm" => Ok(Self::Aes128Gcm),
            "aes-256-gcm" => Ok(Self::Aes256Gcm),
            _ => Err(Error::UnsupportedCipher),
        }
    }
}

/// Carrier admission secret, not a Meshway node identity. No Display/serialization.
/// This buffer is zeroized; upstream's internal password/key copies are not
/// guaranteed to be zeroized. The caller owns its original input string.
#[derive(Clone)]
pub struct SecretPassword(Zeroizing<String>);

impl SecretPassword {
    pub fn new(value: String) -> Result<Self, Error> {
        let value = Zeroizing::new(value);
        if value.is_empty() || value.len() > 1024 {
            return Err(Error::InvalidPassword);
        }
        Ok(Self(value))
    }
}

impl FromStr for SecretPassword {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value.to_owned())
    }
}

impl fmt::Debug for SecretPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretPassword([REDACTED])")
    }
}

/// Address kind is significant. Domains never resolve during validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetHost {
    Ip(IpAddr),
    Domain(String),
}

/// One exact endpoint: nonzero port, IPv4/IPv6 or bounded ASCII DNS labels.
/// Domains are lowercased and one terminal root dot is removed. No wildcards.
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

    pub(crate) fn address(&self) -> Address {
        match &self.host {
            TargetHost::Ip(ip) => Address::SocketAddress(SocketAddr::new(*ip, self.port())),
            TargetHost::Domain(name) => Address::DomainNameAddress(name.clone(), self.port()),
        }
    }

    pub(crate) fn from_address(address: Address) -> Result<Self, Error> {
        match address {
            Address::SocketAddress(address) => Self::ip(address.ip(), address.port()),
            Address::DomainNameAddress(name, port) => Self::domain(&name, port),
        }
    }
}

pub(crate) struct Prepared {
    pub backend: BackendConfig,
    pub context: SharedContext,
}

impl Prepared {
    fn new(cipher: Cipher, password: SecretPassword, role: ServerType) -> Result<Arc<Self>, Error> {
        // Upstream requires a server address even for from_stream; this unused
        // loopback placeholder is never dialed, resolved, or exposed.
        let backend = BackendConfig::new(
            SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 1),
            password.0.as_str(),
            cipher.backend(),
        )
        .map_err(|_| Error::InvalidConfiguration)?;
        let mut context = Context::new(role);
        // Upstream's default accepts classic-AEAD replays even with its feature
        // enabled. Both the compiled filter AND explicit Reject are necessary.
        context.set_replay_attack_policy(ReplayAttackPolicy::Reject);
        Ok(Arc::new(Self {
            backend,
            context: Arc::new(context),
        }))
    }
}

/// Reuse or clone this config across connections to preserve response replay state.
/// The target is the private Meshway endpoint as seen by an external proxy.
#[derive(Clone)]
pub struct ClientConfig {
    pub(crate) prepared: Arc<Prepared>,
    pub(crate) target: Target,
    cipher: Cipher,
}

impl ClientConfig {
    pub fn new(cipher: Cipher, password: SecretPassword, target: Target) -> Result<Self, Error> {
        Ok(Self {
            prepared: Prepared::new(cipher, password, ServerType::Local)?,
            target,
            cipher,
        })
    }

    pub fn target(&self) -> &Target {
        &self.target
    }
}

impl fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientConfig")
            .field("cipher", &self.cipher)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// Reuse or clone this config for the listener lifetime: it owns the shared
/// request replay filter. Construction creates a new, empty filter.
#[derive(Clone)]
pub struct ServerConfig {
    pub(crate) prepared: Arc<Prepared>,
    pub(crate) expected_target: Target,
    cipher: Cipher,
}

impl ServerConfig {
    pub fn new(
        cipher: Cipher,
        password: SecretPassword,
        expected_target: Target,
    ) -> Result<Self, Error> {
        Ok(Self {
            prepared: Prepared::new(cipher, password, ServerType::Server)?,
            expected_target,
            cipher,
        })
    }

    pub fn expected_target(&self) -> &Target {
        &self.expected_target
    }
}

impl fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerConfig")
            .field("cipher", &self.cipher)
            .field("expected_target", &self.expected_target)
            .finish_non_exhaustive()
    }
}
