use std::{
    collections::VecDeque,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard},
    task::{Context, Poll, ready},
};

use futures_util::{Sink, Stream};
use tokio::io::{AsyncWrite, ReadHalf, WriteHalf};
use tokio_util::{
    codec::{FramedRead, FramedWrite, LengthDelimitedCodec, LengthDelimitedCodecError},
    sync::CancellationToken,
};

use super::send::SendFailure;
use crate::output::{OutputScope, ScopedStream};
use crate::{CancelReason, Frame, Limits, SendContext, raw};

enum Reader {
    Stream {
        reader: FramedRead<ReadHalf<Box<dyn raw::Stream>>, LengthDelimitedCodec>,
        deferred: Option<Result<Frame, super::ReceiveError>>,
        batch_count: usize,
    },
    Datagram {
        endpoint: Arc<dyn raw::Datagram>,
        buffer: Vec<u8>,
        batch_buffers: Vec<Vec<u8>>,
        batch_count: usize,
        scratch_count: usize,
        pending: VecDeque<Frame>,
        deferred: Option<super::ReceiveError>,
    },
}

enum Writer {
    Stream(FramedWrite<PollLimitedWriter<WriteHalf<Box<dyn raw::Stream>>>, LengthDelimitedCodec>),
    Datagram(Arc<dyn raw::Datagram>),
}

/// LengthDelimitedCodec's flush may poll the raw writer repeatedly. Bound one
/// engine poll's work; ScopedStream guards both directions below the split.
struct PollLimitedWriter<W> {
    inner: W,
    remaining_polls: usize,
}

impl<W> PollLimitedWriter<W> {
    fn activate(&mut self) {
        self.remaining_polls = 64;
    }

    fn ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.remaining_polls == 0 {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        self.remaining_polls -= 1;
        Poll::Ready(Ok(()))
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for PollLimitedWriter<W> {
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
    output: Option<Arc<OutputScope>>,
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
        let stop = CancellationToken::new();
        let output = OutputScope::new(stop.clone());
        let stream: Box<dyn raw::Stream> = Box::new(ScopedStream {
            inner: stream,
            scope: output.clone(),
        });
        let (read, write) = tokio::io::split(stream);
        let write = PollLimitedWriter {
            inner: write,
            remaining_polls: 0,
        };
        let buffer_bytes = limits.stream_buffer_bytes();
        let mut write = FramedWrite::with_capacity(write, codec(limits), buffer_bytes);
        write.set_backpressure_boundary(buffer_bytes);
        Self {
            stop,
            peer: Some(peer),
            output: Some(output),
            reader: Mutex::new(Some(Reader::Stream {
                reader: FramedRead::with_capacity(read, codec(limits), buffer_bytes.min(64 * 1024)),
                deferred: None,
                batch_count: limits.max_batch_frames().min(32),
            })),
            writer: Mutex::new(Some(Writer::Stream(write))),
        }
    }

    pub fn datagram<D: raw::Datagram>(endpoint: D, limits: Limits) -> Self {
        let endpoint: Arc<dyn raw::Datagram> = Arc::new(endpoint);
        Self {
            stop: CancellationToken::new(),
            peer: None,
            output: None,
            reader: Mutex::new(Some(Reader::Datagram {
                endpoint: endpoint.clone(),
                buffer: vec![0; limits.max_frame_bytes()],
                batch_buffers: Vec::new(),
                batch_count: limits.max_batch_frames().min(32),
                scratch_count: limits.max_batch_frames().min(32).min(
                    (limits.max_bytes().min(2 * 1024 * 1024) / limits.max_frame_bytes()).max(1),
                ),
                pending: VecDeque::new(),
                deferred: None,
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
        if let Some(output) = &self.output {
            output.complete();
        }
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
                // Admission validated the u32 length and codec ceiling. Borrow
                // custom packet storage directly into the existing length-framed
                // staging group, without a temporary Vec/Bytes payload copy.
                writer
                    .write_buffer_mut()
                    .extend_from_slice(&(frame.payload.len() as u32).to_be_bytes());
                writer.write_buffer_mut().extend_from_slice(&frame.payload);
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
        let output = self.output.as_ref().expect("stream owns an output gate");
        if !progress.flush_started {
            output.activate(context);
            progress.flush_started = true;
        }
        writer.get_mut().activate();
        let result = Pin::new(&mut *writer).poll_flush(cx);
        match result {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(self
                .check_scope(context)
                .err()
                .unwrap_or_else(|| error.into()))),
            Poll::Ready(Ok(())) => {
                self.check_scope(context)?;
                output.complete();
                Poll::Ready(Ok(()))
            }
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

    pub fn poll_datagram_batch(
        &self,
        cx: &mut Context<'_>,
        frames: &VecDeque<Frame>,
        context: &SendContext,
        started: &mut bool,
        count: usize,
    ) -> Poll<Result<usize, SendFailure>> {
        let mut writer = lock(&self.writer);
        self.check_scope(context)?;
        let Some(Writer::Datagram(endpoint)) = writer.as_mut() else {
            return Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe).into()));
        };
        let packets: Vec<_> = frames
            .iter()
            .take(count)
            .map(|frame| raw::DatagramSend {
                payload: &frame.payload,
                peer: frame.peer.expect("admitted datagram peer"),
            })
            .collect();
        *started = true;
        let sent = ready!(endpoint.poll_send_batch(cx, &packets))?;
        if sent == 0 || sent > count {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid batch send count",
            )
            .into()));
        }
        Poll::Ready(Ok(sent))
    }

    pub fn poll_recv_batch(
        &self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Vec<Frame>, super::ReceiveError>> {
        use super::ReceiveError;
        if self.is_stream() {
            let count = match lock(&self.reader).as_ref() {
                Some(Reader::Stream { batch_count, .. }) => *batch_count,
                _ => return Poll::Ready(Err(ReceiveError::Closed)),
            };
            let mut frames = Vec::new();
            let mut bytes = 0;
            while frames.len() < count {
                match self.poll_recv(cx) {
                    Poll::Pending if frames.is_empty() => return Poll::Pending,
                    Poll::Pending => break,
                    Poll::Ready(result) => {
                        let defer = !frames.is_empty()
                            && match &result {
                                Ok(frame) => bytes + frame.payload.len() > 65536,
                                Err(_) => true,
                            };
                        if defer {
                            if let Some(Reader::Stream { deferred, .. }) =
                                lock(&self.reader).as_mut()
                            {
                                *deferred = Some(result);
                            }
                            break;
                        }
                        let frame = result?;
                        bytes += frame.payload.len();
                        frames.push(frame);
                        if bytes >= 65536 {
                            break;
                        }
                    }
                }
            }
            return Poll::Ready(Ok(frames));
        }
        let mut reader = lock(&self.reader);
        let Some(Reader::Datagram {
            endpoint,
            buffer,
            batch_buffers,
            batch_count,
            scratch_count,
            pending,
            deferred,
        }) = reader.as_mut()
        else {
            return Poll::Ready(Err(ReceiveError::Closed));
        };
        if !pending.is_empty() {
            return Poll::Ready(Ok(drain_datagrams(pending)));
        }
        if let Some(error) = deferred.take() {
            return Poll::Ready(Err(error));
        }
        if batch_buffers.is_empty() {
            // Scratch capacity is independent of the delivered batch size.
            // Reuse a small scratch across ready polls instead of forcing a
            // task handoff after every scratch-sized prefix.
            batch_buffers.extend((0..*scratch_count).map(|_| vec![0; buffer.len()]));
        }
        let mut bytes = 0;
        while pending.len() < *batch_count && bytes < 65536 {
            let count = batch_buffers.len().min(*batch_count - pending.len());
            let received = match endpoint.poll_recv_batch(cx, &mut batch_buffers[..count]) {
                Poll::Pending if pending.is_empty() => return Poll::Pending,
                Poll::Pending => break,
                Poll::Ready(result) => result.map_err(ReceiveError::Io).and_then(|reads| {
                    if reads.is_empty() || reads.len() > count {
                        return Err(ReceiveError::InvalidMetadata);
                    }
                    let mut frames = Vec::with_capacity(reads.len());
                    for (received, buffer) in reads.into_iter().zip(&*batch_buffers) {
                        if received.truncated {
                            return Err(ReceiveError::Truncated(received));
                        }
                        if received.copied_len > buffer.len()
                            || received.original_len != Some(received.copied_len)
                        {
                            return Err(ReceiveError::InvalidMetadata);
                        }
                        if received.copied_len == 0 {
                            return Err(ReceiveError::Empty);
                        }
                        frames.push(Frame::new(
                            buffer[..received.copied_len].to_vec(),
                            Some(received.source),
                        ));
                    }
                    Ok(frames)
                }),
            };
            match received {
                Ok(frames) => {
                    bytes += frames
                        .iter()
                        .map(|frame| frame.payload.len())
                        .sum::<usize>();
                    pending.extend(frames);
                }
                Err(error) if pending.is_empty() => return Poll::Ready(Err(error)),
                Err(error) => {
                    *deferred = Some(error);
                    break;
                }
            }
        }
        Poll::Ready(Ok(drain_datagrams(pending)))
    }

    pub fn poll_recv(&self, cx: &mut Context<'_>) -> Poll<Result<Frame, super::ReceiveError>> {
        use super::ReceiveError;
        let mut reader = lock(&self.reader);
        match reader.as_mut() {
            Some(Reader::Stream {
                reader, deferred, ..
            }) => {
                if let Some(result) = deferred.take() {
                    return Poll::Ready(result);
                }
                match ready!(Pin::new(reader).poll_next(cx)) {
                    Some(Ok(payload)) if payload.is_empty() => {
                        Poll::Ready(Err(ReceiveError::Empty))
                    }
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
                }
            }
            Some(Reader::Datagram {
                endpoint,
                buffer,
                pending,
                deferred,
                ..
            }) => {
                if let Some(frame) = pending.pop_front() {
                    return Poll::Ready(Ok(frame));
                }
                if let Some(error) = deferred.take() {
                    return Poll::Ready(Err(error));
                }
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

fn drain_datagrams(pending: &mut VecDeque<Frame>) -> Vec<Frame> {
    let mut bytes = 0;
    let mut frames = Vec::new();
    while let Some(frame) = pending.front() {
        if frames.len() == 32 || (!frames.is_empty() && bytes + frame.payload.len() > 65536) {
            break;
        }
        bytes += frame.payload.len();
        frames.push(pending.pop_front().expect("front exists"));
    }
    frames
}
