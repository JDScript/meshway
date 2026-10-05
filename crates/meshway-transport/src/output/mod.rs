//! The session owns one frozen output scope. Synchronous provider polls carry
//! it through opaque stream adapters to the actual output boundary.
use std::{
    cell::RefCell,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_util::sync::CancellationToken;

use crate::SendContext;

thread_local! {
    // This is a synchronous poll context, never task-local state across await.
    // No authorization is conveyed; the context can only restrict output.
    static CURRENT: RefCell<Option<Arc<OutputScope>>> = const { RefCell::new(None) };
}

pub(crate) struct OutputScope {
    stop: CancellationToken,
    active: Mutex<Option<SendContext>>,
}

impl OutputScope {
    pub(crate) fn new(stop: CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            stop,
            active: Mutex::new(None),
        })
    }

    pub(crate) fn activate(&self, context: &SendContext) {
        let previous = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(context.clone());
        drop(previous);
    }

    pub(crate) fn complete(&self) {
        let completed = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(completed);
    }

    fn check(&self) -> io::Result<()> {
        // User lifetimes may call other synchronous code. Do not retain the
        // state lock while invoking one, or while polling the provider.
        let active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if self.stop.is_cancelled()
            || active
                .as_ref()
                .is_some_and(|context| context.invalid_reason().is_some())
        {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "transport output scope invalidated",
            ));
        }
        Ok(())
    }

    fn enter(self: &Arc<Self>) -> PollScope {
        PollScope(CURRENT.with(|current| current.replace(Some(self.clone()))))
    }
}

struct PollScope(Option<Arc<OutputScope>>);

impl Drop for PollScope {
    fn drop(&mut self) {
        CURRENT.with(|current| current.replace(self.0.take()));
    }
}

fn check_current() -> io::Result<()> {
    let scope = CURRENT.with(|current| current.borrow().clone());
    scope.map_or(Ok(()), |scope| scope.check())
}

/// Install this immediately below a buffering provider (for example below TLS
/// and WebSocket), around its caller-supplied I/O. Each actual write, vectored
/// write, flush and shutdown rechecks the enclosing Session's frozen operation
/// scope, including output driven by reads and multiple writes in one poll.
///
/// Session installs the restriction for both read and write polls, through any
/// intervening opaque stream wrappers. It survives Pending in the Session, not
/// in thread-local storage: the synchronous poll context is restored on return
/// or unwind. Providers must not detach output into background tasks. Direct
/// raw use outside a Session has no operation restriction. Socket protection
/// and authorization still belong to the injected underlay.
pub struct CheckedOutput<S> {
    inner: S,
}

impl<S> CheckedOutput<S> {
    pub fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for CheckedOutput<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buffer)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for CheckedOutput<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        check_current()?;
        Pin::new(&mut self.get_mut().inner).poll_write(cx, bytes)
    }
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        check_current()?;
        Pin::new(&mut self.get_mut().inner).poll_write_vectored(cx, bytes)
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        check_current()?;
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        check_current()?;
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// Above the whole provider and below Tokio's split, so either direction uses
/// the same operation gate even when AsyncRead flushes application output.
pub(crate) struct ScopedStream<S> {
    pub(crate) inner: S,
    pub(crate) scope: Arc<OutputScope>,
}

impl<S: AsyncRead + Unpin> AsyncRead for ScopedStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.scope.check()?;
        let _scope = this.scope.enter();
        Pin::new(&mut this.inner).poll_read(cx, buffer)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for ScopedStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.scope.check()?;
        let _scope = this.scope.enter();
        Pin::new(&mut this.inner).poll_write(cx, bytes)
    }
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.scope.check()?;
        let _scope = this.scope.enter();
        Pin::new(&mut this.inner).poll_write_vectored(cx, bytes)
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.scope.check()?;
        let _scope = this.scope.enter();
        Pin::new(&mut this.inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.scope.check()?;
        let _scope = this.scope.enter();
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests;
