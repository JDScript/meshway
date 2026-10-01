//! SOCKS5 TCP CONNECT (RFC 1928/1929) as an ordered, reliable, full-duplex carrier.
//!
//! Proxy authentication does **not** authenticate a Meshway node. Raw SOCKS5
//! encrypts neither credentials nor payload. Callers must establish Meshway Noise
//! over the returned stream before business payloads; outer TLS is caller-explicit.
//! Dialing/listening, connection limits, and deadlines belong to the caller.
//!
//! The embedded server accepts exactly one configured target and never opens an
//! outbound connection. Both handshakes consume only their wire prefix, leaving
//! payload bytes untouched. The acceptor can write first after CONNECT.
//!
//! **Enforce a timeout on every handshake (including Noise).** On any error,
//! timeout, or cancellation, close the stream immediately; a partially consumed
//! borrowed handshake cannot be resumed/retried. Cancelling an owned constructor
//! drops its owned stream. Established streams forward half-close and cancellation
//! semantics directly to the underlying I/O, without background tasks or buffering.

mod config;
mod error;
mod stream;

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

pub use config::{Auth, BoundAddress, ClientConfig, Credentials, ServerConfig, Target, TargetHost};
pub use error::{Error, ReplyCode};
pub use stream::{Socks5Stream, client_stream, server_stream};

const VERSION: u8 = 5;
const AUTH_VERSION: u8 = 1;
const CONNECT: u8 = 1;
const IPV4: u8 = 1;
const DOMAIN: u8 = 3;
const IPV6: u8 = 4;

/// Maximum wire length of a CONNECT request/reply (including a 255-byte name).
pub const MAX_CONNECT_HEADER_LEN: usize = 262;
/// Maximum RFC 1929 request size, including both 255-byte credentials.
pub const MAX_AUTH_HEADER_LEN: usize = 513;
/// Maximum incoming method negotiation size. The client always sends 3 bytes.
pub const MAX_GREETING_LEN: usize = 257;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportLabel {
    Socks5Tcp,
}
pub const TRANSPORT_LABEL: TransportLabel = TransportLabel::Socks5Tcp;

/// Validated proxy-layer authentication and target, not Meshway peer identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedRequest {
    pub target: Target,
}

/// Negotiate exactly the configured authentication method, then TCP CONNECT.
/// Returns only after validating a success reply and its complete bound address.
pub async fn client_handshake<S>(
    stream: &mut S,
    config: &ClientConfig,
) -> Result<BoundAddress, Error>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    let method = config.auth.method();
    write_flush(stream, &[VERSION, 1, method]).await?;
    check_version(stream.read_u8().await?)?;
    let selected = stream.read_u8().await?;
    if selected == 0xff {
        return Err(Error::NoAcceptableMethod);
    }
    if selected != method {
        return Err(Error::UnofferedMethod(selected));
    }
    if let Auth::UsernamePassword(credentials) = &config.auth {
        let mut header = Zeroizing::new([0; MAX_AUTH_HEADER_LEN]);
        header[0] = AUTH_VERSION;
        let username_len = usize::from(credentials.username[0]) + 1;
        let password_len = usize::from(credentials.password[0]) + 1;
        header[1..1 + username_len].copy_from_slice(&credentials.username[..username_len]);
        header[1 + username_len..1 + username_len + password_len]
            .copy_from_slice(&credentials.password[..password_len]);
        write_flush(stream, &header[..1 + username_len + password_len]).await?;
        check_auth_version(stream.read_u8().await?)?;
        if stream.read_u8().await? != 0 {
            return Err(Error::AuthenticationFailed);
        }
    }
    write_message(stream, CONNECT, config.target.host(), config.target.port()).await?;
    check_version(stream.read_u8().await?)?;
    let reply = stream.read_u8().await?;
    check_reserved(stream.read_u8().await?)?;
    // Failure replies terminate the connection; do not wait for/drain rejected
    // address fields. A successful response is consumed in full, without read-ahead.
    if reply != 0 {
        return Err(Error::Reply(ReplyCode::try_from(reply)?));
    }
    read_address(stream).await
}

/// Require the configured method and exact target, then eagerly acknowledge.
/// Rejects BIND, UDP ASSOCIATE, unknown methods and arbitrary destinations.
/// Protocol rejection replies are flushed before returning; caller must close
/// immediately after any error, as required by RFC 1928/1929.
pub async fn server_handshake<S>(
    stream: &mut S,
    config: &ServerConfig,
) -> Result<ValidatedRequest, Error>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    check_version(stream.read_u8().await?)?;
    let length = usize::from(stream.read_u8().await?);
    if length == 0 {
        write_flush(stream, &[VERSION, 0xff]).await?;
        return Err(Error::EmptyMethods);
    }
    let mut methods = [0; 255];
    stream.read_exact(&mut methods[..length]).await?;
    let method = config.auth.method();
    if !methods[..length].contains(&method) {
        write_flush(stream, &[VERSION, 0xff]).await?;
        return Err(Error::NoAcceptableMethod);
    }
    write_flush(stream, &[VERSION, method]).await?;
    if let Auth::UsernamePassword(expected) = &config.auth {
        let result = read_credentials(stream).await;
        match result {
            Ok(credentials) if credentials.matches(expected) => {
                write_flush(stream, &[AUTH_VERSION, 0]).await?;
            }
            result => {
                write_flush(stream, &[AUTH_VERSION, 1]).await?;
                return Err(result.err().unwrap_or(Error::AuthenticationFailed));
            }
        }
    }
    let request = read_request(stream).await;
    let target = match request {
        Ok(target) if target == config.expected_target => target,
        result => {
            let error = result.err().unwrap_or(Error::TargetMismatch);
            let reply = match error {
                Error::UnsupportedCommand(_) => ReplyCode::CommandNotSupported,
                Error::UnsupportedAddressType(_) | Error::InvalidDomain => {
                    ReplyCode::AddressTypeNotSupported
                }
                Error::TargetMismatch => ReplyCode::ConnectionNotAllowed,
                Error::Io(_) => return Err(error),
                _ => ReplyCode::GeneralFailure,
            };
            write_message(
                stream,
                reply as u8,
                &TargetHost::Ip(Ipv4Addr::UNSPECIFIED.into()),
                0,
            )
            .await?;
            return Err(error);
        }
    };
    write_message(
        stream,
        0,
        &TargetHost::Ip(config.bound_address.ip()),
        config.bound_address.port(),
    )
    .await?;
    Ok(ValidatedRequest { target })
}

async fn read_credentials<S: AsyncRead + Unpin + ?Sized>(
    stream: &mut S,
) -> Result<Credentials, Error> {
    check_auth_version(stream.read_u8().await?)?;
    let mut credentials = Credentials::empty();
    credentials.username[0] = stream.read_u8().await?;
    let username_len = usize::from(credentials.username[0]);
    if username_len == 0 {
        return Err(Error::InvalidCredentialLength);
    }
    stream
        .read_exact(&mut credentials.username[1..=username_len])
        .await?;
    credentials.password[0] = stream.read_u8().await?;
    let password_len = usize::from(credentials.password[0]);
    if password_len == 0 {
        return Err(Error::InvalidCredentialLength);
    }
    stream
        .read_exact(&mut credentials.password[1..=password_len])
        .await?;
    Ok(credentials)
}

async fn read_request<S: AsyncRead + Unpin + ?Sized>(stream: &mut S) -> Result<Target, Error> {
    check_version(stream.read_u8().await?)?;
    let command = stream.read_u8().await?;
    if command != CONNECT {
        return Err(Error::UnsupportedCommand(command));
    }
    check_reserved(stream.read_u8().await?)?;
    let address = read_address(stream).await?;
    Target::new(address.host, address.port)
}

async fn read_address<S: AsyncRead + Unpin + ?Sized>(
    stream: &mut S,
) -> Result<BoundAddress, Error> {
    let host = match stream.read_u8().await? {
        IPV4 => {
            let mut bytes = [0; 4];
            stream.read_exact(&mut bytes).await?;
            TargetHost::Ip(Ipv4Addr::from(bytes).into())
        }
        IPV6 => {
            let mut bytes = [0; 16];
            stream.read_exact(&mut bytes).await?;
            TargetHost::Ip(Ipv6Addr::from(bytes).into())
        }
        DOMAIN => {
            let length = usize::from(stream.read_u8().await?);
            let mut bytes = [0; 255];
            stream.read_exact(&mut bytes[..length]).await?;
            let name = std::str::from_utf8(&bytes[..length]).map_err(|_| Error::InvalidDomain)?;
            TargetHost::Domain(config::domain(name)?)
        }
        kind => return Err(Error::UnsupportedAddressType(kind)),
    };
    Ok(BoundAddress {
        host,
        port: stream.read_u16().await?,
    })
}

async fn write_message<S: AsyncWrite + Unpin + ?Sized>(
    stream: &mut S,
    code: u8,
    host: &TargetHost,
    port: u16,
) -> Result<(), Error> {
    let mut header = [0; MAX_CONNECT_HEADER_LEN];
    header[..3].copy_from_slice(&[VERSION, code, 0]);
    let end = match host {
        TargetHost::Ip(IpAddr::V4(address)) => {
            header[3] = IPV4;
            header[4..8].copy_from_slice(&address.octets());
            8
        }
        TargetHost::Ip(IpAddr::V6(address)) => {
            header[3] = IPV6;
            header[4..20].copy_from_slice(&address.octets());
            20
        }
        TargetHost::Domain(name) => {
            header[3] = DOMAIN;
            header[4] = name.len() as u8;
            header[5..5 + name.len()].copy_from_slice(name.as_bytes());
            5 + name.len()
        }
    };
    header[end..end + 2].copy_from_slice(&port.to_be_bytes());
    write_flush(stream, &header[..end + 2]).await
}

async fn write_flush<S: AsyncWrite + Unpin + ?Sized>(
    stream: &mut S,
    bytes: &[u8],
) -> Result<(), Error> {
    stream.write_all(bytes).await?;
    stream.flush().await?;
    Ok(())
}

fn check_version(version: u8) -> Result<(), Error> {
    if version == VERSION {
        Ok(())
    } else {
        Err(Error::UnsupportedVersion(version))
    }
}
fn check_auth_version(version: u8) -> Result<(), Error> {
    if version == AUTH_VERSION {
        Ok(())
    } else {
        Err(Error::UnsupportedAuthVersion(version))
    }
}
fn check_reserved(value: u8) -> Result<(), Error> {
    if value == 0 {
        Ok(())
    } else {
        Err(Error::InvalidReserved(value))
    }
}
