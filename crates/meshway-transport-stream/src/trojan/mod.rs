//! Standard Trojan TCP CONNECT over TLS on a host-opened stream.
//!
//! The destination is protocol metadata, not routing authority or Mesh peer
//! authentication. The host chooses and authorizes its service destination.
mod protocol;

use std::{fmt, io, time::Duration};

use meshway_transport::raw::{CheckedOutput, Stream};
use meshway_transport::spec::{DialParameters, Endpoint, Listener, PreparedListener, TrojanStream};
use tokio::{io::AsyncWriteExt, time::Instant};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsupported or invalid Trojan configuration")]
    Configuration,
    #[error("Trojan credential rejected")]
    Credential,
    #[error("invalid or unsupported Trojan handshake")]
    Protocol,
    #[error("Trojan handshake timed out")]
    Timeout,
    #[error("Trojan TLS failed")]
    Tls(#[from] crate::tls::Error),
    #[error("Trojan stream I/O failed")]
    Io(io::ErrorKind),
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

pub struct Connector {
    tls: crate::tls::Client,
    key: Zeroizing<[u8; 56]>,
    timeout: Duration,
}

pub struct Acceptor {
    tls: crate::tls::Server,
    key: Zeroizing<[u8; 56]>,
    timeout: Duration,
}

/// A validated CONNECT request. No arbitrary outbound routing is performed.
pub struct Accepted {
    pub stream: Box<dyn Stream>,
    pub destination: Endpoint,
}

impl Connector {
    pub fn new(parameters: &DialParameters) -> Result<Self, Error> {
        let DialParameters::Trojan {
            options,
            password,
            tls,
        } = parameters
        else {
            return Err(Error::Configuration);
        };
        if !options.enabled || options.tls != tls.options {
            return Err(Error::Configuration);
        }
        let timeout = options.connect_timeout.as_std();
        deadline(timeout)?;
        Ok(Self {
            key: protocol::key(password.expose())?,
            tls: crate::tls::Client::new(tls)?,
            timeout,
        })
    }

    /// Own the stream throughout TLS and CONNECT; cancellation drops it.
    /// The caller's total connection/lease deadline must enclose this local cap.
    pub async fn connect<S: Stream>(
        &self,
        io: S,
        destination: &Endpoint,
    ) -> Result<Box<dyn Stream>, Error> {
        tokio::time::timeout_at(deadline(self.timeout)?, async {
            let request = protocol::request(&self.key, destination)?;
            let mut stream = self.tls.connect(CheckedOutput::new(io)).await?;
            stream.write_all(&request).await?;
            stream.flush().await?;
            Ok(Box::new(stream) as Box<dyn Stream>)
        })
        .await
        .map_err(|_| Error::Timeout)?
    }
}

impl Acceptor {
    pub fn new(listener: &PreparedListener) -> Result<Self, Error> {
        let Listener::Trojan(options) = listener.declaration() else {
            return Err(Error::Configuration);
        };
        if !options.enabled || options.stream != TrojanStream::Tcp {
            return Err(Error::Configuration);
        }
        let timeout = options.tls.handshake_timeout.as_std();
        deadline(timeout)?;
        Ok(Self {
            tls: crate::tls::Server::new(&options.tls)?,
            key: protocol::key(
                options
                    .password
                    .as_ref()
                    .ok_or(Error::Configuration)?
                    .expose(),
            )?,
            timeout,
        })
    }

    /// Accept TLS and one CONNECT request under the listener handshake cap.
    pub async fn accept<S: Stream>(&self, io: S) -> Result<Accepted, Error> {
        tokio::time::timeout_at(deadline(self.timeout)?, async {
            let mut stream = self.tls.accept(CheckedOutput::new(io)).await?;
            let destination = protocol::accept(&mut stream, &self.key).await?;
            Ok(Accepted {
                stream: Box::new(stream),
                destination,
            })
        })
        .await
        .map_err(|_| Error::Timeout)?
    }
}

fn deadline(timeout: Duration) -> Result<Instant, Error> {
    Instant::now()
        .checked_add(timeout)
        .ok_or(Error::Configuration)
}

impl fmt::Debug for Connector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TrojanConnector { .. }")
    }
}
impl fmt::Debug for Acceptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TrojanAcceptor { .. }")
    }
}
impl fmt::Debug for Accepted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TrojanAccepted { .. }")
    }
}
