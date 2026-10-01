//! Verified TLS 1.2/1.3 wrappers. No insecure verifier, 0-RTT, ALPN camouflage,
//! Reality or client-certificate identity is configured. Meshway Noise still runs
//! on the returned stream. Configuration holds rustls-owned private keys; callers
//! must load them from private local files and protect their original buffers.
//! Pass an owned I/O object to `connect`/`accept` so dropping it closes the socket.
//! If `S` is a borrowed I/O object, the caller must close it after any failure.

use std::{fmt, net::SocketAddr, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    time::timeout,
};
use tokio_rustls::{TlsAcceptor, TlsConnector, client, server};

use rustls::{
    RootCertStore,
    pki_types::{CertificateDer, PrivateKeyDer, ServerName},
};
pub use tokio_rustls::rustls;

use crate::{
    ClientConfig, Error, ServerConfig, ValidatedRequest, client_handshake, server_handshake,
};

/// Immutable connector: normal chain, time, signature and server-name validation.
/// The server name is required even when dialing a literal IP address.
#[derive(Clone)]
pub struct ClientTls {
    connector: TlsConnector,
    server_name: ServerName<'static>,
}

impl fmt::Debug for ClientTls {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ClientTls(verified)")
    }
}

impl ClientTls {
    pub fn new(server_name: &str, roots: RootCertStore) -> Result<Self, Error> {
        if roots.is_empty() {
            return Err(Error::EmptyTrustStore);
        }
        let server_name =
            ServerName::try_from(server_name.to_owned()).map_err(|_| Error::InvalidServerName)?;
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::TlsConfig)?
        .with_root_certificates(roots)
        .with_no_client_auth();
        Ok(Self {
            connector: TlsConnector::from(Arc::new(config)),
            server_name,
        })
    }

    /// One deadline covers TCP connect, verified TLS, and Trojan request flush.
    /// The address is pre-resolved, so caller-controlled DNS has its own deadline.
    pub async fn dial(
        &self,
        address: SocketAddr,
        config: &ClientConfig,
        deadline: Duration,
    ) -> Result<client::TlsStream<TcpStream>, Error> {
        timeout(deadline, async {
            let stream = TcpStream::connect(address).await?;
            self.connect_inner(stream, config).await
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    /// Wrap an owned connected stream with TLS and Trojan under one deadline.
    /// Cancellation drops `S` (which closes an owned connection). The returned stream directly
    /// supports AsyncRead/AsyncWrite, split, flush and TLS write-half shutdown.
    pub async fn connect<S>(
        &self,
        stream: S,
        config: &ClientConfig,
        deadline: Duration,
    ) -> Result<client::TlsStream<S>, Error>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        timeout(deadline, self.connect_inner(stream, config))
            .await
            .map_err(|_| Error::Timeout)?
    }

    async fn connect_inner<S>(
        &self,
        stream: S,
        config: &ClientConfig,
    ) -> Result<client::TlsStream<S>, Error>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let mut stream = self
            .connector
            .connect(self.server_name.clone(), stream)
            .await
            .map_err(|error| Error::TlsHandshake(error.kind()))?;
        client_handshake(&mut stream, config).await?;
        Ok(stream)
    }
}

#[derive(Clone)]
pub struct ServerTls {
    acceptor: TlsAcceptor,
}

impl fmt::Debug for ServerTls {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServerTls([REDACTED])")
    }
}

impl ServerTls {
    /// Certificate chain is leaf first. rustls checks the key matches the leaf.
    pub fn new(
        certificates: Vec<CertificateDer<'static>>,
        private_key: PrivateKeyDer<'static>,
    ) -> Result<Self, Error> {
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::TlsConfig)?
        .with_no_client_auth()
        .with_single_cert(certificates, private_key)
        .map_err(|_| Error::TlsConfig)?;
        Ok(Self {
            acceptor: TlsAcceptor::from(Arc::new(config)),
        })
    }

    /// One deadline covers TLS accept and complete Trojan authorization. On any
    /// failure `S` is dropped, with no fallback or outbound dialing.
    pub async fn accept<S>(
        &self,
        stream: S,
        config: &ServerConfig,
        deadline: Duration,
    ) -> Result<(server::TlsStream<S>, ValidatedRequest), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        timeout(deadline, async {
            let mut stream = self
                .acceptor
                .accept(stream)
                .await
                .map_err(|error| Error::TlsHandshake(error.kind()))?;
            let request = server_handshake(&mut stream, config).await?;
            Ok((stream, request))
        })
        .await
        .map_err(|_| Error::Timeout)?
    }
}
