use std::{
    fmt, io,
    pin::Pin,
    task::{Context, Poll},
};

use shadowsocks::relay::tcprelay::{ProxyClientStream, ProxyServerStream};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

enum Inner<S> {
    Client(ProxyClientStream<S>),
    Server(ProxyServerStream<S>),
}

/// Owned encrypted full-duplex carrier. Can be split with `tokio::io::split`.
/// No raw-I/O escape hatch: upstream can retain decrypted payload/read state.
/// I/O errors poison both directions and retain only the nonsecret error kind.
pub struct CarrierStream<S> {
    inner: Inner<S>,
    failed: bool,
}

impl<S> CarrierStream<S> {
    pub(crate) fn client(stream: ProxyClientStream<S>) -> Self {
        Self {
            inner: Inner::Client(stream),
            failed: false,
        }
    }

    pub(crate) fn server(stream: ProxyServerStream<S>) -> Self {
        Self {
            inner: Inner::Server(stream),
            failed: false,
        }
    }

    fn checked<T>(&mut self, result: Poll<io::Result<T>>) -> Poll<io::Result<T>> {
        match result {
            Poll::Ready(Err(error)) => {
                self.failed = true;
                Poll::Ready(Err(io::Error::new(
                    error.kind(),
                    "Shadowsocks carrier I/O or authentication failed",
                )))
            }
            other => other,
        }
    }
}

impl<S> fmt::Debug for CarrierStream<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CarrierStream")
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

fn failed<T>() -> Poll<io::Result<T>> {
    Poll::Ready(Err(io::Error::new(
        io::ErrorKind::BrokenPipe,
        "Shadowsocks carrier is closed after an error",
    )))
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for CarrierStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.failed {
            return failed();
        }
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let result = match &mut this.inner {
            Inner::Client(stream) => Pin::new(stream).poll_read(cx, buf),
            Inner::Server(stream) => Pin::new(stream).poll_read(cx, buf),
        };
        this.checked(result)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for CarrierStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.failed {
            return failed();
        }
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let result = match &mut this.inner {
            Inner::Client(stream) => Pin::new(stream).poll_write(cx, buf),
            Inner::Server(stream) => Pin::new(stream).poll_write(cx, buf),
        };
        this.checked(result)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.failed {
            return failed();
        }
        let result = match &mut this.inner {
            Inner::Client(stream) => Pin::new(stream).poll_flush(cx),
            Inner::Server(stream) => Pin::new(stream).poll_flush(cx),
        };
        this.checked(result)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.failed {
            return failed();
        }
        let result = match &mut this.inner {
            Inner::Client(stream) => Pin::new(stream).poll_shutdown(cx),
            Inner::Server(stream) => Pin::new(stream).poll_shutdown(cx),
        };
        this.checked(result)
    }
}
