use std::{
    collections::VecDeque,
    future::{Future, poll_fn},
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
    task::{Context, Poll},
};

use thiserror::Error;
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    task::JoinHandle,
};

use super::io::{PollBudget, Shared, StreamProgress};
use crate::{CancelReason, Frame, Limits, ScopedFrame, SendContext};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionStop {
    Closed,
    ScopeInvalid(CancelReason),
    EmptyFrame,
    Oversized { size: usize, maximum: usize },
    MissingPeer,
    PeerMismatch,
    FrameBudget,
    ByteBudget,
    OperationBudget,
    BatchLimit,
    ChannelScopeMismatch,
}

/// Exactly `accepted` frames were removed from the caller's front. `stop`
/// describes why a suffix remains; a nonempty prefix always has a completion.
pub struct Admission {
    pub accepted: usize,
    pub stop: Option<AdmissionStop>,
    pub completion: Option<Completion>,
}

#[derive(Clone, Debug, Error)]
pub enum SendFailure {
    #[error("send invalidated: {0:?}")]
    Cancelled(CancelReason),
    /// The original error is shared across frames in a failed stream group.
    /// `From<io::Error>` wraps it without losing its kind or source chain.
    #[error(transparent)]
    Io(#[from] Arc<io::Error>),
    /// A provider panicked while the operation engine was polling it. The
    /// carrier is closed; panic payloads are never attached to this error.
    #[error("transport provider panicked")]
    ProviderPanicked,
}

impl From<io::Error> for SendFailure {
    fn from(error: io::Error) -> Self {
        Self::Io(Arc::new(error))
    }
}

#[derive(Clone, Debug)]
pub enum SendOutcome {
    /// Complete local write and required flush. This is not a peer ack.
    LocalComplete,
    /// No frame I/O was started. Its payload is safe to resubmit under a new
    /// host decision; the transport never resubmits it automatically.
    NotStarted(SendFailure),
    /// I/O was attempted, and may have partially or fully reached the peer.
    /// In particular, cancellation must not be interpreted as "not sent".
    /// All frames staged into an interrupted stream flush group receive this
    /// outcome, even if raw write counts could suggest a narrower prefix:
    /// a generic AsyncWrite may itself buffer data.
    Indeterminate(SendFailure),
}

#[derive(Debug)]
pub struct CompletedFrame {
    pub frame: Frame,
    pub outcome: SendOutcome,
}

#[derive(Debug)]
pub struct BatchCompletion {
    /// Original accepted order, with the original payload Vec allocations.
    pub frames: VecDeque<CompletedFrame>,
}

#[derive(Clone, Debug)]
pub enum BatchOutcome {
    LocalComplete,
    NotStarted(SendFailure),
    Uncertain(SendFailure),
}

impl BatchCompletion {
    /// Batch-level result; any partly completed or attempted failed batch is
    /// uncertain, even if individual datagrams have a narrower known outcome.
    pub fn outcome(&self) -> BatchOutcome {
        let failure = self.frames.iter().find_map(|f| match &f.outcome {
            SendOutcome::LocalComplete => None,
            SendOutcome::NotStarted(error) | SendOutcome::Indeterminate(error) => {
                Some(error.clone())
            }
        });
        let Some(failure) = failure else {
            return BatchOutcome::LocalComplete;
        };
        if self
            .frames
            .iter()
            .all(|f| matches!(f.outcome, SendOutcome::NotStarted(_)))
        {
            BatchOutcome::NotStarted(failure)
        } else {
            BatchOutcome::Uncertain(failure)
        }
    }
}

/// One bounded notification per accepted batch. Dropping it only drops the
/// observer: the send owner retains the operation and reclaims its buffers.
/// Explicit shutdown reports through outstanding completions. If the sender
/// is dropped/aborted instead, unreported buffers are freed by the owner and
/// the waiter receives `CompletionLost`, with no delivery guarantee.
pub struct Completion(oneshot::Receiver<BatchCompletion>);

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("send owner was aborted before reporting completion; delivery is unknown")]
pub struct CompletionLost;

impl Completion {
    pub async fn wait(self) -> Result<BatchCompletion, CompletionLost> {
        self.0.await.map_err(|_| CompletionLost)
    }
}

/// A diagnostic snapshot, not a reservation. Completed buffers are owned by
/// the result consumer and no longer charged to this session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BudgetAvailable {
    pub frames: usize,
    pub bytes: usize,
    pub operations: usize,
}

struct Reservation {
    _frames: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
    _operation: OwnedSemaphorePermit,
}

struct Operation {
    batch_io: bool,
    frames: VecDeque<Frame>,
    completed: VecDeque<CompletedFrame>,
    progress: Progress,
    terminal: Option<SendFailure>,
    context: SendContext,
    completion: oneshot::Sender<BatchCompletion>,
    reservation: Reservation,
}

enum Progress {
    Stream(StreamProgress),
    Datagram {
        started: bool,
        count: usize,
    },
    Report {
        remaining: usize,
        outcome: SendOutcome,
    },
}

impl Progress {
    fn new(io: &Shared) -> Self {
        if io.is_stream() {
            Self::Stream(StreamProgress::default())
        } else {
            Self::Datagram {
                started: false,
                count: 1,
            }
        }
    }
}

impl Operation {
    /// The one operation engine, used both by nonwaiting admission and by the
    /// worker. All state that survives Pending is owned, never a borrowed
    /// write future. FramedWrite retains its own partial flush cursor.
    fn poll(&mut self, io: &Shared, cx: &mut Context<'_>) -> Poll<()> {
        match catch_unwind(AssertUnwindSafe(|| self.poll_inner(io, cx))) {
            Ok(result) => result,
            Err(_panic) => {
                self.fail(io, SendFailure::ProviderPanicked);
                // Reporting also has a bounded quantum. A large failed
                // operation is handed off just like a large successful one.
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }

    fn poll_inner(&mut self, io: &Shared, cx: &mut Context<'_>) -> Poll<()> {
        let mut budget = PollBudget::fresh();
        let mut reports_left = 128;
        loop {
            if self.frames.is_empty() {
                return Poll::Ready(());
            }
            if let Progress::Report { remaining, outcome } = &mut self.progress {
                if reports_left == 0 {
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                self.completed.push_back(CompletedFrame {
                    frame: self
                        .frames
                        .pop_front()
                        .expect("report owns an accepted frame"),
                    outcome: outcome.clone(),
                });
                reports_left -= 1;
                *remaining -= 1;
                if *remaining == 0 {
                    self.progress = Progress::new(io);
                }
                continue;
            }
            if let Some(error) = &self.terminal {
                self.progress = Progress::Report {
                    remaining: self.frames.len(),
                    outcome: SendOutcome::NotStarted(error.clone()),
                };
                continue;
            }
            let result = match &mut self.progress {
                Progress::Stream(progress) => {
                    io.poll_stream_group(cx, &mut self.frames, &self.context, progress, &mut budget)
                }
                Progress::Datagram { started, count } => {
                    if self.batch_io {
                        let mut available = 0;
                        for frame in self.frames.iter().take(32) {
                            if !budget.take_frame(frame.payload.len()) {
                                break;
                            }
                            available += 1;
                        }
                        if available == 0 {
                            cx.waker().wake_by_ref();
                            return Poll::Pending;
                        }
                        *count = available;
                        io.poll_datagram_batch(cx, &self.frames, &self.context, started, available)
                            .map(|result| {
                                result.map(|sent| {
                                    *count = sent;
                                })
                            })
                    } else {
                        if !budget.take_frame(self.frames[0].payload.len()) {
                            cx.waker().wake_by_ref();
                            return Poll::Pending;
                        }
                        io.poll_datagram(cx, &self.frames[0], &self.context, started)
                    }
                }
                Progress::Report { .. } => unreachable!("reports handled above"),
            };
            match result {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => self.fail(io, error),
                Poll::Ready(Ok(())) => {
                    let count = match &self.progress {
                        Progress::Stream(progress) => progress.staged,
                        Progress::Datagram { count, .. } => *count,
                        Progress::Report { .. } => unreachable!("I/O has active progress"),
                    };
                    self.progress = Progress::Report {
                        remaining: count,
                        outcome: SendOutcome::LocalComplete,
                    };
                }
            }
        }
    }

    fn fail(&mut self, io: &Shared, error: SendFailure) {
        let panic = matches!(error, SendFailure::ProviderPanicked);
        let attempted = match &self.progress {
            Progress::Stream(progress) if progress.flush_started => progress.staged,
            Progress::Datagram {
                started: true,
                count,
            } => *count,
            _ => 0,
        };
        if panic || (io.is_stream() && attempted != 0) {
            // Keep caught provider unwinds out of admission as well as the
            // worker. close removes both resources and tolerates poison.
            let _ = catch_unwind(AssertUnwindSafe(|| io.close()));
        } else if io.is_stream() {
            io.discard_staged_stream();
        }
        if matches!(self.progress, Progress::Report { .. }) {
            // No raw I/O is polled while reporting an already-completed group.
            // Preserve those established outcomes on an unexpected unwind.
            self.terminal = Some(SendFailure::Cancelled(CancelReason::SessionClosed));
        } else if attempted == 0 {
            self.terminal = Some(error);
        } else {
            self.terminal = if panic || io.is_stream() {
                Some(SendFailure::Cancelled(CancelReason::SessionClosed))
            } else if self.batch_io || matches!(error, SendFailure::Cancelled(_)) {
                Some(error.clone())
            } else {
                None // A datagram I/O error does not poison the next frame.
            };
            self.progress = Progress::Report {
                remaining: attempted,
                outcome: SendOutcome::Indeterminate(error),
            };
        }
    }

    fn finish(self) {
        let Self {
            completed,
            completion,
            reservation,
            ..
        } = self;
        // Observing completion is also a guarantee that these budgets have
        // been restored. The writer lease remains held through notification.
        drop(reservation);
        let _ = completion.send(BatchCompletion { frames: completed });
    }
}

/// Explicit ownership, independent of payload budgets. In particular a
/// worker that has released its reservation but has not notified its waiter
/// is not idle. On unwind/abort close BEFORE making the writer idle, including
/// the gap before the outer worker close guard executes.
struct WriterLease {
    io: Arc<Shared>,
    _permit: Option<OwnedSemaphorePermit>,
    clean: bool,
}

impl WriterLease {
    fn new(io: Arc<Shared>, permit: OwnedSemaphorePermit) -> Self {
        Self {
            io,
            _permit: Some(permit),
            clean: false,
        }
    }

    fn release(mut self) {
        self.clean = true;
    }

    fn waiting(io: Arc<Shared>) -> Self {
        Self {
            io,
            _permit: None,
            clean: false,
        }
    }
}

impl Drop for WriterLease {
    fn drop(&mut self) {
        if !self.clean {
            let _ = catch_unwind(AssertUnwindSafe(|| self.io.close()));
        }
    }
}

/// Single admission owner. The task, queue, and all payloads are bounded. The
/// sender is intentionally not Clone; move it to the host's scheduling owner.
pub struct Sender {
    io: Arc<Shared>,
    limits: Limits,
    frame_budget: Arc<Semaphore>,
    byte_budget: Arc<Semaphore>,
    operation_budget: Arc<Semaphore>,
    idle: Arc<Semaphore>,
    queue: Option<mpsc::Sender<Operation>>,
    worker: Option<JoinHandle<()>>,
    control_reserve: BudgetAvailable,
}

impl Sender {
    pub(super) fn start(io: Arc<Shared>, limits: Limits) -> Self {
        let (queue, receiver) = mpsc::channel(limits.max_operations());
        let idle = Arc::new(Semaphore::new(1));
        let worker = tokio::spawn(run(io.clone(), receiver, idle.clone()));
        Self {
            io,
            limits,
            frame_budget: Arc::new(Semaphore::new(limits.max_frames())),
            byte_budget: Arc::new(Semaphore::new(limits.max_bytes())),
            operation_budget: Arc::new(Semaphore::new(limits.max_operations())),
            idle,
            queue: Some(queue),
            worker: Some(worker),
            control_reserve: BudgetAvailable {
                frames: 0,
                bytes: 0,
                operations: 0,
            },
        }
    }

    pub fn is_closed(&self) -> bool {
        self.io.stop.is_cancelled() || self.queue.as_ref().is_none_or(mpsc::Sender::is_closed)
    }

    pub fn available_budget(&self) -> BudgetAvailable {
        BudgetAvailable {
            frames: self.frame_budget.available_permits(),
            bytes: self.byte_budget.available_permits(),
            operations: self.operation_budget.available_permits(),
        }
    }

    /// Keep a portion of the existing budget unavailable to business batches.
    /// Independent single-frame control operations use the same I/O owner and
    /// original lifetimes; reserving capacity does not enlarge any resource cap.
    pub fn reserve_control(&mut self, reserve: BudgetAvailable) -> Result<(), AdmissionStop> {
        if reserve.frames >= self.limits.max_frames() {
            return Err(AdmissionStop::FrameBudget);
        }
        if reserve.bytes >= self.limits.max_bytes() {
            return Err(AdmissionStop::ByteBudget);
        }
        if reserve.operations >= self.limits.max_operations() {
            return Err(AdmissionStop::OperationBudget);
        }
        self.control_reserve = reserve;
        Ok(())
    }

    /// Wait until one frame's allocation can be admitted. This does not retain
    /// a reservation: callers must still use `try_send` and check its result.
    /// Closing the carrier wakes the waiter; impossible capacities fail early.
    pub async fn wait_capacity(&self, capacity: usize) -> Result<(), AdmissionStop> {
        if capacity > self.limits.max_bytes() {
            return Err(AdmissionStop::ByteBudget);
        }
        tokio::select! { biased;
            _ = self.io.stop.cancelled() => Err(AdmissionStop::Closed),
            result = async {
                let _frame = self.frame_budget.clone().acquire_owned().await.map_err(|_| AdmissionStop::Closed)?;
                let _bytes = self.byte_budget.clone().acquire_many_owned(capacity as u32).await.map_err(|_| AdmissionStop::Closed)?;
                let _operation = self.operation_budget.clone().acquire_owned().await.map_err(|_| AdmissionStop::Closed)?;
                Ok(())
            } => result,
        }
    }

    /// Nonwaiting prefix admission. Capacity is charged across queued **and**
    /// in-flight batches until their completion is produced. No suffix frame
    /// is cloned, retained, or consumed. One operation permit covers a batch.
    /// Stream frames are grouped within this operation's encoded-buffer limit;
    /// a group becomes locally complete only after the raw flush succeeds.
    /// When the writer is idle, one bounded poll runs here: up to 128 frames,
    /// a 256 KiB work quantum (or one larger validated frame), and one stream
    /// group flush attempt. Remaining work is owned by the worker, including
    /// any partial inline write. Ready completions need no worker scheduling.
    pub fn try_send(&mut self, frames: &mut VecDeque<Frame>, context: SendContext) -> Admission {
        self.admit(frames, context, false)
    }

    /// Admit an immediately ready business prefix (at most 32 frames/64 KiB).
    /// Scope identity and destination must match. Each original context and
    /// the supplied context restrict the entire operation; one completion owns
    /// every accepted buffer. No timer or wait is used to fill the batch.
    pub fn submit_batch(
        &mut self,
        frames: &mut VecDeque<ScopedFrame>,
        context: SendContext,
    ) -> Admission {
        if self.is_closed() {
            return Admission {
                accepted: 0,
                stop: Some(AdmissionStop::Closed),
                completion: None,
            };
        }
        if let Some(reason) = context.invalid_reason() {
            return Admission {
                accepted: 0,
                stop: Some(AdmissionStop::ScopeInvalid(reason)),
                completion: None,
            };
        }
        let mut count = 0;
        let mut bytes = 0;
        let mut stop = None;
        let mut available = self.available_budget();
        available.frames = available.frames.saturating_sub(self.control_reserve.frames);
        available.bytes = available.bytes.saturating_sub(self.control_reserve.bytes);
        if available.operations <= self.control_reserve.operations {
            return Admission {
                accepted: 0,
                stop: Some(AdmissionStop::OperationBudget),
                completion: None,
            };
        }
        if let Some(first) = frames.front() {
            for scoped in frames.iter() {
                stop = if !scoped.scope.same(&first.scope) {
                    Some(AdmissionStop::ChannelScopeMismatch)
                } else if scoped.frame.peer != first.frame.peer {
                    Some(AdmissionStop::PeerMismatch)
                } else if count == 32 {
                    Some(AdmissionStop::BatchLimit)
                } else if bytes + scoped.frame.payload.capacity() > 65536 {
                    Some(AdmissionStop::ByteBudget)
                } else {
                    self.frame_stop(&scoped.frame, count, bytes, available)
                };
                if stop.is_some() {
                    break;
                }
                count += 1;
                bytes += scoped.frame.payload.capacity();
            }
        }
        if count == 0 {
            return Admission {
                accepted: 0,
                stop,
                completion: None,
            };
        }
        let mut contexts = vec![context];
        let mut metadata = VecDeque::new();
        let mut plain = VecDeque::new();
        for scoped in frames.drain(..count) {
            contexts.push(scoped.context.clone());
            metadata.push_back((scoped.scope, scoped.context));
            plain.push_back(scoped.frame);
        }
        let mut admission = self.admit(&mut plain, SendContext::intersection(contexts), true);
        // A close or concurrent host invalidation may reject the prepared
        // prefix. Restore exactly its unaccepted allocations and metadata.
        for frame in plain.into_iter().rev() {
            let (scope, context) = metadata.pop_back().expect("unaccepted frame metadata");
            frames.push_front(ScopedFrame {
                frame,
                scope,
                context,
            });
        }
        if admission.stop.is_none() {
            admission.stop = stop;
        }
        admission
    }

    fn admit(
        &mut self,
        frames: &mut VecDeque<Frame>,
        context: SendContext,
        batch_io: bool,
    ) -> Admission {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("transport.submit");
        let _profile = PROFILE.enter();
        let rejected = |stop| Admission {
            accepted: 0,
            stop: Some(stop),
            completion: None,
        };
        if self.is_closed() {
            return rejected(AdmissionStop::Closed);
        }
        if let Some(reason) = context.invalid_reason() {
            return rejected(AdmissionStop::ScopeInvalid(reason));
        }
        if frames.is_empty() {
            return Admission {
                accepted: 0,
                stop: None,
                completion: None,
            };
        }
        let may_inline = self.available_budget()
            == BudgetAvailable {
                frames: self.limits.max_frames(),
                bytes: self.limits.max_bytes(),
                operations: self.limits.max_operations(),
            };
        let Ok(operation_permit) = self.operation_budget.clone().try_acquire_owned() else {
            return rejected(AdmissionStop::OperationBudget);
        };
        // Reserve BEFORE transferring payloads or attempting I/O. An owned
        // permit can publish even after Receiver::close, and prevents the
        // worker's close/drain from finishing before this handoff.
        let queue = self
            .queue
            .as_ref()
            .expect("open sender has a queue")
            .clone();
        let queue_permit = match queue.try_reserve_owned() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Closed(_)) => return rejected(AdmissionStop::Closed),
            Err(mpsc::error::TrySendError::Full(_)) => {
                return rejected(AdmissionStop::OperationBudget);
            }
        };
        let available = self.available_budget();
        let mut accepted = 0;
        let mut bytes = 0;
        let mut stop = None;
        for (index, frame) in frames
            .iter()
            .take(self.limits.max_batch_frames() + 1)
            .enumerate()
        {
            stop = self.frame_stop(frame, index, bytes, available);
            if stop.is_some() {
                break;
            }
            bytes += frame.payload.capacity();
            accepted = index + 1;
        }
        if accepted == 0 {
            return rejected(stop.expect("nonempty batch has a rejection reason"));
        }
        // Admission is exclusive (&mut self); the worker only returns permits.
        // The validated caps make the following conversions representable.
        let frame_permit = self
            .frame_budget
            .clone()
            .try_acquire_many_owned(u32::try_from(accepted).expect("validated frame budget"))
            .expect("exclusive admission owns available frame permits");
        let byte_permit = self
            .byte_budget
            .clone()
            .try_acquire_many_owned(u32::try_from(bytes).expect("validated byte budget"))
            .expect("exclusive admission owns available byte permits");
        let (completion, result) = oneshot::channel();
        let mut operation = Operation {
            batch_io,
            frames: frames.drain(..accepted).collect(),
            completed: VecDeque::with_capacity(accepted),
            progress: Progress::new(&self.io),
            terminal: None,
            context,
            completion,
            reservation: Reservation {
                _frames: frame_permit,
                _bytes: byte_permit,
                _operation: operation_permit,
            },
        };
        if may_inline && let Ok(permit) = self.idle.clone().try_acquire_owned() {
            let lease = WriterLease::new(self.io.clone(), permit);
            let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
            if operation.poll(&self.io, &mut cx).is_ready() {
                operation.finish();
                drop(queue_permit);
            } else {
                // No borrowed future/buffer is lost. Publication wakes the
                // worker, which repolls the exact cursor with a real waker.
                // Even an unexpectedly aborted receiver cannot turn attempted
                // frames back into a rejected suffix: that yields CompletionLost.
                queue_permit.send(operation);
            }
            lease.release();
        } else {
            queue_permit.send(operation);
        }
        Admission {
            accepted,
            stop,
            completion: Some(Completion(result)),
        }
    }

    fn frame_stop(
        &self,
        frame: &Frame,
        count: usize,
        bytes: usize,
        available: BudgetAvailable,
    ) -> Option<AdmissionStop> {
        if count == self.limits.max_batch_frames() {
            return Some(AdmissionStop::BatchLimit);
        }
        if frame.payload.is_empty() {
            return Some(AdmissionStop::EmptyFrame);
        }
        if frame.payload.len() > self.limits.max_frame_bytes() {
            return Some(AdmissionStop::Oversized {
                size: frame.payload.len(),
                maximum: self.limits.max_frame_bytes(),
            });
        }
        match (self.io.peer, frame.peer) {
            (None, None) => return Some(AdmissionStop::MissingPeer),
            (Some(expected), Some(actual)) if expected != actual => {
                return Some(AdmissionStop::PeerMismatch);
            }
            _ => {}
        }
        if count == available.frames {
            return Some(AdmissionStop::FrameBudget);
        }
        if frame.payload.capacity() > available.bytes - bytes {
            return Some(AdmissionStop::ByteBudget);
        }
        None
    }

    /// Stops admission and cancels blocked I/O, drains accepted work into
    /// completions, closes both raw halves, and joins the owner. Cancellation
    /// of this shutdown future is safe: the JoinHandle stays in this sender.
    /// Caught provider panics are reported as `SendFailure::ProviderPanicked`
    /// in completions; they do not make this join fail. Unexpected task
    /// aborts/unwinds can still return a JoinError and lose completions.
    pub async fn shutdown(&mut self) -> Result<(), tokio::task::JoinError> {
        self.io.close();
        self.queue.take();
        let result = match self.worker.as_mut() {
            Some(worker) => worker.await,
            None => Ok(()),
        };
        self.worker.take();
        result
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        // The task only polls I/O through Shared; after close returns there
        // can be no background socket access, even before abort is observed.
        self.io.close();
        if let Some(worker) = &self.worker {
            worker.abort();
        }
    }
}

async fn run(io: Arc<Shared>, mut queue: mpsc::Receiver<Operation>, idle: Arc<Semaphore>) {
    struct CloseOnDrop(Arc<Shared>);
    impl Drop for CloseOnDrop {
        fn drop(&mut self) {
            self.0.close();
        }
    }
    let _close = CloseOnDrop(io.clone());
    loop {
        let operation = tokio::select! {
            biased;
            _ = io.stop.cancelled() => {
                queue.close();
                queue.recv().await
            }
            operation = queue.recv() => operation,
        };
        let Some(mut operation) = operation else {
            break;
        };
        // Install cleanup before the await as well: abort can happen after a
        // semaphore grant but before its future resumes. Budgets must not be
        // released before the carrier closes in that window either.
        let mut lease = WriterLease::waiting(io.clone());
        let permit = idle
            .clone()
            .acquire_owned()
            .await
            .expect("writer semaphore is never closed");
        lease._permit = Some(permit);
        let context = operation.context.clone();
        let invalidated = context.invalidated();
        let stopped = io.stop.cancelled();
        tokio::pin!(invalidated, stopped);
        let mut invalid_ready = false;
        let mut stop_ready = false;
        poll_fn(|cx| {
            // Register scope/deadline and shutdown wakeups even when carrier
            // I/O is indefinitely Pending. A wake before handoff is harmless:
            // the engine also checks the current scope synchronously.
            if !invalid_ready {
                match catch_unwind(AssertUnwindSafe(|| invalidated.as_mut().poll(cx))) {
                    Ok(state) => invalid_ready = state.is_ready(),
                    Err(_panic) => {
                        invalid_ready = true;
                        operation.fail(&io, SendFailure::ProviderPanicked);
                    }
                }
            }
            if !stop_ready {
                stop_ready = stopped.as_mut().poll(cx).is_ready();
            }
            operation.poll(&io, cx)
        })
        .await;
        operation.finish();
        lease.release();
    }
}
