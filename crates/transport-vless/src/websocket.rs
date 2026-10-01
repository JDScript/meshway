//! Bounded binary WebSocket byte carrier. RFC6455 framing, fragmentation,
//! masking and control frames are delegated to tungstenite. The caller owns
//! TLS, an overall handshake deadline and cancellation by dropping this stream.

use futures_util::{Sink, Stream};
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        handshake::server::{ErrorResponse, Request, Response},
        protocol::WebSocketConfig,
    },
};

pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;
const WRITE_CHUNK: usize = 16 * 1024;
pub const MAX_UPGRADE_BYTES: usize = 64 * 1024;

/// Limits HTTP upgrade reads until tungstenite has accepted the handshake.
pub struct HandshakeIo<S> {
    inner: S,
    remaining: Option<usize>,
}

impl<S: AsyncRead + Unpin> AsyncRead for HandshakeIo<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let Some(remaining) = this.remaining else {
            return Pin::new(&mut this.inner).poll_read(cx, output);
        };
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if remaining == 0 {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WebSocket upgrade exceeds byte budget",
            )));
        }
        let count = output.remaining().min(remaining);
        let read = {
            let mut limited = ReadBuf::new(output.initialize_unfilled_to(count));
            match Pin::new(&mut this.inner).poll_read(cx, &mut limited) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(())) => limited.filled().len(),
            }
        };
        this.remaining = Some(remaining - read);
        output.advance(read);
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for HandshakeIo<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// Exact HTTP upgrade parameters; Host is independent of TLS SNI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameters {
    pub path: String,
    pub host: String,
}

impl Parameters {
    pub fn validate(&self) -> io::Result<()> {
        if !self.path.starts_with('/')
            || self.path.len() > 1024
            || self.path.contains(['\r', '\n', '#', '?', ' '])
            || self.host.is_empty()
            || self.host.len() > 253
            || !self.host.is_ascii()
            || self.host.contains(['/', '\\', '?', '#', '@'])
            || self
                .host
                .bytes()
                .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid WebSocket parameters",
            ));
        }
        Ok(())
    }
}

fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .read_buffer_size(16 * 1024)
        .write_buffer_size(0)
        .max_write_buffer_size(MAX_MESSAGE_BYTES)
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES))
}

fn error(failure: tokio_tungstenite::tungstenite::Error) -> io::Error {
    // Remote HTTP bodies and payloads must never enter logs through errors.
    if let tokio_tungstenite::tungstenite::Error::Http(response) = failure {
        return io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!(
                "WebSocket upgrade rejected (HTTP {})",
                response.status().as_u16()
            ),
        );
    }
    io::Error::new(io::ErrorKind::InvalidData, "WebSocket carrier failed")
}

// tungstenite's fixed callback ABI returns an HTTP Response by value.
#[allow(clippy::result_large_err)]
pub async fn accept<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    parameters: &Parameters,
) -> io::Result<ByteStream<HandshakeIo<S>>> {
    parameters.validate()?;
    let callback = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        if request.uri().path_and_query().map(|p| p.as_str()) != Some(parameters.path.as_str())
            || request.headers().get_all("host").iter().count() != 1
            || request.headers().get("host").and_then(|v| v.to_str().ok())
                != Some(parameters.host.as_str())
        {
            let mut rejected = ErrorResponse::new(Some("WebSocket path or Host mismatch".into()));
            *rejected.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::NOT_FOUND;
            return Err(rejected);
        }
        Ok(response)
    };
    let stream = HandshakeIo {
        inner: stream,
        remaining: Some(MAX_UPGRADE_BYTES),
    };
    let mut websocket =
        tokio_tungstenite::accept_hdr_async_with_config(stream, callback, Some(config()))
            .await
            .map_err(error)?;
    websocket.get_mut().remaining = None;
    Ok(ByteStream::new(websocket))
}

pub async fn connect<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    parameters: &Parameters,
) -> io::Result<ByteStream<HandshakeIo<S>>> {
    parameters.validate()?;
    let request = format!("wss://{}{}", parameters.host, parameters.path)
        .into_client_request()
        .map_err(error)?;
    let stream = HandshakeIo {
        inner: stream,
        remaining: Some(MAX_UPGRADE_BYTES),
    };
    let (mut websocket, _) =
        tokio_tungstenite::client_async_with_config(request, stream, Some(config()))
            .await
            .map_err(error)?;
    websocket.get_mut().remaining = None;
    Ok(ByteStream::new(websocket))
}

pub struct ByteStream<S> {
    websocket: WebSocketStream<S>,
    pending: Vec<u8>,
    offset: usize,
    eof: bool,
}

impl<S> ByteStream<S> {
    fn new(websocket: WebSocketStream<S>) -> Self {
        Self {
            websocket,
            pending: Vec::new(),
            offset: 0,
            eof: false,
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for ByteStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        // Bound control-only work so a ping flood cannot starve withdrawal.
        for _ in 0..32 {
            if this.offset < this.pending.len() {
                let count = output.remaining().min(this.pending.len() - this.offset);
                output.put_slice(&this.pending[this.offset..this.offset + count]);
                this.offset += count;
                if this.offset == this.pending.len() {
                    this.pending.clear();
                    this.offset = 0;
                }
                return Poll::Ready(Ok(()));
            }
            if this.eof {
                // Complete tungstenite's queued close reply before reporting
                // EOF, including a reader that never calls shutdown itself.
                return match Pin::new(&mut this.websocket).poll_flush(cx) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
                    Poll::Ready(Err(
                        tokio_tungstenite::tungstenite::Error::ConnectionClosed
                        | tokio_tungstenite::tungstenite::Error::AlreadyClosed,
                    )) => Poll::Ready(Ok(())),
                    Poll::Ready(Err(e)) => Poll::Ready(Err(error(e))),
                };
            }
            // Flush tungstenite's queued pong/close replies even in read-only use.
            match Pin::new(&mut this.websocket).poll_flush(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(error(e))),
                Poll::Ready(Ok(())) => {}
            }
            match Pin::new(&mut this.websocket).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(Message::Binary(bytes)))) => {
                    this.pending = bytes.to_vec();
                }
                Poll::Ready(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => {}
                Poll::Ready(Some(Ok(Message::Close(_)))) | Poll::Ready(None) => {
                    this.eof = true;
                }
                Poll::Ready(Some(Ok(_))) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "non-binary WebSocket payload",
                    )));
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(error(e))),
            }
        }
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for ByteStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.eof {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        if bytes.is_empty() {
            return Poll::Ready(Ok(0));
        }
        match Pin::new(&mut this.websocket).poll_ready(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(error(e))),
            Poll::Ready(Ok(())) => {
                let count = bytes.len().min(WRITE_CHUNK);
                Pin::new(&mut this.websocket)
                    .start_send(Message::Binary(bytes[..count].to_vec().into()))
                    .map_err(error)?;
                Poll::Ready(Ok(count))
            }
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().websocket)
            .poll_flush(cx)
            .map_err(error)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().websocket)
            .poll_close(cx)
            .map_err(error)
    }
}
