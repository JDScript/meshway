use crate::{Carrier, Error};
use meshway_control::ExecutionSnapshot;
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
        if self.cancellation.is_cancelled() || Instant::now() >= self.deadline {
            return Err(Error::Cancelled);
        }
        self.snapshot
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        self.edge_deadline()?;
        Ok(())
    }

    /// Check on every resumed poll, including futures that perform multiple
    /// writes internally. Cancellation drops the future and its owned stream.
    pub async fn run<T>(&self, future: impl Future<Output = Result<T, Error>>) -> Result<T, Error> {
        let edge_deadline = self.edge_deadline()?;
        tokio::pin!(future);
        let result = tokio::select! { biased;
            _ = self.cancellation.cancelled() => Err(Error::Cancelled),
            _ = self.snapshot.invalidated_at(edge_deadline) => Err(Error::Authority),
            _ = tokio::time::sleep_until(self.deadline) => Err(Error::Cancelled),
            result = poll_fn(|cx| {
                if let Err(error) = self.check() { return Poll::Ready(Err(error)); }
                future.as_mut().poll(cx)
            }) => result,
        }?;
        self.check()?;
        Ok(result)
    }
}

/// The last poll boundary, below provider buffering and below Session. It has
/// the execution deadline, while individual handshake futures have a shorter cap.
pub(crate) struct Stream {
    pub inner: Box<dyn meshway_transport::raw::Stream>,
    pub scope: Scope,
}
impl Stream {
    fn check(&self) -> std::io::Result<()> {
        self.scope
            .check()
            .map_err(|_| std::io::ErrorKind::PermissionDenied.into())
    }
}
impl tokio::io::AsyncRead for Stream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.check()?;
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buffer)
    }
}
impl tokio::io::AsyncWrite for Stream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.check()?;
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buffer)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.check()?;
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.check()?;
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
