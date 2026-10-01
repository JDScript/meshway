//! Trojan TCP CONNECT over verified TLS: reliable, ordered, full-duplex bytes.
//!
//! The real wire format is `hex(SHA224(password)) CRLF CMD SOCKS_ADDR CRLF`.
//! Trojan has **no version byte and no server response header**. The client can
//! write Noise immediately after its request; successful client handshake means
//! only that the request was sent, not that the server accepted its password.
//!
//! Use [`tls::ClientTls`] / [`tls::ServerTls`] for deadline-bounded verified TLS.
//! The generic borrowed helpers require an **already TLS-protected stream** and
//! caller-enforced deadlines. On error, timeout or cancellation, close the stream;
//! a partially consumed handshake cannot be resumed or retried on that stream.
//!
//! Password authentication is not Meshway peer identity. Run the Meshway Noise
//! channel before business data, regardless of which node dialed TCP. This crate
//! never dials the requested destination and permits only one fixed server target.

mod config;
mod error;
pub mod tls;

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

pub use config::{ClientConfig, SecretPassword, ServerConfig, Target, TargetHost};
pub use error::Error;

const COMMAND_TCP: u8 = 1;
const ADDRESS_IPV4: u8 = 1;
const ADDRESS_DOMAIN: u8 = 3;
const ADDRESS_IPV6: u8 = 4;

/// Absolute wire bound with a 255-byte domain (DNS validation allows <=253).
/// No payload is read ahead or buffered by these helpers.
pub const MAX_REQUEST_HEADER_LEN: usize = 320;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportLabel {
    TrojanTcpTls,
}

pub const TRANSPORT_LABEL: TransportLabel = TransportLabel::TrojanTcpTls;

/// Validated carrier authorization and destination, not Meshway peer identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedRequest {
    pub target: Target,
}

/// Write and flush CONNECT on an already TLS-protected stream. There is no
/// response to wait for, so either side can write first after parsing the request.
/// The caller MUST impose a timeout and close the stream on error/cancellation.
pub async fn client_handshake<S>(stream: &mut S, config: &ClientConfig) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    let mut header = Zeroizing::new([0_u8; MAX_REQUEST_HEADER_LEN]);
    header[..56].copy_from_slice(&config.password.0[..]);
    header[56..58].copy_from_slice(b"\r\n");
    header[58] = COMMAND_TCP;
    let end = match config.target.host() {
        TargetHost::Ip(IpAddr::V4(address)) => {
            header[59] = ADDRESS_IPV4;
            header[60..64].copy_from_slice(&address.octets());
            64
        }
        TargetHost::Ip(IpAddr::V6(address)) => {
            header[59] = ADDRESS_IPV6;
            header[60..76].copy_from_slice(&address.octets());
            76
        }
        TargetHost::Domain(name) => {
            header[59] = ADDRESS_DOMAIN;
            header[60] = name.len() as u8; // validated <=253 ASCII bytes
            header[61..61 + name.len()].copy_from_slice(name.as_bytes());
            61 + name.len()
        }
    };
    header[end..end + 2].copy_from_slice(&config.target.port().to_be_bytes());
    header[end + 2..end + 4].copy_from_slice(b"\r\n");
    stream.write_all(&header[..end + 4]).await?;
    stream.flush().await?;
    Ok(())
}

/// Validate exactly one password and target on an already TLS-protected stream.
/// Password-derived bytes use constant-time comparison. UDP, mux, unknown SOCKS
/// address types, malformed delimiters and arbitrary targets fail closed. No
/// fallback, outbound connection or response header is emitted.
/// The caller MUST impose a timeout and close the stream on error/cancellation.
pub async fn server_handshake<S>(
    stream: &mut S,
    config: &ServerConfig,
) -> Result<ValidatedRequest, Error>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    let mut password = Zeroizing::new([0_u8; 56]);
    stream.read_exact(&mut password[..]).await?;
    // Comparing against a canonical lowercase hex value also rejects non-hex
    // and uppercase input, without branching on individual credential bytes.
    if !bool::from(password[..].ct_eq(&config.password.0[..])) {
        return Err(Error::AuthenticationFailed);
    }
    read_crlf(stream).await?;
    let command = stream.read_u8().await?;
    if command != COMMAND_TCP {
        return Err(Error::UnsupportedCommand(command));
    }
    let target = match stream.read_u8().await? {
        ADDRESS_IPV4 => {
            let mut bytes = [0; 4];
            stream.read_exact(&mut bytes).await?;
            Target::ip(Ipv4Addr::from(bytes).into(), stream.read_u16().await?)?
        }
        ADDRESS_IPV6 => {
            let mut bytes = [0; 16];
            stream.read_exact(&mut bytes).await?;
            Target::ip(Ipv6Addr::from(bytes).into(), stream.read_u16().await?)?
        }
        ADDRESS_DOMAIN => {
            let length = usize::from(stream.read_u8().await?);
            if length == 0 || length > 253 {
                return Err(Error::InvalidDomain);
            }
            let mut bytes = [0; 253];
            stream.read_exact(&mut bytes[..length]).await?;
            let name = std::str::from_utf8(&bytes[..length]).map_err(|_| Error::InvalidDomain)?;
            Target::domain(name, stream.read_u16().await?)?
        }
        kind => return Err(Error::UnsupportedAddressType(kind)),
    };
    read_crlf(stream).await?;
    if target != config.expected_target {
        return Err(Error::TargetMismatch);
    }
    Ok(ValidatedRequest { target })
}

async fn read_crlf<S: AsyncRead + Unpin + ?Sized>(stream: &mut S) -> Result<(), Error> {
    let mut bytes = [0; 2];
    stream.read_exact(&mut bytes).await?;
    if bytes != *b"\r\n" {
        return Err(Error::InvalidCrlf);
    }
    Ok(())
}
