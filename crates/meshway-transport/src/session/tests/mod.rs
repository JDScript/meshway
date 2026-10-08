use std::{
    collections::VecDeque,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf},
    time::{Instant, timeout},
};

use crate::{
    AdmissionStop, BudgetAvailable, CancelReason, Frame, Limits, ReceiveError, SendContext,
    SendFailure, SendOutcome, Session,
};

fn peer() -> SocketAddr {
    "127.0.0.1:12345".parse().unwrap()
}

fn limits() -> Limits {
    Limits::new(64, 4, 256, 3, 3).unwrap()
}

fn batch(payloads: &[&[u8]]) -> VecDeque<Frame> {
    payloads
        .iter()
        .map(|payload| Frame::new(payload.to_vec(), None))
        .collect()
}

fn wire(payloads: &[&[u8]]) -> Vec<u8> {
    let mut wire = Vec::new();
    for payload in payloads {
        wire.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        wire.extend_from_slice(payload);
    }
    wire
}

struct DropObserved {
    stream: DuplexStream,
    dropped: Arc<AtomicBool>,
    flush_started: Option<tokio::sync::oneshot::Sender<()>>,
    block_flush: bool,
}

impl Drop for DropObserved {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl AsyncRead for DropObserved {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buffer)
    }
}

impl AsyncWrite for DropObserved {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buffer)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.block_flush {
            if let Some(started) = self.flush_started.take() {
                let _ = started.send(());
            }
            return Poll::Pending;
        }
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[derive(Default)]
struct IoObservation {
    flush_sizes: Vec<usize>,
    bytes_since_flush: usize,
    write_polls: usize,
    maximum_write_request: usize,
    read_requests: Vec<usize>,
}

/// Records raw I/O calls while using a real Tokio memory stream; encoding,
/// grouping, cancellation and completion still run through production code.
struct ObservedStream {
    stream: DuplexStream,
    observation: Arc<Mutex<IoObservation>>,
}

fn observed_stream(capacity: usize) -> (ObservedStream, DuplexStream, Arc<Mutex<IoObservation>>) {
    let (stream, remote) = tokio::io::duplex(capacity);
    let observation = Arc::new(Mutex::new(IoObservation::default()));
    let observed = ObservedStream {
        stream,
        observation: observation.clone(),
    };
    (observed, remote, observation)
}

impl AsyncRead for ObservedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.observation
            .lock()
            .unwrap()
            .read_requests
            .push(buffer.remaining());
        Pin::new(&mut self.stream).poll_read(cx, buffer)
    }
}

impl AsyncWrite for ObservedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        {
            let mut observed = self.observation.lock().unwrap();
            observed.write_polls += 1;
            observed.maximum_write_request = observed.maximum_write_request.max(buffer.len());
        }
        let result = Pin::new(&mut self.stream).poll_write(cx, buffer);
        if let Poll::Ready(Ok(written)) = result {
            self.observation.lock().unwrap().bytes_since_flush += written;
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let result = Pin::new(&mut self.stream).poll_flush(cx);
        if let Poll::Ready(Ok(())) = result {
            let mut observed = self.observation.lock().unwrap();
            let bytes = std::mem::take(&mut observed.bytes_since_flush);
            observed.flush_sizes.push(bytes);
        }
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[derive(Default)]
struct ShortWriteObservation {
    bytes: Vec<u8>,
    writes: usize,
    flushes: usize,
}

struct AlwaysShortWriter {
    observation: Arc<Mutex<ShortWriteObservation>>,
    cancel_on_write: Option<tokio_util::sync::CancellationToken>,
    vectored: bool,
}

impl AsyncRead for AlwaysShortWriter {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

impl AsyncWrite for AlwaysShortWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if bytes.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let mut observation = self.observation.lock().unwrap();
        observation.bytes.push(bytes[0]);
        observation.writes += 1;
        if let Some(cancel) = &self.cancel_on_write {
            cancel.cancel();
        }
        Poll::Ready(Ok(1))
    }
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        self.poll_write(
            cx,
            bytes
                .iter()
                .find(|slice| !slice.is_empty())
                .map_or(&[], |slice| &slice[..]),
        )
    }
    fn is_write_vectored(&self) -> bool {
        self.vectored
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.observation.lock().unwrap().flushes += 1;
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

mod admission;

mod cancellation;

mod receive;

mod scheduling;
