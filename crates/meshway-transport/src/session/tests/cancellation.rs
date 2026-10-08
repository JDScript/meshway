use super::*;

#[tokio::test]
async fn scoped_batch_earliest_deadline_closes_partial_stream_and_returns_buffers() {
    use crate::{BatchOutcome, ChannelScope, ScopedFrame};
    let (stream, _remote) = tokio::io::duplex(1);
    let mut session = Session::stream(stream, peer(), limits());
    let scope = ChannelScope::new();
    let mut frames: VecDeque<_> = batch(&[b"first", b"second"])
        .into_iter()
        .enumerate()
        .map(|(index, frame)| ScopedFrame {
            frame,
            scope: scope.clone(),
            context: SendContext {
                deadline: Some(
                    Instant::now() + Duration::from_millis(if index == 0 { 10000 } else { 20 }),
                ),
                ..SendContext::default()
            },
        })
        .collect();
    let pointers: Vec<_> = frames
        .iter()
        .map(|frame| frame.frame.payload.as_ptr())
        .collect();
    let admission = session
        .sender()
        .submit_batch(&mut frames, SendContext::default());
    let result = timeout(Duration::from_secs(1), admission.completion.unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.outcome(), BatchOutcome::Uncertain(_)));
    assert_eq!(
        result
            .frames
            .iter()
            .map(|frame| frame.frame.payload.as_ptr())
            .collect::<Vec<_>>(),
        pointers
    );
    assert!(session.sender().is_closed());
    assert_eq!(
        session.sender().available_budget().frames,
        limits().max_frames()
    );
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn scoped_datagram_batch_retains_every_cancellation_after_partial_completion() {
    use crate::{BatchOutcome, ChannelScope, ScopedFrame};
    struct Endpoint(Arc<AtomicUsize>);
    impl crate::raw::Datagram for Endpoint {
        fn local_addr(&self) -> io::Result<SocketAddr> {
            Ok(peer())
        }
        fn poll_send_to(
            &self,
            _: &mut Context<'_>,
            _: &[u8],
            _: SocketAddr,
        ) -> Poll<io::Result<usize>> {
            panic!("batch must use batch poll")
        }
        fn poll_recv_from(
            &self,
            _: &mut Context<'_>,
            _: &mut [u8],
        ) -> Poll<io::Result<crate::raw::DatagramRead>> {
            Poll::Pending
        }
        fn poll_send_batch(
            &self,
            _: &mut Context<'_>,
            frames: &[crate::raw::DatagramSend<'_>],
        ) -> Poll<io::Result<usize>> {
            if self.0.load(Ordering::SeqCst) == 0 {
                assert_eq!(frames.len(), 2);
                self.0.fetch_add(1, Ordering::SeqCst);
                Poll::Ready(Ok(1))
            } else {
                Poll::Pending
            }
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mut session = Session::datagram(Endpoint(calls.clone()), limits());
    let scope = ChannelScope::new();
    let last = SendContext::default();
    let mut frames: VecDeque<_> = [SendContext::default(), last.clone()]
        .into_iter()
        .map(|context| ScopedFrame {
            frame: Frame::new(b"payload".to_vec(), Some(peer())),
            scope: scope.clone(),
            context,
        })
        .collect();
    let admission = session
        .sender()
        .submit_batch(&mut frames, SendContext::default());
    last.cancellation.cancel();
    let result = timeout(Duration::from_secs(1), admission.completion.unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.outcome(), BatchOutcome::Uncertain(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        session.sender().available_budget().frames,
        limits().max_frames()
    );
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn wait_capacity_wakes_closed_when_receive_half_closes_blocked_carrier() {
    let configured = Limits::new(16, 1, 16, 1, 1).unwrap();
    let (stream, _remote) = tokio::io::duplex(1);
    let (mut sender, receiver) = Session::stream(stream, peer(), configured).into_split();
    let mut frames = batch(&[b"0123456789abcdef"]);
    let admitted = sender.try_send(&mut frames, SendContext::default());
    assert_eq!(admitted.accepted, 1);
    let (waiting, started) = tokio::sync::oneshot::channel();
    let ready = tokio::spawn(async move {
        let mut ready = Box::pin(sender.wait_capacity(16));
        assert!(futures_util::poll!(&mut ready).is_pending());
        waiting.send(()).unwrap();
        let result = ready.await;
        (sender, result)
    });
    timeout(Duration::from_secs(2), started)
        .await
        .unwrap()
        .unwrap();
    // Receiver's public close boundary is Drop; it closes the shared carrier
    // without borrowing Sender while wait_capacity retains that borrow.
    drop(receiver);
    let (mut sender, result) = timeout(Duration::from_secs(2), ready)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result, Err(AdmissionStop::Closed));
    assert!(sender.is_closed());
    let completed = timeout(Duration::from_secs(2), admitted.completion.unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        completed.frames[0].outcome,
        SendOutcome::Indeterminate(SendFailure::Cancelled(CancelReason::SessionClosed))
    ));
    assert_eq!(
        sender.available_budget(),
        BudgetAvailable {
            frames: 1,
            bytes: 16,
            operations: 1
        }
    );
    sender.shutdown().await.unwrap();
}

#[tokio::test]
async fn dropping_completion_waiter_does_not_cancel_owned_send() {
    let (stream, mut remote) = tokio::io::duplex(1);
    let mut session = Session::stream(stream, peer(), limits());
    let mut first = batch(&[b"one"]);
    drop(
        session
            .sender()
            .try_send(&mut first, SendContext::default())
            .completion,
    );
    let mut second = batch(&[b"two"]);
    let completion = session
        .sender()
        .try_send(&mut second, SendContext::default())
        .completion
        .unwrap();
    let expected = wire(&[b"one", b"two"]);
    let mut received = vec![0; expected.len()];
    remote.read_exact(&mut received).await.unwrap();
    assert_eq!(received, expected);
    assert!(matches!(
        completion.wait().await.unwrap().frames[0].outcome,
        SendOutcome::LocalComplete
    ));
    assert_eq!(
        session.sender().available_budget().operations,
        limits().max_operations()
    );
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn expired_queued_scope_is_not_started_and_stream_remains_usable() {
    let (stream, mut remote) = tokio::io::duplex(1);
    let mut session = Session::stream(stream, peer(), limits());
    let mut first = batch(&[b"first"]);
    let first = session
        .sender()
        .try_send(&mut first, SendContext::default());
    let deadline = Instant::now() + Duration::from_millis(30);
    let mut second = batch(&[b"expired"]);
    let second = session.sender().try_send(
        &mut second,
        SendContext {
            deadline: Some(deadline),
            ..SendContext::default()
        },
    );
    assert_eq!(second.accepted, 1);
    tokio::time::sleep_until(deadline).await;
    let mut bytes = vec![0; wire(&[b"first"]).len()];
    remote.read_exact(&mut bytes).await.unwrap();
    first.completion.unwrap().wait().await.unwrap();
    let second = second.completion.unwrap().wait().await.unwrap();
    assert!(matches!(
        second.frames[0].outcome,
        SendOutcome::NotStarted(SendFailure::Cancelled(CancelReason::DeadlineExceeded))
    ));
    assert!(!session.sender().is_closed());
    let mut third = batch(&[b"third"]);
    let third = session
        .sender()
        .try_send(&mut third, SendContext::default());
    let mut bytes = vec![0; wire(&[b"third"]).len()];
    remote.read_exact(&mut bytes).await.unwrap();
    assert_eq!(bytes, wire(&[b"third"]));
    third.completion.unwrap().wait().await.unwrap();
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn hard_cancel_of_partial_write_closes_carrier_and_cancels_suffix() {
    let (stream, mut remote) = tokio::io::duplex(1);
    let payload_size = 96 * 1024;
    let configured = Limits::new(payload_size, 6, 6 * payload_size, 2, 5).unwrap();
    let mut session = Session::stream(stream, peer(), configured);
    let context = SendContext::default();
    let mut frames = (0..5)
        .map(|index| Frame::new(vec![index; payload_size], None))
        .collect();
    let admission = session.sender().try_send(&mut frames, context.clone());
    let mut queued = batch(&[b"untouched queued operation"]);
    let queued = session
        .sender()
        .try_send(&mut queued, SendContext::default());
    remote.read_exact(&mut [0; 1]).await.unwrap();
    context.cancellation.cancel();
    let completion = admission.completion.unwrap().wait().await.unwrap();
    // Two 96 KiB frames fit in the 256 KiB group. Both were staged before
    // flushing, so neither can be called definitely unsent from raw counts.
    for completed in completion.frames.iter().take(2) {
        assert!(matches!(
            completed.outcome,
            SendOutcome::Indeterminate(SendFailure::Cancelled(CancelReason::ScopeCancelled))
        ));
    }
    for completed in completion.frames.iter().skip(2) {
        assert!(matches!(
            completed.outcome,
            SendOutcome::NotStarted(SendFailure::Cancelled(CancelReason::SessionClosed))
        ));
    }
    assert!(matches!(
        queued.completion.unwrap().wait().await.unwrap().frames[0].outcome,
        SendOutcome::NotStarted(SendFailure::Cancelled(CancelReason::SessionClosed))
    ));
    assert!(session.sender().is_closed());
    let mut more = batch(&[b"no reuse"]);
    assert_eq!(
        session
            .sender()
            .try_send(&mut more, SendContext::default())
            .stop,
        Some(AdmissionStop::Closed)
    );
    let mut remainder = Vec::new();
    timeout(Duration::from_secs(2), remote.read_to_end(&mut remainder))
        .await
        .unwrap()
        .unwrap();
    assert!(remainder.len() <= 1);
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_cancels_blocked_write_joins_and_returns_all_payloads() {
    let (stream, mut remote) = tokio::io::duplex(1);
    let mut session = Session::stream(stream, peer(), limits());
    let mut first = batch(&[b"blocked"]);
    let first = session
        .sender()
        .try_send(&mut first, SendContext::default());
    let mut second = batch(&[b"queued"]);
    let second = session
        .sender()
        .try_send(&mut second, SendContext::default());
    remote.read_exact(&mut [0; 1]).await.unwrap();
    timeout(Duration::from_secs(2), session.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        first.completion.unwrap().wait().await.unwrap().frames[0].outcome,
        SendOutcome::Indeterminate(_)
    ));
    assert!(matches!(
        second.completion.unwrap().wait().await.unwrap().frames[0].outcome,
        SendOutcome::NotStarted(_)
    ));
    assert_eq!(
        session.sender().available_budget(),
        BudgetAvailable {
            frames: 4,
            bytes: 256,
            operations: 3
        }
    );
    assert!(matches!(
        session.receiver().recv().await,
        Err(ReceiveError::Closed)
    ));
}

#[tokio::test]
async fn sender_drop_synchronously_closes_both_halves_before_task_abort() {
    let (stream, mut remote) = tokio::io::duplex(1);
    let dropped = Arc::new(AtomicBool::new(false));
    let observed = DropObserved {
        stream,
        dropped: dropped.clone(),
        flush_started: None,
        block_flush: false,
    };
    let (mut sender, mut receiver) = Session::stream(observed, peer(), limits()).into_split();
    let mut frames = batch(&[b"blocked"]);
    let completion = sender
        .try_send(&mut frames, SendContext::default())
        .completion
        .unwrap();
    remote.read_exact(&mut [0; 1]).await.unwrap();
    drop(sender);
    assert!(dropped.load(Ordering::SeqCst));
    assert!(matches!(receiver.recv().await, Err(ReceiveError::Closed)));
    assert!(completion.wait().await.is_err());
}

#[tokio::test]
async fn flush_is_part_of_completion_and_hard_cancellation_closes_carrier() {
    let (stream, mut remote) = tokio::io::duplex(128);
    let (flush_started, started) = tokio::sync::oneshot::channel();
    let dropped = Arc::new(AtomicBool::new(false));
    let observed = DropObserved {
        stream,
        dropped: dropped.clone(),
        flush_started: Some(flush_started),
        block_flush: true,
    };
    let mut session = Session::stream(observed, peer(), limits());
    let context = SendContext::default();
    let mut frames = batch(&[b"needs flush", b"same staged group"]);
    let completion = session
        .sender()
        .try_send(&mut frames, context.clone())
        .completion
        .unwrap();
    started.await.unwrap();
    let expected = wire(&[b"needs flush", b"same staged group"]);
    let mut received = vec![0; expected.len()];
    timeout(Duration::from_secs(2), remote.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received, expected);
    let mut completion = Box::pin(completion.wait());
    assert!(futures_util::poll!(&mut completion).is_pending());
    context.cancellation.cancel();
    for completed in completion.await.unwrap().frames {
        assert!(matches!(
            completed.outcome,
            SendOutcome::Indeterminate(SendFailure::Cancelled(CancelReason::ScopeCancelled))
        ));
    }
    assert!(dropped.load(Ordering::SeqCst));
    assert!(session.sender().is_closed());
    session.shutdown().await.unwrap();
}

#[test]
fn completed_frame_debug_keeps_payload_contents_private() {
    let completed = super::super::CompletedFrame {
        frame: crate::Frame::new(vec![17, 29, 43, 71], None),
        outcome: super::super::SendOutcome::LocalComplete,
    };
    for debug in [format!("{completed:?}"), format!("{completed:#?}")] {
        assert!(debug.contains("payload_len"));
        assert!(!debug.contains("17"));
        assert!(!debug.contains("29"));
        assert!(!debug.contains("43"));
        assert!(!debug.contains("71"));
    }
}

#[tokio::test]
async fn provider_panic_is_opaque_and_handled_in_both_inline_and_deferred_polls() {
    struct PanicsOnWrite {
        dropped: Arc<AtomicBool>,
        defer: bool,
    }
    impl AsyncRead for PanicsOnWrite {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }
    impl AsyncWrite for PanicsOnWrite {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<io::Result<usize>> {
            if self.defer {
                self.defer = false;
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            panic!("private provider fixture panic body")
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
    impl Drop for PanicsOnWrite {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }
    for defer in [false, true] {
        let dropped = Arc::new(AtomicBool::new(false));
        let mut session = Session::stream(
            PanicsOnWrite {
                dropped: dropped.clone(),
                defer,
            },
            peer(),
            limits(),
        );
        let mut frames = batch(&[b"owned"]);
        let original = frames[0].payload.as_ptr();
        // A provider panic must never escape this synchronous admission call.
        let submitted = session
            .sender()
            .try_send(&mut frames, SendContext::default());
        assert_eq!(submitted.accepted, 1);
        assert!(frames.is_empty());
        let completed = timeout(Duration::from_secs(2), submitted.completion.unwrap().wait())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            completed.frames[0].outcome,
            SendOutcome::Indeterminate(SendFailure::ProviderPanicked)
        ));
        assert_eq!(completed.frames[0].frame.payload.as_ptr(), original);
        assert!(!format!("{:?}", completed.frames[0].outcome).contains("private"));
        assert!(session.sender().is_closed());
        assert!(matches!(
            session.receiver().recv().await,
            Err(ReceiveError::Closed)
        ));
        session.shutdown().await.unwrap();
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(
            session.sender().available_budget().bytes,
            limits().max_bytes()
        );
    }
}

#[tokio::test]
async fn cancellation_during_cooperative_staging_discards_bytes_without_poisoning_stream() {
    let configured = Limits::new(1, 1026, 1026, 2, 1024).unwrap();
    let (observed, mut remote, observation) = observed_stream(configured.stream_buffer_bytes());
    let mut session = Session::stream(observed, peer(), configured);
    let context = SendContext::default();
    let mut frames = (0..1024).map(|_| Frame::new(vec![7], None)).collect();
    let submitted = session.sender().try_send(&mut frames, context.clone());
    assert_eq!(submitted.accepted, 1024);
    // Admission stages only its bounded quantum and hands off the cursor.
    assert_eq!(observation.lock().unwrap().write_polls, 0);
    let mut valid = batch(&[&[9], &[9]]);
    let valid = session
        .sender()
        .try_send(&mut valid, SendContext::default());
    assert_eq!(valid.accepted, 2);
    context.cancellation.cancel();
    let completed = submitted.completion.unwrap().wait().await.unwrap();
    assert_eq!(completed.frames.len(), 1024);
    assert!(completed.frames.iter().all(|frame| matches!(
        frame.outcome,
        SendOutcome::NotStarted(SendFailure::Cancelled(CancelReason::ScopeCancelled))
    )));
    assert!(!session.sender().is_closed());

    // The already-queued other scope must not flush any cancelled staged bytes.
    let completed = valid.completion.unwrap().wait().await.unwrap();
    assert!(
        completed
            .frames
            .iter()
            .all(|frame| matches!(frame.outcome, SendOutcome::LocalComplete))
    );
    let expected = wire(&[&[9], &[9]]);
    assert_eq!(observation.lock().unwrap().flush_sizes, [expected.len()]);
    let mut received = vec![0; expected.len()];
    remote.read_exact(&mut received).await.unwrap();
    assert_eq!(received, expected);
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn write_error_after_provider_buffering_marks_whole_group_uncertain() {
    struct BuffersThenFails(Arc<AtomicBool>, bool);
    impl AsyncRead for BuffersThenFails {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }
    impl AsyncWrite for BuffersThenFails {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            if !self.1 {
                self.1 = true;
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            // A generic provider can process/buffer bytes and then fail; no
            // successful write count is available to narrow the uncertainty.
            assert_eq!(buffer, wire(&[b"one", b"two"]));
            self.0.store(true, Ordering::SeqCst);
            Poll::Ready(Err(io::ErrorKind::ConnectionReset.into()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            panic!("write failed before raw flush")
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
    let attempted = Arc::new(AtomicBool::new(false));
    let mut session = Session::stream(BuffersThenFails(attempted.clone(), false), peer(), limits());
    let mut frames = batch(&[b"one", b"two"]);
    let first = session
        .sender()
        .try_send(&mut frames, SendContext::default());
    let mut frames = batch(&[b"untouched"]);
    let queued = session
        .sender()
        .try_send(&mut frames, SendContext::default());
    let completed = first.completion.unwrap().wait().await.unwrap();
    let errors: Vec<_> = completed
        .frames
        .iter()
        .map(|frame| {
            let SendOutcome::Indeterminate(SendFailure::Io(error)) = &frame.outcome else {
                panic!("whole staged group must be uncertain");
            };
            assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
            error
        })
        .collect();
    assert!(Arc::ptr_eq(errors[0], errors[1]));
    assert!(attempted.load(Ordering::SeqCst));
    assert!(session.sender().is_closed());
    assert!(matches!(
        queued.completion.unwrap().wait().await.unwrap().frames[0].outcome,
        SendOutcome::NotStarted(_)
    ));
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn deadline_after_inline_pending_wakes_a_carrier_with_no_further_io_readiness() {
    let (observed, mut remote, observation) = observed_stream(1);
    let mut session = Session::stream(observed, peer(), limits());
    let context = SendContext {
        deadline: Some(Instant::now() + Duration::from_millis(100)),
        ..SendContext::default()
    };
    let mut frames = batch(&[b"blocked until deadline"]);
    let submitted = session.sender().try_send(&mut frames, context);
    assert_eq!(observation.lock().unwrap().bytes_since_flush, 1);
    // Do not drain the peer or otherwise wake the raw writer.
    let completed = timeout(Duration::from_secs(2), submitted.completion.unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        completed.frames[0].outcome,
        SendOutcome::Indeterminate(SendFailure::Cancelled(CancelReason::DeadlineExceeded))
    ));
    assert!(session.sender().is_closed());
    let mut remainder = Vec::new();
    remote.read_to_end(&mut remainder).await.unwrap();
    assert_eq!(remainder.len(), 1);
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn inline_invalidation_keeps_attempted_frames_accepted_through_closed_queue_drain() {
    struct InvalidatesDuringPoll(Arc<Mutex<Option<tokio_util::sync::CancellationToken>>>);
    impl AsyncRead for InvalidatesDuringPoll {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }
    impl AsyncWrite for InvalidatesDuringPoll {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.0.lock().unwrap().as_ref().unwrap().cancel();
            Poll::Pending
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
    let token = Arc::new(Mutex::new(None));
    let io = Arc::new(super::super::io::Shared::stream(
        InvalidatesDuringPoll(token.clone()),
        peer(),
        limits(),
    ));
    *token.lock().unwrap() = Some(io.stop.clone());
    let mut sender = super::super::Sender::start(io, limits());
    let mut frames = batch(&[b"accepted despite concurrent close"]);
    let pointer = frames[0].payload.as_ptr();
    let submitted = sender.try_send(&mut frames, SendContext::default());
    assert!(sender.is_closed());
    assert_eq!(submitted.accepted, 1);
    assert!(frames.is_empty());
    // Shutdown closes/drains the worker queue. The pre-I/O reservation makes
    // handoff irrevocable; attempted payloads never reappear as caller suffix.
    sender.shutdown().await.unwrap();
    let completed = submitted.completion.unwrap().wait().await.unwrap();
    assert_eq!(completed.frames[0].frame.payload.as_ptr(), pointer);
    assert!(matches!(
        completed.frames[0].outcome,
        SendOutcome::Indeterminate(SendFailure::Cancelled(CancelReason::SessionClosed))
    ));
    assert_eq!(
        sender.available_budget().operations,
        limits().max_operations()
    );
}

#[tokio::test]
async fn successful_short_write_cancellation_prevents_every_subsequent_raw_call() {
    for vectored in [false, true] {
        let context = SendContext::default();
        let observation = Arc::new(Mutex::new(ShortWriteObservation::default()));
        let stream = AlwaysShortWriter {
            observation: observation.clone(),
            cancel_on_write: Some(context.cancellation.clone()),
            vectored,
        };
        let mut session = Session::stream(stream, peer(), limits());
        let mut frames = batch(&[b"first", b"second"]);
        let accepted = session.sender().try_send(&mut frames, context);
        assert_eq!(accepted.accepted, 2);
        let completed = accepted.completion.unwrap().wait().await.unwrap();
        assert!(completed.frames.iter().all(|frame| matches!(
            frame.outcome,
            SendOutcome::Indeterminate(SendFailure::Cancelled(CancelReason::ScopeCancelled))
        )));
        assert_eq!(observation.lock().unwrap().writes, 1);
        assert_eq!(observation.lock().unwrap().flushes, 0);
        assert!(session.sender().is_closed());
        session.shutdown().await.unwrap();
    }
}
