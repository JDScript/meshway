use std::{sync::Arc, time::Duration};

use meshway_transport::spec::ListenerTls;
use rustls::{ServerConfig, sign::SingleCertAndKey};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::{TlsAcceptor, TlsStream};

use crate::tls::{
    Error, material,
    options::{self, Options},
};

/// Cached server configuration backed by cryptographically verified material.
#[derive(Clone)]
pub struct Server {
    acceptor: TlsAcceptor,
    timeout: Duration,
}

impl Server {
    /// Verify material using the same implementation as [`crate::tls::verify_listener`].
    /// ALPN entries retain their order and are limited to 255 bytes each and
    /// 16 KiB in total (including each entry's one-byte length).
    pub fn new(tls: &ListenerTls) -> Result<Self, Error> {
        let options = Options::new(
            tls.min_version,
            tls.max_version,
            &tls.alpn,
            &tls.handshake_timeout,
        )?;
        let provider = material::provider();
        let certified = material::listener(tls, provider.clone())?;
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(options.versions)
            .map_err(|_| Error::Configuration)?
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(SingleCertAndKey::from(certified)));
        config.alpn_protocols = options.alpn;
        // Exact material is checked for every connection, on both sides. Do
        // not retain sessions or queue unsolicited tickets ahead of protocol
        // payload on a small/backpressured injected stream.
        config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
        config.send_tls13_tickets = 0;
        Ok(Self {
            acceptor: TlsAcceptor::from(Arc::new(config)),
            timeout: options.timeout,
        })
    }

    /// Negotiate TLS over an accepted stream. The caller's overall timeout or
    /// remaining lease must enclose this handshake's independent local cap.
    pub async fn accept<S>(&self, stream: S) -> Result<TlsStream<S>, Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        tokio::time::timeout_at(
            options::deadline(self.timeout)?,
            self.acceptor.accept(stream),
        )
        .await
        .map_err(|_| Error::Timeout)?
        .map(Into::into)
        .map_err(|_| Error::Handshake)
    }
}
