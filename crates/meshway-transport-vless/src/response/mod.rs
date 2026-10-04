use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll, ready},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::{Instant, Sleep};

use crate::wake::WakeBridge;

/// Cancellation-safe lazy response parsing. Only the empty-addon v0 response
/// is supported; headers never reach the caller's buffer, including on error.
pub(crate) struct Response<S> {
    inner: Option<S>,
    header: [u8; 2],
    filled: usize,
    timer: Option<Pin<Box<Sleep>>>,
    wake: WakeBridge,
    failure: Option<io::ErrorKind>,
}

impl<S> Response<S> {
    pub(crate) fn new(inner: S, deadline: Instant) -> Self {
        Self {
            inner: Some(inner),
            header: [0; 2],
            filled: 0,
            timer: Some(Box::pin(tokio::time::sleep_until(deadline))),
            wake: WakeBridge::new(),
            failure: None,
        }
    }

    fn fail(&mut self, kind: io::ErrorKind) -> io::Error {
        self.inner.take();
        self.timer.take();
        self.failure = Some(kind);
        self.wake.wake_all();
        failed(kind)
    }

    /// Poll before *every* carrier operation while the header is unconfirmed.
    /// Checking the Instant as well as Sleep prevents accepting a readable but
    /// already-stale header; polling Sleep arms blocked writes/flush/shutdown.
    /// Every direction supplies the same cached bridge waker to the one clock.
    fn check_deadline(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        if let Some(kind) = self.failure {
            return Err(failed(kind));
        }
        if let Some(timer) = &mut self.timer
            && (Instant::now() >= timer.deadline() || timer.as_mut().poll(cx).is_ready())
        {
            return Err(self.fail(io::ErrorKind::TimedOut));
        }
        Ok(())
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Response<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let waker = this.wake.read(cx);
        let mut bridge_cx = Context::from_waker(&waker);
        let cx = &mut bridge_cx;
        ready!(this.poll_header(cx))?;
        let Some(inner) = &mut this.inner else {
            return Poll::Ready(Err(failed(io::ErrorKind::InvalidData)));
        };
        Pin::new(inner).poll_read(cx, output)
    }
}

impl<S: AsyncRead + Unpin> Response<S> {
    fn poll_header(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self;
        this.check_deadline(cx)?;
        while this.filled < 2 {
            let Some(inner) = &mut this.inner else {
                return Poll::Ready(Err(failed(io::ErrorKind::InvalidData)));
            };
            let mut header = ReadBuf::new(&mut this.header[this.filled..]);
            match Pin::new(inner).poll_read(cx, &mut header) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(error)) => {
                    return Poll::Ready(Err(this.fail(error.kind())));
                }
                Poll::Pending => return Poll::Pending,
            }
            let read = header.filled().len();
            this.check_deadline(cx)?;
            if read == 0 {
                return Poll::Ready(Err(this.fail(io::ErrorKind::UnexpectedEof)));
            }
            this.filled += read;
            if this.header[0] != 0 || (this.filled == 2 && this.header[1] != 0) {
                return Poll::Ready(Err(this.fail(io::ErrorKind::InvalidData)));
            }
        }
        if this.timer.take().is_some() {
            this.wake.wake_all();
        }
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for Response<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let waker = this.wake.write(cx);
        this.poll_io(&mut Context::from_waker(&waker), |inner, cx| {
            inner.poll_write(cx, bytes)
        })
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let waker = this.wake.write(cx);
        this.poll_io(&mut Context::from_waker(&waker), |inner, cx| {
            inner.poll_flush(cx)
        })
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let waker = this.wake.write(cx);
        this.poll_io(&mut Context::from_waker(&waker), |inner, cx| {
            inner.poll_shutdown(cx)
        })
    }
}

impl<S: AsyncRead + Unpin> Response<S> {
    fn poll_io<T>(
        &mut self,
        cx: &mut Context<'_>,
        poll: impl FnOnce(Pin<&mut S>, &mut Context<'_>) -> Poll<io::Result<T>>,
    ) -> Poll<io::Result<T>> {
        // Confirm an already-available header without holding application
        // writes behind a Pending read. This also lets one-way writers finish
        // the handshake before its original deadline, without consuming payload.
        if let Poll::Ready(result) = self.poll_header(cx) {
            result?;
        }
        self.check_deadline(cx)?;
        let Some(inner) = &mut self.inner else {
            return Poll::Ready(Err(failed(io::ErrorKind::InvalidData)));
        };
        let result = poll(Pin::new(inner), cx);
        self.check_deadline(cx)?;
        if self.timer.is_some()
            && let Poll::Ready(Err(error)) = &result
        {
            return Poll::Ready(Err(self.fail(error.kind())));
        }
        result
    }
}

fn failed(kind: io::ErrorKind) -> io::Error {
    io::Error::new(kind, "VLESS response handshake failed")
}

#[cfg(test)]
mod tests;
