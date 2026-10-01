use std::{
    fmt,
    io::{self, IoSlice},
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::{
    BoundAddress, ClientConfig, Error, ServerConfig, ValidatedRequest, client_handshake,
    server_handshake,
};

/// Owned, established carrier. SOCKS5 headers are already validated and consumed;
/// payload I/O (including vectored writes and half-close) delegates without copying.
/// The wrapper provides no encryption; establish Meshway Noise before payloads.
pub struct Socks5Stream<S> {
    inner: S,
}

impl<S> fmt::Debug for Socks5Stream<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Socks5Stream").finish_non_exhaustive()
    }
}

/// Complete the handshake, returning an owned stream and untrusted bound metadata.
/// Cancelling/dropping this future drops the stream it owns.
pub async fn client_stream<S>(
    mut stream: S,
    config: &ClientConfig,
) -> Result<(Socks5Stream<S>, BoundAddress), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let bound = client_handshake(&mut stream, config).await?;
    Ok((Socks5Stream { inner: stream }, bound))
}

/// Complete the fixed-target acceptor handshake and return its owned carrier.
pub async fn server_stream<S>(
    mut stream: S,
    config: &ServerConfig,
) -> Result<(Socks5Stream<S>, ValidatedRequest), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request = server_handshake(&mut stream, config).await?;
    Ok((Socks5Stream { inner: stream }, request))
}

impl<S> Socks5Stream<S> {
    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Socks5Stream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buffer)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Socks5Stream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buffer)
    }
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffers: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write_vectored(cx, buffers)
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}
