//! Classic AEAD Shadowsocks TCP as a reliable, ordered full-duplex byte stream.
//!
//! Cryptography, KDF, salts, records, and replay filtering are provided by the
//! pinned `shadowsocks` backend. The shared password admits a carrier, **not a
//! Meshway identity**. Run the authenticated Meshway Noise channel above it.
//!
//! The caller owns dial/listen, connection limits and deadlines. Put construction
//! and Noise under a timeout. Drop the connection on errors or cancellation;
//! never retry a partially completed write/handshake. Config clones share the
//! backend's bounded in-memory replay filter. See the crate README for limits.

#![doc = include_str!("../README.md")]

mod config;
mod error;
mod stream;

pub use config::{Cipher, ClientConfig, SecretPassword, ServerConfig, Target, TargetHost};
pub use error::Error;
pub use stream::CarrierStream;

use shadowsocks::relay::tcprelay::{ProxyClientStream, ProxyServerStream};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

/// This is not a datagram carrier or an IP tunnel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportLabel {
    ShadowsocksTcp,
}

pub const TRANSPORT_LABEL: TransportLabel = TransportLabel::ShadowsocksTcp;

/// Write the encrypted target and flush, without waiting for a server reply.
/// External proxies can defer their response until Noise sends client-first data.
/// Success means only that the request was written, not that the peer accepted it.
///
/// Accepts an owned (or borrowed) generic duplex stream, including outer TLS.
/// Cancellation drops owned I/O; callers supplying a borrow must close it themselves.
pub async fn client_stream<S>(stream: S, config: &ClientConfig) -> Result<CarrierStream<S>, Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut stream = ProxyClientStream::from_stream(
        config.prepared.context.clone(),
        stream,
        &config.prepared.backend,
        config.target.address(),
    );
    // Upstream explicitly supports this header-only initial write. write_all([])
    // would skip poll_write and fail to transmit the target. Never await a reply.
    let written = stream.write(&[]).await?;
    debug_assert_eq!(written, 0);
    stream.flush().await?;
    Ok(CarrierStream::client(stream))
}

/// Decrypt/authenticate the request and require the exact configured target.
/// Never resolves or dials the destination. Any coalesced first payload remains
/// buffered in the returned stream. The acceptor can immediately write back.
pub async fn server_stream<S>(stream: S, config: &ServerConfig) -> Result<CarrierStream<S>, Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut stream = ProxyServerStream::from_stream(
        config.prepared.context.clone(),
        stream,
        config.prepared.backend.method(),
        config.prepared.backend.key(),
    );
    let target = Target::from_address(stream.handshake().await?)?;
    if target != config.expected_target {
        return Err(Error::TargetMismatch);
    }
    Ok(CarrierStream::server(stream))
}
