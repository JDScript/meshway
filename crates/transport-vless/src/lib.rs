//! VLESS v0 TCP CONNECT as a reliable, ordered, full-duplex byte-stream carrier.
//!
//! This is the real VLESS wire format, with empty addons and no flow. VLESS does
//! not encrypt the stream or authenticate a Meshway identity: the caller must
//! run the Meshway Noise channel before exchanging business payloads. The caller
//! owns dialing/listening, optional outer TLS, deadlines, and connection limits.
//!
//! [`server_handshake`] accepts only the configured target and performs no proxy
//! dialing. [`client_handshake`] is suitable for this eager-response server.
//! External Xray/sing-box servers may defer their response until the destination
//! writes: use [`client_stream`] so Noise can write first, or the split
//! [`write_client_request`]/[`read_server_response`] functions.
//!
//! **Every handshake (including lazy response reading and Noise) must have a
//! caller-enforced timeout.** On error, timeout, or cancellation, close the stream;
//! do not retry a partly consumed handshake on that connection.

mod config;
mod error;
mod stream;
pub mod websocket;

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

pub use config::{ClientConfig, SecretUuid, ServerConfig, Target, TargetHost};
pub use error::Error;
pub use stream::{ClientStream, client_stream};

const VERSION: u8 = 0;
const COMMAND_TCP: u8 = 1;
const ADDRESS_IPV4: u8 = 1;
const ADDRESS_DOMAIN: u8 = 2;
const ADDRESS_IPV6: u8 = 3;

/// Absolute wire-format bound with empty addons and a 255-byte domain.
/// Validated DNS names are at most 253 bytes, but inbound parsing is bounded by
/// the wire length before DNS validation. No payload bytes are read ahead.
pub const MAX_REQUEST_HEADER_LEN: usize = 278;
pub const RESPONSE_HEADER_LEN: usize = 2;

/// Transport metadata; this carrier does not provide unreliable datagrams.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportLabel {
    VlessTcp,
}

pub const TRANSPORT_LABEL: TransportLabel = TransportLabel::VlessTcp;

/// Validated VLESS authorization and destination, not a Meshway peer identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedRequest {
    pub target: Target,
}

/// Write and flush a real VLESS v0 TCP CONNECT request without waiting for its
/// response. No payload is buffered. Follow with `read_server_response` before
/// reading application bytes, or use `client_stream` to do that automatically.
pub async fn write_client_request<S>(stream: &mut S, config: &ClientConfig) -> Result<(), Error>
where
    S: AsyncWrite + Unpin + ?Sized,
{
    let mut header = Zeroizing::new([0_u8; MAX_REQUEST_HEADER_LEN]);
    header[0] = VERSION;
    header[1..17].copy_from_slice(&config.uuid.0[..]);
    header[17] = 0; // addons length: no flow, seed, or other extension
    header[18] = COMMAND_TCP;
    header[19..21].copy_from_slice(&config.target.port().to_be_bytes());
    let length = match config.target.host() {
        TargetHost::Ip(IpAddr::V4(address)) => {
            header[21] = ADDRESS_IPV4;
            header[22..26].copy_from_slice(&address.octets());
            26
        }
        TargetHost::Ip(IpAddr::V6(address)) => {
            header[21] = ADDRESS_IPV6;
            header[22..38].copy_from_slice(&address.octets());
            38
        }
        TargetHost::Domain(name) => {
            header[21] = ADDRESS_DOMAIN;
            // Target's constructor guarantees an ASCII DNS name of <=253 bytes.
            header[22] = name.len() as u8;
            header[23..23 + name.len()].copy_from_slice(name.as_bytes());
            23 + name.len()
        }
    };
    stream.write_all(&header[..length]).await?;
    stream.flush().await?;
    Ok(())
}

/// Consume exactly the two-byte, empty-addon VLESS response.
///
/// Nonzero addon lengths are rejected immediately (including malformed addons),
/// rather than skipped or interpreted as supported features. The stream must be
/// closed after any error; rejected extension bytes are deliberately not drained.
pub async fn read_server_response<S>(stream: &mut S) -> Result<(), Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let version = stream.read_u8().await?;
    check_version(version)?;
    let addons_length = stream.read_u8().await?;
    check_addons(addons_length)
}

/// Complete the request/response exchange, leaving `stream` at raw payload bytes.
///
/// Use `client_stream` for external proxies whose response is deferred until the
/// private destination writes; eagerly waiting here can deadlock client-first Noise.
pub async fn client_handshake<S>(stream: &mut S, config: &ClientConfig) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    write_client_request(stream, config).await?;
    read_server_response(stream).await
}

/// Validate one configured UUID and exact target, then eagerly acknowledge.
///
/// Authentication uses constant-time UUID comparison. This accepts only TCP
/// CONNECT with empty addons. UDP, mux, Vision, and arbitrary destinations fail
/// closed. It never opens an outbound connection; the returned borrowed stream
/// is the Meshway carrier in both directions, regardless of who dialed TCP.
pub async fn server_handshake<S>(
    stream: &mut S,
    config: &ServerConfig,
) -> Result<ValidatedRequest, Error>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    let version = stream.read_u8().await?;
    check_version(version)?;
    let mut uuid = Zeroizing::new([0_u8; 16]);
    stream.read_exact(&mut uuid[..]).await?;
    if !bool::from(uuid[..].ct_eq(&config.uuid.0[..])) {
        return Err(Error::AuthenticationFailed);
    }
    let addons_length = stream.read_u8().await?;
    check_addons(addons_length)?;
    let command = stream.read_u8().await?;
    if command != COMMAND_TCP {
        return Err(Error::UnsupportedCommand(command));
    }
    let port = stream.read_u16().await?;
    if port == 0 {
        return Err(Error::InvalidPort);
    }
    let address_type = stream.read_u8().await?;
    let target = match address_type {
        ADDRESS_IPV4 => {
            let mut bytes = [0; 4];
            stream.read_exact(&mut bytes).await?;
            Target::ip(Ipv4Addr::from(bytes).into(), port)?
        }
        ADDRESS_IPV6 => {
            let mut bytes = [0; 16];
            stream.read_exact(&mut bytes).await?;
            Target::ip(Ipv6Addr::from(bytes).into(), port)?
        }
        ADDRESS_DOMAIN => {
            let length = usize::from(stream.read_u8().await?);
            let mut bytes = [0; 255];
            stream.read_exact(&mut bytes[..length]).await?;
            let name = std::str::from_utf8(&bytes[..length]).map_err(|_| Error::InvalidDomain)?;
            Target::domain(name, port)?
        }
        kind => return Err(Error::UnsupportedAddressType(kind)),
    };
    if target != config.expected_target {
        return Err(Error::TargetMismatch);
    }
    stream.write_all(&[VERSION, 0]).await?;
    stream.flush().await?;
    Ok(ValidatedRequest { target })
}

fn check_version(version: u8) -> Result<(), Error> {
    if version == VERSION {
        Ok(())
    } else {
        Err(Error::UnsupportedVersion(version))
    }
}

fn check_addons(length: u8) -> Result<(), Error> {
    if length == 0 {
        Ok(())
    } else {
        Err(Error::UnsupportedAddons(length))
    }
}
