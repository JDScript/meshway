//! HTTP/1.1 upgrade and one bounded binary-message byte-stream adapter.
use std::{
    io,
    net::IpAddr,
    pin::Pin,
    task::{Context, Poll, ready},
};

use bytes::Bytes;
use futures_util::{Sink, Stream as MessageStream};
use meshway_transport::raw::Stream;
use meshway_transport::spec::{
    DialerWebSocket, ListenerWebSocket, WebSocketHeaders, normalize_host,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        handshake::server::{Callback, ErrorResponse, Request, Response},
        http::{HeaderMap, HeaderName, HeaderValue, StatusCode, Uri},
        protocol::WebSocketConfig,
    },
};
use zeroize::Zeroizing;

use crate::{Error, wake::WakeBridge};

const CHUNK: usize = 16 * 1024;
const MAX_MESSAGE: usize = 1024 * 1024;
const MAX_UPGRADE: usize = 16 * 1024;
const MAX_HEADERS: usize = 8 * 1024;
const MAX_PATH: usize = 2048;

pub(super) struct Settings {
    path: String,
    host: HeaderValue,
    authority: String,
    request_headers: Headers,
    response_headers: Headers,
}

struct Headers(Vec<(HeaderName, Zeroizing<Vec<u8>>)>);

impl Headers {
    fn new(headers: &WebSocketHeaders) -> Result<Self, Error> {
        if headers.len() > 64 {
            return Err(Error::Configuration);
        }
        let mut size = 0usize;
        let mut result = Vec::with_capacity(headers.len());
        for (name, value) in headers.iter() {
            size = size
                .checked_add(name.len())
                .and_then(|n| n.checked_add(value.len()))
                .and_then(|n| n.checked_add(4))
                .ok_or(Error::Configuration)?;
            if size > MAX_HEADERS {
                return Err(Error::Configuration);
            }
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| Error::Configuration)?;
            // Tungstenite serializes through HeaderValue::to_str, which accepts
            // visible ASCII/HTAB but not obs-text (including UTF-8 values).
            HeaderValue::from_bytes(value.as_bytes())
                .map_err(|_| Error::Configuration)?
                .to_str()
                .map_err(|_| Error::Configuration)?;
            result.push((name, Zeroizing::new(value.as_bytes().to_vec())));
        }
        Ok(Self(result))
    }

    fn apply(&self, headers: &mut HeaderMap) -> Result<(), Error> {
        for (name, value) in &self.0 {
            let mut value = HeaderValue::from_bytes(value).map_err(|_| Error::Configuration)?;
            // Library diagnostics must not print custom authorization material.
            value.set_sensitive(true);
            headers.insert(name.clone(), value);
        }
        Ok(())
    }
}

impl Settings {
    pub(super) fn new(
        listener: &ListenerWebSocket,
        dialer: Option<&DialerWebSocket>,
        server_name: &str,
    ) -> Result<Self, Error> {
        let path = listener.path.as_str();
        if path.len() > MAX_PATH {
            return Err(Error::Configuration);
        }
        let path = path.to_owned();
        let host = normalize_host(listener.host.as_deref().unwrap_or(server_name))
            .map_err(|_| Error::Configuration)?;
        let authority = if matches!(host.parse::<IpAddr>(), Ok(IpAddr::V6(_))) {
            format!("[{host}]")
        } else {
            host
        };
        let host = HeaderValue::from_str(&authority).map_err(|_| Error::Configuration)?;
        let request_headers = match dialer {
            Some(dialer) => Headers::new(&dialer.headers)?,
            None => Headers(Vec::new()),
        };
        // Validate URI representability during construction, not after TLS.
        format!("wss://{authority}{path}")
            .parse::<Uri>()
            .map_err(|_| Error::Configuration)?;
        Ok(Self {
            path,
            host,
            authority,
            request_headers,
            response_headers: Headers::new(&listener.response_headers)?,
        })
    }

    pub(super) async fn connect<S: Stream>(
        &self,
        stream: S,
    ) -> Result<ByteStream<UpgradeIo<S>>, Error> {
        let mut request = format!("wss://{}{}", self.authority, self.path)
            .into_client_request()
            .map_err(|_| Error::Configuration)?;
        request.headers_mut().insert("host", self.host.clone());
        self.request_headers.apply(request.headers_mut())?;
        let (mut stream, response) = tokio_tungstenite::client_async_with_config(
            request,
            UpgradeIo::new(stream),
            Some(config()),
        )
        .await
        .map_err(|_| Error::WebSocket)?;
        if response.headers().contains_key("content-length")
            || response.headers().contains_key("transfer-encoding")
            || response.headers().contains_key("sec-websocket-extensions")
            || response.headers().contains_key("sec-websocket-protocol")
        {
            return Err(Error::WebSocket);
        }
        stream.get_mut().remaining = None;
        Ok(ByteStream::new(stream))
    }

    pub(super) async fn accept<S: Stream>(
        &self,
        stream: S,
    ) -> Result<ByteStream<UpgradeIo<S>>, Error> {
        let mut stream = tokio_tungstenite::accept_hdr_async_with_config(
            UpgradeIo::new(stream),
            Upgrade(self),
            Some(config()),
        )
        .await
        .map_err(|_| Error::WebSocket)?;
        stream.get_mut().remaining = None;
        Ok(ByteStream::new(stream))
    }
}

struct Upgrade<'a>(&'a Settings);

impl Callback for Upgrade<'_> {
    fn on_request(
        self,
        request: &Request,
        mut response: Response,
    ) -> Result<Response, ErrorResponse> {
        let settings = self.0;
        if request.uri().path_and_query().map(|p| p.as_str()) != Some(settings.path.as_str())
            || request.uri().scheme().is_some()
            || request.uri().authority().is_some()
            || request.headers().get_all("host").iter().count() != 1
            || request.headers().get("host") != Some(&settings.host)
            || ["sec-websocket-key", "sec-websocket-version", "upgrade"]
                .iter()
                .any(|name| request.headers().get_all(*name).iter().count() != 1)
            || request.headers().contains_key("content-length")
            || request.headers().contains_key("transfer-encoding")
            || request.headers().contains_key("sec-websocket-protocol")
            || request.headers().contains_key("sec-websocket-extensions")
        {
            let mut rejection = ErrorResponse::new(Some("invalid WebSocket upgrade".into()));
            *rejection.status_mut() = StatusCode::BAD_REQUEST;
            return Err(rejection);
        }
        if settings
            .response_headers
            .apply(response.headers_mut())
            .is_err()
        {
            let mut rejection = ErrorResponse::new(None);
            *rejection.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            return Err(rejection);
        }
        Ok(response)
    }
}

fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .read_buffer_size(CHUNK)
        .write_buffer_size(0)
        .max_write_buffer_size(2 * CHUNK)
        .max_frame_size(Some(MAX_MESSAGE))
        .max_message_size(Some(MAX_MESSAGE))
}

/// Caps inbound upgrade bytes including any library read-ahead. Disabled only
/// after a successful upgrade; the enclosing provider caps elapsed time.
pub(super) struct UpgradeIo<S> {
    inner: S,
    remaining: Option<usize>,
}

impl<S> UpgradeIo<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            remaining: Some(MAX_UPGRADE),
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for UpgradeIo<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let Some(remaining) = this.remaining.as_mut() else {
            return Pin::new(&mut this.inner).poll_read(cx, output);
        };
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if *remaining == 0 {
            return Poll::Ready(Err(invalid()));
        }
        let length = output.remaining().min(*remaining);
        let mut limited = ReadBuf::new(output.initialize_unfilled_to(length));
        ready!(Pin::new(&mut this.inner).poll_read(cx, &mut limited))?;
        let length = limited.filled().len();
        output.advance(length);
        *remaining -= length;
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for UpgradeIo<S> {
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

/// One incoming message and at most one outstanding outgoing 16 KiB message.
/// Tungstenite owns continuation/control framing and masking; this adapter only
/// flattens binary messages, never HTTP, text, ping, pong, or close bytes.
pub(super) struct ByteStream<S> {
    inner: WebSocketStream<S>,
    wake: WakeBridge,
    buffered: Bytes,
    flush: Flush,
    ws_closed: bool,
    read_closed: bool,
    shutdown_started: bool,
    write_closed: bool,
    failed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flush {
    Done,
    WebSocket,
    Transport,
}

impl<S> ByteStream<S> {
    fn new(inner: WebSocketStream<S>) -> Self {
        Self {
            inner,
            wake: WakeBridge::new(),
            buffered: Bytes::new(),
            // Even Sink::start_send's context-free internal writes must inherit
            // our bridge: poll_flush installs it before the first message.
            flush: Flush::WebSocket,
            ws_closed: false,
            read_closed: false,
            shutdown_started: false,
            write_closed: false,
            failed: false,
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> ByteStream<S> {
    fn poll_output_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Server tungstenite may return ConnectionClosed after write_out_buffer
        // but before flushing its carrier. Remember completion of that phase so
        // backpressure never causes another poll of the terminated WS state.
        if self.flush != Flush::Transport {
            if !self.ws_closed {
                if ready!(Pin::new(&mut self.inner).poll_flush(cx)).is_err() {
                    self.failed = true;
                    self.wake.wake_all();
                    return Poll::Ready(Err(invalid()));
                }
                if self.read_closed {
                    self.ws_closed = true;
                }
            }
            self.flush = Flush::Transport;
        }
        match ready!(Pin::new(self.inner.get_mut()).poll_flush(cx)) {
            Ok(()) => {
                self.flush = Flush::Done;
                Poll::Ready(Ok(()))
            }
            Err(_) => {
                self.failed = true;
                self.wake.wake_all();
                Poll::Ready(Err(invalid()))
            }
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
        if this.failed {
            return Poll::Ready(Err(invalid()));
        }
        let waker = this.wake.read(cx);
        let mut bridge_cx = Context::from_waker(&waker);
        let cx = &mut bridge_cx;
        // Bound work even when the peer sends many empty messages/control frames.
        for _ in 0..32 {
            if !this.buffered.is_empty() {
                let length = this.buffered.len().min(output.remaining());
                output.put_slice(&this.buffered.split_to(length));
                return Poll::Ready(Ok(()));
            }
            // Drive automatic pong/close and pending TLS output, but still
            // permit reads when writing blocks (full-duplex backpressure).
            if this.flush != Flush::Done
                && let Poll::Ready(result) = this.poll_output_flush(cx)
            {
                result?;
            }
            if this.read_closed {
                if this.flush != Flush::Done {
                    return Poll::Pending;
                }
                return Poll::Ready(Ok(()));
            }
            match ready!(Pin::new(&mut this.inner).poll_next(cx)) {
                Some(Ok(Message::Binary(bytes))) => this.buffered = bytes,
                Some(Ok(Message::Ping(_))) => this.flush = Flush::WebSocket,
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) => {
                    this.read_closed = true;
                    this.flush = Flush::WebSocket;
                    this.wake.wake_all();
                }
                None => {
                    this.read_closed = true;
                    this.ws_closed = true;
                    this.flush = Flush::Transport;
                    this.wake.wake_all();
                }
                Some(Ok(_)) | Some(Err(_)) => {
                    this.failed = true;
                    this.wake.wake_all();
                    return Poll::Ready(Err(invalid()));
                }
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
        if this.failed {
            return Poll::Ready(Err(invalid()));
        }
        if this.shutdown_started || this.read_closed {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        if bytes.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let waker = this.wake.write(cx);
        let mut bridge_cx = Context::from_waker(&waker);
        let cx = &mut bridge_cx;
        if this.flush != Flush::Done {
            ready!(this.poll_output_flush(cx))?;
        }
        if ready!(Pin::new(&mut this.inner).poll_ready(cx)).is_err() {
            this.failed = true;
            this.wake.wake_all();
            return Poll::Ready(Err(invalid()));
        }
        let length = bytes.len().min(CHUNK);
        if Pin::new(&mut this.inner)
            .start_send(Message::Binary(Bytes::copy_from_slice(&bytes[..length])))
            .is_err()
        {
            this.failed = true;
            this.wake.wake_all();
            return Poll::Ready(Err(invalid()));
        }
        this.flush = Flush::WebSocket;
        Poll::Ready(Ok(length))
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.failed {
            return Poll::Ready(Err(invalid()));
        }
        let waker = this.wake.write(cx);
        this.poll_output_flush(&mut Context::from_waker(&waker))
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.failed {
            return Poll::Ready(Err(invalid()));
        }
        let waker = this.wake.write(cx);
        let mut bridge_cx = Context::from_waker(&waker);
        let cx = &mut bridge_cx;
        this.shutdown_started = true;
        if !this.write_closed {
            if !this.ws_closed && ready!(Pin::new(&mut this.inner).poll_close(cx)).is_err() {
                this.failed = true;
                this.wake.wake_all();
                return Poll::Ready(Err(invalid()));
            }
            this.write_closed = true;
            this.ws_closed |= this.read_closed;
            this.flush = Flush::Transport;
        }
        ready!(this.poll_output_flush(cx))?;
        Pin::new(this.inner.get_mut()).poll_shutdown(cx)
    }
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid WebSocket stream")
}

#[cfg(test)]
mod tests;
