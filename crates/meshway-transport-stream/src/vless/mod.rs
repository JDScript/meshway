//! VLESS v0 TCP CONNECT over TLS, optionally inside WebSocket binary messages.
//!
//! Destination metadata never authorizes proxy routing or authenticates a Mesh
//! peer. The host supplies its service target and checks accepted destinations.
mod protocol;
mod response;
mod wake;
mod websocket;

use std::{fmt, io, time::Duration};

use meshway_model::VlessStream;
use meshway_transport::raw::{CheckedOutput, Stream};
use meshway_transport::spec::{
    DialParameters, DialerStream, Endpoint, Listener, ListenerSecret, PreparedListener,
    validate_websocket_alpn,
};
use tokio::{io::AsyncWriteExt, time::Instant};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsupported or invalid VLESS configuration")]
    Configuration,
    #[error("VLESS credential rejected")]
    Credential,
    #[error("invalid or unsupported VLESS handshake")]
    Protocol,
    #[error("VLESS handshake timed out")]
    Timeout,
    #[error("VLESS WebSocket handshake failed")]
    WebSocket,
    #[error("VLESS TLS failed")]
    Tls(#[from] crate::tls::Error),
    #[error("VLESS stream I/O failed")]
    Io(io::ErrorKind),
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

pub struct Connector {
    tls: crate::tls::Client,
    uuid: Zeroizing<[u8; 16]>,
    websocket: Option<websocket::Settings>,
    timeout: Duration,
}

pub struct Acceptor {
    tls: crate::tls::Server,
    uuid: Zeroizing<[u8; 16]>,
    websocket: Option<websocket::Settings>,
    timeout: Duration,
}

pub struct Accepted {
    pub stream: Box<dyn Stream>,
    pub destination: Endpoint,
}

impl Connector {
    pub fn new(parameters: &DialParameters) -> Result<Self, Error> {
        let DialParameters::Vless {
            options,
            stream,
            flow,
            uuid,
            tls,
            websocket,
        } = parameters
        else {
            return Err(Error::Configuration);
        };
        if !options.enabled || !flow.is_empty() || options.tls != tls.options {
            return Err(Error::Configuration);
        }
        let websocket = match stream {
            VlessStream::Tcp => {
                if options.stream == DialerStream::WebSocket
                    || websocket.is_some()
                    || options.websocket.is_some()
                {
                    return Err(Error::Configuration);
                }
                None
            }
            VlessStream::WebSocket => {
                if options.stream == DialerStream::Tcp {
                    return Err(Error::Configuration);
                }
                validate_websocket_alpn(&tls.options.alpn).map_err(|_| Error::Configuration)?;
                Some(websocket::Settings::new(
                    websocket.as_ref().ok_or(Error::Configuration)?,
                    options.websocket.as_ref(),
                    &tls.server_name,
                )?)
            }
        };
        let timeout = options.connect_timeout.as_std();
        deadline(timeout)?;
        Ok(Self {
            tls: crate::tls::Client::new(tls)?,
            uuid: protocol::uuid(uuid.expose())?,
            websocket,
            timeout,
        })
    }

    /// Writes the request eagerly and consumes an available response header
    /// during subsequent I/O. Application writes never wait for the header:
    /// upstreams may wait for the first application write before replying.
    /// The original connect deadline caps all I/O until the header is confirmed.
    /// Cancellation drops the owned stream; the host's total budget encloses us.
    pub async fn connect<S: Stream>(
        &self,
        io: S,
        destination: &Endpoint,
    ) -> Result<Box<dyn Stream>, Error> {
        let deadline = deadline(self.timeout)?;
        tokio::time::timeout_at(deadline, async {
            let request = protocol::request(&self.uuid, destination)?;
            let tls = self.tls.connect(CheckedOutput::new(io)).await?;
            let mut stream: Box<dyn Stream> = match &self.websocket {
                Some(settings) => Box::new(settings.connect(tls).await?),
                None => Box::new(tls),
            };
            stream.write_all(&request).await?;
            stream.flush().await?;
            Ok(Box::new(response::Response::new(stream, deadline)) as Box<dyn Stream>)
        })
        .await
        .map_err(|_| Error::Timeout)?
    }
}

impl Acceptor {
    pub fn new(listener: &PreparedListener) -> Result<Self, Error> {
        let Listener::Vless(options) = listener.declaration() else {
            return Err(Error::Configuration);
        };
        if !options.enabled || !options.flow.is_empty() {
            return Err(Error::Configuration);
        }
        let websocket = match options.stream {
            VlessStream::Tcp if options.websocket.is_none() => None,
            VlessStream::Tcp => return Err(Error::Configuration),
            VlessStream::WebSocket => {
                validate_websocket_alpn(&options.tls.alpn).map_err(|_| Error::Configuration)?;
                Some(websocket::Settings::new(
                    options.websocket.as_ref().ok_or(Error::Configuration)?,
                    None,
                    &options.tls.server_name,
                )?)
            }
        };
        let timeout = options.tls.handshake_timeout.as_std();
        deadline(timeout)?;
        Ok(Self {
            tls: crate::tls::Server::new(&options.tls)?,
            uuid: protocol::uuid(
                options
                    .uuid
                    .as_ref()
                    .and_then(ListenerSecret::value)
                    .ok_or(Error::Configuration)?
                    .expose(),
            )?,
            websocket,
            timeout,
        })
    }

    /// The listener TLS handshake cap encloses TLS, upgrade, and VLESS parsing.
    pub async fn accept<S: Stream>(&self, io: S) -> Result<Accepted, Error> {
        tokio::time::timeout_at(deadline(self.timeout)?, async {
            let tls = self.tls.accept(CheckedOutput::new(io)).await?;
            let mut stream: Box<dyn Stream> = match &self.websocket {
                Some(settings) => Box::new(settings.accept(tls).await?),
                None => Box::new(tls),
            };
            let destination = protocol::accept(&mut stream, &self.uuid).await?;
            stream.write_all(&[0, 0]).await?;
            stream.flush().await?;
            Ok(Accepted {
                stream,
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
        f.write_str("VlessConnector { .. }")
    }
}
impl fmt::Debug for Acceptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VlessAcceptor { .. }")
    }
}
impl fmt::Debug for Accepted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VlessAccepted { .. }")
    }
}
