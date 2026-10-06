use crate::{Carrier, Error};
use meshway_authority::ExecutionSnapshot;
use meshway_model::NodeId;
use std::{
    future::{Future, poll_fn},
    task::Poll,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(crate) struct Scope {
    pub snapshot: ExecutionSnapshot,
    pub cancellation: CancellationToken,
    pub deadline: Instant,
    pub edge: Option<(NodeId, Carrier)>,
    pub health: Option<std::sync::Arc<std::sync::Mutex<Instant>>>,
}

impl Scope {
    fn edge_deadline(&self) -> Result<u64, Error> {
        match &self.edge {
            Some((peer, carrier)) => meshway_policy::admission::admit_adjacency(
                self.snapshot
                    .configuration()
                    .map_err(|_| Error::Authority)?,
                *peer,
                carrier,
                self.snapshot.now_ms().map_err(|_| Error::Authority)?,
            )
            .map_err(|_| Error::Authority),
            None => Ok(self.snapshot.expires_at_ms()),
        }
    }
    pub fn check(&self) -> Result<(), Error> {
        let result = self.check_current();
        if result.is_err() {
            self.cancellation.cancel();
        }
        result
    }

    fn check_current(&self) -> Result<(), Error> {
        if self.cancellation.is_cancelled() || Instant::now() >= self.deadline {
            return Err(Error::Cancelled);
        }
        self.snapshot
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        self.edge_deadline()?;
        if let Some(health) = &self.health
            && Instant::now() >= *health.lock().map_err(|_| Error::Worker)?
        {
            return Err(Error::Unavailable);
        }
        Ok(())
    }

    async fn health_expired(&self) {
        let Some(health) = &self.health else {
            return std::future::pending().await;
        };
        loop {
            let deadline = match health.lock() {
                Ok(deadline) => *deadline,
                Err(_) => return,
            };
            tokio::time::sleep_until(deadline).await;
            if self.check().is_err() {
                return;
            }
        }
    }

    /// Check on every resumed poll, including futures that perform multiple
    /// writes internally. Cancellation drops the operation; shared endpoint
    /// ownership stays with its driver.
    pub async fn run<T>(&self, future: impl Future<Output = Result<T, Error>>) -> Result<T, Error> {
        let edge_deadline = self.edge_deadline()?;
        tokio::pin!(future);
        let result = tokio::select! { biased;
            _ = self.cancellation.cancelled() => Err(Error::Cancelled),
            _ = self.snapshot.invalidated_at(edge_deadline) => Err(Error::Authority),
            _ = tokio::time::sleep_until(self.deadline) => Err(Error::Cancelled),
            _ = self.health_expired() => Err(Error::Unavailable),
            result = poll_fn(|cx| {
                if let Err(error) = self.check() { return Poll::Ready(Err(error)); }
                future.as_mut().poll(cx)
            }) => result,
        }?;
        self.check()?;
        Ok(result)
    }
}

/// Endpoint lifetime below the formal Session. Per-output peer/proof lifetime
/// remains in SendContext, so sharing a socket never shares packet authority.
pub(crate) struct Datagram {
    pub inner: Box<dyn meshway_transport::raw::Datagram>,
    pub scope: std::sync::Arc<Physical>,
}
impl meshway_transport::raw::Datagram for Datagram {
    fn local_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        self.scope
            .check()
            .map_err(|_| std::io::ErrorKind::PermissionDenied)?;
        self.inner.local_addr()
    }
    fn poll_send_to(
        &self,
        cx: &mut std::task::Context<'_>,
        bytes: &[u8],
        peer: std::net::SocketAddr,
    ) -> Poll<std::io::Result<usize>> {
        std::task::ready!(self.scope.poll(cx, false))?;
        self.inner.poll_send_to(cx, bytes, peer)
    }
    fn poll_recv_from(
        &self,
        cx: &mut std::task::Context<'_>,
        bytes: &mut [u8],
    ) -> Poll<std::io::Result<meshway_transport::raw::DatagramRead>> {
        std::task::ready!(self.scope.poll(cx, true))?;
        let result = self.inner.poll_recv_from(cx, bytes);
        if matches!(result, Poll::Ready(Err(_))) && self.scope.check().is_err() {
            return self.scope.poll(cx, true).map(|result| {
                result.and_then(|_| Err(std::io::ErrorKind::PermissionDenied.into()))
            });
        }
        result
    }
}

impl std::fmt::Debug for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerScope")
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

impl meshway_transport::SendLifetime for Scope {
    fn is_current(&self) -> bool {
        self.check().is_ok()
    }

    fn invalidated(
        self: std::sync::Arc<Self>,
    ) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            let _: Result<(), Error> = self.run(std::future::pending()).await;
            self.cancellation.cancel();
        })
    }
}

/// Physical poll gate below providers/Session. Opening retains a frozen guard
/// until authentication completes; established outputs use Session's frozen
/// per-operation SendContext. Buffered provider channels are not retained.
pub(crate) struct Stream {
    pub inner: Box<dyn meshway_transport::raw::Stream>,
    pub scope: std::sync::Arc<Physical>,
    pub opening: Option<std::sync::Arc<Opening>>,
}
impl Stream {
    fn opening(&self) -> std::io::Result<()> {
        if let Some(opening) = &self.opening {
            opening
                .check()
                .map_err(|_| std::io::ErrorKind::PermissionDenied)?;
        }
        Ok(())
    }
}
impl tokio::io::AsyncRead for Stream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.opening()?;
        std::task::ready!(self.scope.poll(cx, true))?;
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buffer)
    }
}
impl tokio::io::AsyncWrite for Stream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.opening()?;
        std::task::ready!(self.scope.poll(cx, false))?;
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buffer)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.opening()?;
        std::task::ready!(self.scope.poll(cx, false))?;
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.opening()?;
        std::task::ready!(self.scope.poll(cx, false))?;
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub(crate) struct Opening(std::sync::Mutex<Option<Scope>>);
impl Opening {
    pub fn new(scope: Scope) -> Self {
        Self(std::sync::Mutex::new(Some(scope)))
    }
    fn check(&self) -> Result<(), Error> {
        if let Some(scope) = &*self.0.lock().map_err(|_| Error::Worker)? {
            scope.check()?;
        }
        Ok(())
    }
    pub fn release(&self) -> Result<(), Error> {
        let mut scope = self.0.lock().map_err(|_| Error::Worker)?;
        if let Some(scope) = &*scope {
            scope.check()?;
        }
        *scope = None;
        Ok(())
    }
}

/// The physical gate follows only Manager-approved epochs. It is NEVER a
/// SendLifetime: every accepted output instead retains its frozen Scope.
pub(crate) struct Physical {
    epochs: tokio::sync::watch::Receiver<meshway_authority::DataPlaneIdentity>,
    stop: CancellationToken,
    read: futures_util::task::AtomicWaker,
    write: futures_util::task::AtomicWaker,
}
impl Physical {
    pub fn new(
        epochs: tokio::sync::watch::Receiver<meshway_authority::DataPlaneIdentity>,
        stop: CancellationToken,
    ) -> Self {
        Self {
            epochs,
            stop,
            read: Default::default(),
            write: Default::default(),
        }
    }
    pub fn wake(&self) {
        self.read.wake();
        self.write.wake();
    }
    pub fn check(&self) -> Result<(), Error> {
        if self.stop.is_cancelled() {
            return Err(Error::Cancelled);
        }
        self.epochs
            .borrow()
            .snapshot()
            .ensure_current()
            .map_err(|_| Error::Authority)
    }
    fn poll(&self, cx: &std::task::Context<'_>, read: bool) -> Poll<std::io::Result<()>> {
        if read {
            self.read.register(cx.waker());
        } else {
            self.write.register(cx.waker());
        }
        if self.stop.is_cancelled() {
            return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
        }
        let identity = self.epochs.borrow();
        if identity.snapshot().ensure_current().is_ok() {
            return Poll::Ready(Ok(()));
        }
        if identity.snapshot().retention_deadline().is_err() {
            return Poll::Ready(Err(std::io::ErrorKind::PermissionDenied.into()));
        }
        Poll::Pending
    }
}
