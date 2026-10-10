use std::time::Duration;

use meshway_transport::spec::{PositiveDuration, TlsVersion};
use rustls::SupportedProtocolVersion;
use tokio::time::Instant;

use crate::tls::Error;

// Bound the entire encoded ALPN list, leaving ample room for the remaining
// ClientHello extensions within rustls' handshake-message limits.
const MAX_ALPN_BYTES: usize = 16 * 1024;
static TLS12: [&SupportedProtocolVersion; 1] = [&rustls::version::TLS12];
static TLS13: [&SupportedProtocolVersion; 1] = [&rustls::version::TLS13];
static TLS_BOTH: [&SupportedProtocolVersion; 2] =
    [&rustls::version::TLS13, &rustls::version::TLS12];

pub(crate) struct Options {
    pub(crate) versions: &'static [&'static SupportedProtocolVersion],
    pub(crate) alpn: Vec<Vec<u8>>,
    pub(crate) timeout: Duration,
}

impl Options {
    pub(crate) fn new(
        min: TlsVersion,
        max: TlsVersion,
        alpn: &[String],
        timeout: &PositiveDuration,
    ) -> Result<Self, Error> {
        let versions: &'static [&'static SupportedProtocolVersion] = match (min, max) {
            (TlsVersion::Tls12, TlsVersion::Tls12) => &TLS12,
            (TlsVersion::Tls13, TlsVersion::Tls13) => &TLS13,
            (TlsVersion::Tls12, TlsVersion::Tls13) => &TLS_BOTH,
            (TlsVersion::Tls13, TlsVersion::Tls12) => return Err(Error::VersionRange),
        };
        let mut bytes = 0usize;
        for protocol in alpn {
            if protocol.is_empty() || protocol.len() > 255 || protocol.chars().any(char::is_control)
            {
                return Err(Error::Alpn);
            }
            bytes = bytes.checked_add(protocol.len() + 1).ok_or(Error::Alpn)?;
            if bytes > MAX_ALPN_BYTES {
                return Err(Error::Alpn);
            }
        }
        let timeout = timeout.as_std();
        deadline(timeout)?;
        Ok(Self {
            versions,
            alpn: alpn.iter().map(|value| value.as_bytes().to_vec()).collect(),
            timeout,
        })
    }
}

pub(crate) fn deadline(timeout: Duration) -> Result<Instant, Error> {
    // Keep headroom for Tokio's runtime-relative millisecond clock and its
    // reserved tick values, then check the platform's tighter Instant limit.
    if timeout.is_zero() || timeout.as_millis() > i64::MAX as u128 {
        return Err(Error::TimeoutRange);
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(Error::TimeoutRange)?;
    // Tokio rounds deadlines up to a millisecond internally.
    deadline
        .checked_add(Duration::from_millis(1))
        .ok_or(Error::TimeoutRange)?;
    Ok(deadline)
}
