use std::{
    collections::VecDeque,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard},
    task::{Context, Poll, ready},
};

use bytes::Bytes;
use futures_util::{Sink, Stream};
use tokio::io::{AsyncWrite, ReadHalf, WriteHalf};
use tokio_util::{
    codec::{FramedRead, FramedWrite, LengthDelimitedCodec, LengthDelimitedCodecError},
    sync::CancellationToken,
};

use super::send::SendFailure;
use crate::{CancelReason, Frame, Limits, SendContext, raw};

enum Reader {
    Stream(FramedRead<ReadHalf<Box<dyn raw::Stream>>, LengthDelimitedCodec>),
    Datagram {
        endpoint: Arc<dyn raw::Datagram>,
        buffer: Vec<u8>,
    },
}

enum Writer {
    Stream(FramedWrite<ScopedWriter<WriteHalf<Box<dyn raw::Stream>>>, LengthDelimitedCodec>),
    Datagram(Arc<dyn raw::Datagram>),
}

/// LengthDelimitedCodec's flush may poll the raw writer repeatedly. Guard each
/// call, including successful short writes, and bound one engine poll's work.
struct ScopedWriter<W> {
    inner: W,
    stop: CancellationToken,
    context: Option<SendContext>,
    remaining_polls: usize,
}

impl<W> ScopedWriter<W> {
    fn activate(&mut self, context: &SendContext) {
        self.context = Some(context.clone());
        self.remaining_polls = 64;
    }

    fn ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.stop.is_cancelled()
            || self
                .context
                .as_ref()
                .is_none_or(|context| context.invalid_reason().is_some())
        {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "transport send scope invalidated",
            )));
        }
        if self.remaining_polls == 0 {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        self.remaining_polls -= 1;
        Poll::Ready(Ok(()))
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for ScopedWriter<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        ready!(this.ready(cx))?;
        Pin::new(&mut this.inner).poll_write(cx, bytes)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        ready!(this.ready(cx))?;
        Pin::new(&mut this.inner).poll_write_vectored(cx, bytes)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.ready(cx))?;
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.ready(cx))?;
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

/// Owned cursor into the front of one operation's remaining frames. The
/// FramedWrite owns partial encoded/write state; this cursor prevents replay
/// when a poll is handed from admission to the worker.
#[derive(Default)]
pub(super) struct StreamProgress {
    pub staged: usize,
    encoded_bytes: usize,
    pub flush_started: bool,
}

/// The same bounded work allowance applies to admission and worker polls.
/// A single validated frame may exceed the byte quantum, but never the
/// protocol buffer ceiling. Raw flush is attempted at most once per poll.
pub(super) struct PollBudget {
    frames: usize,
    bytes: usize,
    flushes: usize,
    used: bool,
}

impl PollBudget {
    pub fn fresh() -> Self {
        Self {
            frames: 128,
            bytes: 256 * 1024,
            flushes: 1,
            used: false,
        }
    }

    pub fn take_frame(&mut self, bytes: usize) -> bool {
        if self.frames == 0 || (self.used && bytes > self.bytes) {
            return false;
        }
        self.frames -= 1;
        self.bytes = self.bytes.saturating_sub(bytes);
        self.used = true;
        true
    }
}

/// Locks protect only synchronous polls and resource removal, never await
/// points. This permits Drop to close both halves before aborting the task,
/// including when a receive future is idle or a write is blocked.
pub(super) struct Shared {
    pub stop: CancellationToken,
    pub peer: Option<SocketAddr>,
    reader: Mutex<Option<Reader>>,
    writer: Mutex<Option<Writer>>,
}

fn codec(limits: Limits) -> LengthDelimitedCodec {
    LengthDelimitedCodec::builder()
        .big_endian()
        .length_field_type::<u32>()
        .max_frame_length(limits.max_frame_bytes())
        .new_codec()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A provider panic must still let the owner remove and drop its carriers.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Shared {
    pub fn stream<S: raw::Stream>(stream: S, peer: SocketAddr, limits: Limits) -> Self {
        let stream: Box<dyn raw::Stream> = Box::new(stream);
        let (read, write) = tokio::io::split(stream);
        let stop = CancellationToken::new();
        let write = ScopedWriter {
            inner: write,
            stop: stop.clone(),
            context: None,
            remaining_polls: 0,
        };
        let buffer_bytes = limits.stream_buffer_bytes();
        let mut write = FramedWrite::with_capacity(write, codec(limits), buffer_bytes);
        write.set_backpressure_boundary(buffer_bytes);
        Self {
            stop,
            peer: Some(peer),
            reader: Mutex::new(Some(Reader::Stream(FramedRead::with_capacity(
                read,
                codec(limits),
                buffer_bytes.min(64 * 1024),
            )))),
            writer: Mutex::new(Some(Writer::Stream(write))),
        }
    }

    pub fn datagram<D: raw::Datagram>(endpoint: D, limits: Limits) -> Self {
        let endpoint: Arc<dyn raw::Datagram> = Arc::new(endpoint);
        Self {
            stop: CancellationToken::new(),
            peer: None,
            reader: Mutex::new(Some(Reader::Datagram {
                endpoint: endpoint.clone(),
                buffer: vec![0; limits.max_frame_bytes()],
            })),
            writer: Mutex::new(Some(Writer::Datagram(endpoint))),
        }
    }

    pub fn is_stream(&self) -> bool {
        self.peer.is_some()
    }

    pub fn close(&self) {
        self.stop.cancel();
        // Do not hold both locks simultaneously: generic split streams may
        // themselves synchronize read/write polls internally.
        lock(&self.writer).take();
        lock(&self.reader).take();
    }

    /// Staging uses only the codec, never carrier I/O. Cancellation before the
    /// first flush poll can therefore discard the entire staging buffer while
    /// keeping the carrier usable. Once flush is polled, generic AsyncWrite
    /// buffering makes every staged frame potentially sent.
    pub fn poll_stream_group(
        &self,
        cx: &mut Context<'_>,
        frames: &mut VecDeque<Frame>,
        context: &SendContext,
        progress: &mut StreamProgress,
        budget: &mut PollBudget,
    ) -> Poll<Result<(), SendFailure>> {
        let mut writer = lock(&self.writer);
        self.check_scope(context)?;
        let Some(Writer::Stream(writer)) = writer.as_mut() else {
            return Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe).into()));
        };
        let ceiling = writer.backpressure_boundary();
        if !progress.flush_started {
            if progress.staged == 0 {
                // Reclaim the original empty allocation before starting a
                // group, avoiding offset-related BytesMut growth.
                writer.write_buffer_mut().reserve(ceiling);
            }
            while let Some(frame) = frames.get_mut(progress.staged) {
                let wire_len = frame.payload.len() + 4;
                if wire_len > ceiling - progress.encoded_bytes {
                    break;
                }
                if !budget.take_frame(wire_len) {
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                // The existing encoded length is strictly below the boundary;
                // poll_ready cannot auto-flush and thus cannot touch raw I/O.
                match Pin::new(&mut *writer).poll_ready(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(result) => result?,
                }
                // Move the Vec into Bytes without a temporary payload copy.
                // This guard also restores the Vec if encoding unwinds.
                struct RestorePayload<'a> {
                    target: &'a mut Vec<u8>,
                    bytes: Bytes,
                }
                impl Drop for RestorePayload<'_> {
                    fn drop(&mut self) {
                        *self.target = std::mem::take(&mut self.bytes).into();
                    }
                }
                let bytes = Bytes::from(std::mem::take(&mut frame.payload));
                let restore = RestorePayload {
                    target: &mut frame.payload,
                    bytes,
                };
                Pin::new(&mut *writer).start_send(restore.bytes.clone())?;
                drop(restore);
                progress.staged += 1;
                progress.encoded_bytes += wire_len;
            }
        }
        // Encoding is bounded to 128 frames / the byte quantum per lock. Check
        // hard invalidation again after encoding and before any carrier poll.
        self.check_scope(context)?;
        if budget.flushes == 0 {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        budget.flushes -= 1;
        progress.flush_started = true;
        writer.get_mut().activate(context);
        let result = Pin::new(&mut *writer).poll_flush(cx);
        writer.get_mut().context = None;
        match result {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(self
                .check_scope(context)
                .err()
                .unwrap_or_else(|| error.into()))),
            Poll::Ready(Ok(())) => Poll::Ready(self.check_scope(context)),
        }
    }

    pub fn discard_staged_stream(&self) {
        if let Some(Writer::Stream(writer)) = lock(&self.writer).as_mut() {
            writer.write_buffer_mut().clear();
        }
    }

    fn check_scope(&self, context: &SendContext) -> Result<(), SendFailure> {
        let invalid = if self.stop.is_cancelled() {
            Some(CancelReason::SessionClosed)
        } else {
            context.invalid_reason()
        };
        invalid.map_or(Ok(()), |reason| Err(SendFailure::Cancelled(reason)))
    }

    pub fn poll_datagram(
        &self,
        cx: &mut Context<'_>,
        frame: &Frame,
        context: &SendContext,
        started: &mut bool,
    ) -> Poll<Result<(), SendFailure>> {
        let mut writer = lock(&self.writer);
        self.check_scope(context)?;
        match writer.as_mut() {
            Some(Writer::Datagram(endpoint)) => {
                let peer = frame.peer.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "datagram needs a peer")
                })?;
                *started = true;
                let length = ready!(endpoint.poll_send_to(cx, &frame.payload, peer))?;
                if length != frame.payload.len() {
                    Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "partial datagram send",
                    )
                    .into()))
                } else {
                    Poll::Ready(Ok(()))
                }
            }
            _ => Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe).into())),
        }
    }

    pub fn poll_recv(&self, cx: &mut Context<'_>) -> Poll<Result<Frame, super::ReceiveError>> {
        use super::ReceiveError;
        let mut reader = lock(&self.reader);
        match reader.as_mut() {
            Some(Reader::Stream(reader)) => match ready!(Pin::new(reader).poll_next(cx)) {
                Some(Ok(payload)) if payload.is_empty() => Poll::Ready(Err(ReceiveError::Empty)),
                Some(Ok(payload)) => Poll::Ready(Ok(Frame::new(payload.to_vec(), self.peer))),
                Some(Err(error))
                    if error
                        .get_ref()
                        .is_some_and(|source| source.is::<LengthDelimitedCodecError>()) =>
                {
                    Poll::Ready(Err(ReceiveError::Oversized))
                }
                Some(Err(error)) => Poll::Ready(Err(ReceiveError::Io(error))),
                None => Poll::Ready(Err(ReceiveError::Closed)),
            },
            Some(Reader::Datagram { endpoint, buffer }) => {
                let received = ready!(endpoint.poll_recv_from(cx, buffer))?;
                if received.truncated {
                    return Poll::Ready(Err(ReceiveError::Truncated(received)));
                }
                if received.copied_len > buffer.len()
                    || received.original_len != Some(received.copied_len)
                {
                    return Poll::Ready(Err(ReceiveError::InvalidMetadata));
                }
                if received.copied_len == 0 {
                    return Poll::Ready(Err(ReceiveError::Empty));
                }
                Poll::Ready(Ok(Frame::new(
                    buffer[..received.copied_len].to_vec(),
                    Some(received.source),
                )))
            }
            None => Poll::Ready(Err(ReceiveError::Closed)),
        }
    }
}
