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

#[tokio::test]
async fn stream_receive_batch_keeps_partial_codec_and_delivers_prefix_before_eof() {
    let (stream, mut remote) = tokio::io::duplex(128);
    let mut session = Session::stream(stream, peer(), limits());
    remote.write_all(&[0, 0]).await.unwrap();
    {
        let mut read = Box::pin(session.receiver().recv_batch());
        assert!(futures_util::poll!(&mut read).is_pending());
    }
    let encoded = wire(&[b"one", b"two"]);
    remote.write_all(&encoded[2..]).await.unwrap();
    remote.shutdown().await.unwrap();
    let frames = session.receiver().recv_batch().await.unwrap();
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.payload.as_slice())
            .collect::<Vec<_>>(),
        [b"one", b"two"]
    );
    assert!(matches!(
        session.receiver().recv().await,
        Err(ReceiveError::Closed)
    ));
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn stream_receive_batch_preserves_next_frame_at_byte_boundary() {
    let (stream, mut remote) = tokio::io::duplex(100_000);
    let mut session = Session::stream(
        stream,
        peer(),
        Limits::new(50_000, 3, 150_000, 1, 3).unwrap(),
    );
    remote
        .write_all(&wire(&[&vec![1; 40_000], &vec![2; 40_000]]))
        .await
        .unwrap();
    let frames = session.receiver().recv_batch().await.unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].payload, vec![1; 40_000]);
    assert_eq!(
        session.receiver().recv().await.unwrap().payload,
        vec![2; 40_000]
    );
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn scoped_batch_preserves_scope_boundary_and_original_suffix() {
    use crate::{BatchOutcome, ChannelScope, ScopedFrame};
    let (stream, mut remote) = tokio::io::duplex(128);
    let mut session = Session::stream(stream, peer(), limits());
    let scope = ChannelScope::new();
    let mut frames: VecDeque<_> = batch(&[b"first", b"second", b"suffix"])
        .into_iter()
        .enumerate()
        .map(|(index, frame)| ScopedFrame {
            frame,
            scope: if index == 2 {
                ChannelScope::new()
            } else {
                scope.clone()
            },
            context: SendContext::default(),
        })
        .collect();
    let pointer = frames[2].frame.payload.as_ptr();
    let admission = session
        .sender()
        .submit_batch(&mut frames, SendContext::default());
    assert_eq!(admission.accepted, 2);
    assert_eq!(admission.stop, Some(AdmissionStop::ChannelScopeMismatch));
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].frame.payload.as_ptr(), pointer);
    assert!(matches!(
        admission
            .completion
            .unwrap()
            .wait()
            .await
            .unwrap()
            .outcome(),
        BatchOutcome::LocalComplete
    ));
    let expected = wire(&[b"first", b"second"]);
    let mut bytes = vec![0; expected.len()];
    remote.read_exact(&mut bytes).await.unwrap();
    assert_eq!(bytes, expected);
    session.shutdown().await.unwrap();
}

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

#[test]
fn rejects_unrepresentable_or_inconsistent_budgets() {
    assert!(Limits::new(0, 1, 1, 1, 1).is_err());
    assert!(Limits::new(usize::MAX, 1, usize::MAX, 1, 1).is_err());
    assert!(Limits::new(64, 1, 63, 1, 1).is_err());
    assert!(Limits::new(64, 1, 64, 2, 1).is_err());
    assert!(Limits::new(64, 1, 64, 1, 2).is_err());
    assert!(Limits::new(64, usize::MAX, 64, 1, 1).is_err());
}

#[tokio::test]
async fn wait_capacity_wakes_after_send_and_restores_all_budgets_and_original_frame() {
    let configured = Limits::new(16, 1, 16, 1, 1).unwrap();
    let (stream, mut remote) = tokio::io::duplex(1);
    let (mut sender, receiver) = Session::stream(stream, peer(), configured).into_split();
    let mut frames = batch(&[b"0123456789abcdef"]);
    let original = frames[0].payload.as_ptr();
    let admitted = sender.try_send(&mut frames, SendContext::default());
    assert_eq!(admitted.accepted, 1);
    assert!(frames.is_empty());
    assert_eq!(
        sender.available_budget(),
        BudgetAvailable {
            frames: 0,
            bytes: 0,
            operations: 0
        }
    );
    let (waiting, started) = tokio::sync::oneshot::channel();
    let ready = tokio::spawn(async move {
        let mut ready = Box::pin(sender.wait_capacity(16));
        assert!(futures_util::poll!(&mut ready).is_pending());
        waiting.send(()).unwrap();
        ready.await.unwrap();
        sender
    });
    timeout(Duration::from_secs(2), started)
        .await
        .unwrap()
        .unwrap();
    let mut received = [0; 20];
    timeout(Duration::from_secs(2), remote.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.as_slice(), wire(&[b"0123456789abcdef"]));
    // The capacity waiter has its own task/waker: draining the peer must wake
    // it, rather than a later caller poll merely observing restored budgets.
    let mut sender = timeout(Duration::from_secs(2), ready)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        sender.available_budget(),
        BudgetAvailable {
            frames: 1,
            bytes: 16,
            operations: 1
        }
    );
    let completed = timeout(Duration::from_secs(2), admitted.completion.unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(completed.frames.len(), 1);
    assert_eq!(completed.frames[0].frame.payload.as_ptr(), original);
    assert_eq!(completed.frames[0].frame.payload, b"0123456789abcdef");
    assert_eq!(completed.frames[0].frame.peer, None);
    assert!(matches!(
        completed.frames[0].outcome,
        SendOutcome::LocalComplete
    ));
    sender.shutdown().await.unwrap();
    drop(receiver);
}

#[tokio::test]
async fn wait_capacity_rejects_impossible_allocation_without_waiting() {
    let configured = Limits::new(16, 1, 16, 1, 1).unwrap();
    let (stream, _remote) = tokio::io::duplex(1);
    let mut session = Session::stream(stream, peer(), configured);
    {
        let mut ready = Box::pin(session.sender().wait_capacity(17));
        assert_eq!(
            futures_util::poll!(&mut ready),
            Poll::Ready(Err(AdmissionStop::ByteBudget))
        );
    }
    assert_eq!(
        session.sender().available_budget(),
        BudgetAvailable {
            frames: 1,
            bytes: 16,
            operations: 1
        }
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
async fn receive_is_cancellation_safe_across_partial_header_and_payload() {
    let (stream, mut remote) = tokio::io::duplex(128);
    let mut session = Session::stream(stream, peer(), limits());
    let input = wire(&[b"hello", b"world"]);
    remote.write_all(&input[..2]).await.unwrap();
    let mut receive = Box::pin(session.receiver().recv());
    assert!(futures_util::poll!(&mut receive).is_pending());
    drop(receive);
    remote.write_all(&input[2..7]).await.unwrap();
    let context = SendContext::default();
    let mut receive = Box::pin(session.receiver().recv_with_context(&context));
    assert!(futures_util::poll!(&mut receive).is_pending());
    context.cancellation.cancel();
    assert!(matches!(
        receive.await,
        Err(ReceiveError::Cancelled(CancelReason::ScopeCancelled))
    ));
    remote.write_all(&input[7..]).await.unwrap();
    assert_eq!(session.receiver().recv().await.unwrap().payload, b"hello");
    assert_eq!(session.receiver().recv().await.unwrap().payload, b"world");
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_stream_lengths_close_carrier_without_reading_body() {
    for header in [u32::MAX.to_be_bytes(), 0_u32.to_be_bytes()] {
        let (stream, mut remote) = tokio::io::duplex(16);
        let mut session = Session::stream(stream, peer(), limits());
        remote.write_all(&header).await.unwrap();
        // A maximal bogus header must fail immediately with only a header
        // present: no oversized allocation or wait for an impossible body.
        let result = timeout(Duration::from_secs(2), session.receiver().recv())
            .await
            .unwrap();
        if header == u32::MAX.to_be_bytes() {
            assert!(matches!(result, Err(ReceiveError::Oversized)));
        } else {
            assert!(matches!(result, Err(ReceiveError::Empty)));
        }
        assert!(session.sender().is_closed());
        assert!(matches!(
            session.receiver().recv().await,
            Err(ReceiveError::Closed)
        ));
        session.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn incomplete_frame_eof_is_not_a_complete_frame() {
    let (stream, mut remote) = tokio::io::duplex(32);
    let mut session = Session::stream(stream, peer(), limits());
    remote.write_all(&[0, 0, 0, 3, 1]).await.unwrap();
    remote.shutdown().await.unwrap();
    assert!(matches!(
        session.receiver().recv().await,
        Err(ReceiveError::Io(_))
    ));
    assert!(session.sender().is_closed());
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn prefix_admission_restores_frame_byte_and_operation_budgets_and_buffers() {
    let configured = Limits::new(16, 2, 32, 1, 2).unwrap();
    let (stream, mut remote) = tokio::io::duplex(1);
    let mut session = Session::stream(stream, peer(), configured);
    let mut payload = Vec::with_capacity(16);
    payload.extend_from_slice(b"one");
    let original = payload.as_ptr();
    let mut frames = VecDeque::from([
        Frame::new(payload, None),
        Frame::new(b"two".to_vec(), None),
        Frame::new(b"three".to_vec(), None),
    ]);
    let admission = session
        .sender()
        .try_send(&mut frames, SendContext::default());
    assert_eq!(admission.accepted, 2);
    assert_eq!(admission.stop, Some(AdmissionStop::BatchLimit));
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].payload, b"three");
    assert_eq!(
        session.sender().available_budget(),
        BudgetAvailable {
            frames: 0,
            bytes: 13,
            operations: 0
        }
    );
    let rejected = session
        .sender()
        .try_send(&mut frames, SendContext::default());
    assert_eq!(rejected.stop, Some(AdmissionStop::OperationBudget));
    let expected = wire(&[b"one", b"two"]);
    let mut received = vec![0; expected.len()];
    remote.read_exact(&mut received).await.unwrap();
    assert_eq!(received, expected);
    let completed = admission.completion.unwrap().wait().await.unwrap();
    assert_eq!(completed.frames[0].frame.payload.as_ptr(), original);
    assert!(
        completed
            .frames
            .iter()
            .all(|frame| matches!(frame.outcome, SendOutcome::LocalComplete))
    );
    assert_eq!(
        session.sender().available_budget(),
        BudgetAvailable {
            frames: 2,
            bytes: 32,
            operations: 1
        }
    );
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn byte_budget_charges_capacity_and_never_takes_rejected_suffix() {
    let configured = Limits::new(16, 4, 20, 2, 4).unwrap();
    let (stream, _remote) = tokio::io::duplex(1);
    let mut session = Session::stream(stream, peer(), configured);
    let mut large = Vec::with_capacity(20);
    large.push(7);
    let mut frames = VecDeque::from([Frame::new(vec![1], None), Frame::new(large, None)]);
    let pointer = frames[1].payload.as_ptr();
    let admission = session
        .sender()
        .try_send(&mut frames, SendContext::default());
    assert_eq!(admission.accepted, 1);
    assert_eq!(admission.stop, Some(AdmissionStop::ByteBudget));
    assert_eq!(frames[0].payload.as_ptr(), pointer);
    session.shutdown().await.unwrap();
    let completion = admission.completion.unwrap().wait().await.unwrap();
    assert!(matches!(
        completion.frames[0].outcome,
        SendOutcome::Indeterminate(SendFailure::Cancelled(CancelReason::SessionClosed))
    ));
    assert_eq!(session.sender().available_budget().bytes, 20);
}

#[tokio::test]
async fn independent_frame_limit_and_invalid_input_preserve_ownership() {
    let configured = Limits::new(16, 2, 64, 2, 2).unwrap();
    let (stream, _remote) = tokio::io::duplex(1);
    let mut session = Session::stream(stream, peer(), configured);
    let mut first = batch(&[b"one", b"two"]);
    let first = session
        .sender()
        .try_send(&mut first, SendContext::default());
    let mut pending = batch(&[b"three"]);
    assert_eq!(
        session
            .sender()
            .try_send(&mut pending, SendContext::default())
            .stop,
        Some(AdmissionStop::FrameBudget)
    );
    assert_eq!(pending[0].payload, b"three");
    session.shutdown().await.unwrap();
    first.completion.unwrap().wait().await.unwrap();

    let (stream, _remote) = tokio::io::duplex(1);
    let mut session = Session::stream(stream, peer(), configured);
    for (payload, expected) in [
        (Vec::new(), AdmissionStop::EmptyFrame),
        (
            vec![0; 17],
            AdmissionStop::Oversized {
                size: 17,
                maximum: 16,
            },
        ),
    ] {
        let mut frames = VecDeque::from([Frame::new(payload, None)]);
        let pointer = frames[0].payload.as_ptr();
        let admission = session
            .sender()
            .try_send(&mut frames, SendContext::default());
        assert_eq!(admission.stop, Some(expected));
        assert_eq!(admission.accepted, 0);
        assert_eq!(frames[0].payload.as_ptr(), pointer);
    }
    session.shutdown().await.unwrap();
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
    let completed = super::CompletedFrame {
        frame: crate::Frame::new(vec![17, 29, 43, 71], None),
        outcome: super::SendOutcome::LocalComplete,
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

#[tokio::test]
async fn fitting_stream_group_has_one_flush_and_preserves_all_original_buffers() {
    let (stream, mut remote) = tokio::io::duplex(128);
    let observation = Arc::new(Mutex::new(IoObservation::default()));
    let observed = ObservedStream {
        stream,
        observation: observation.clone(),
    };
    let mut session = Session::stream(observed, peer(), limits());
    let mut frames = batch(&[b"one", b"two", b"three"]);
    let pointers: Vec<_> = frames.iter().map(|frame| frame.payload.as_ptr()).collect();
    let submitted = session
        .sender()
        .try_send(&mut frames, SendContext::default());
    let mut waiting = Box::pin(submitted.completion.unwrap().wait());
    // Current-thread runtime: no background task has been scheduled yet.
    let Poll::Ready(Ok(completed)) = futures_util::poll!(&mut waiting) else {
        panic!("ready group should complete during admission");
    };
    assert_eq!(
        session.sender().available_budget().operations,
        limits().max_operations()
    );
    for (index, completed) in completed.frames.iter().enumerate() {
        assert!(matches!(completed.outcome, SendOutcome::LocalComplete));
        assert_eq!(completed.frame.payload.as_ptr(), pointers[index]);
    }
    let expected = wire(&[b"one", b"two", b"three"]);
    assert_eq!(observation.lock().unwrap().flush_sizes, [expected.len()]);
    let mut received = vec![0; expected.len()];
    remote.read_exact(&mut received).await.unwrap();
    assert_eq!(received, expected);
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn stream_groups_respect_encoded_ceiling_and_allow_a_large_single_frame() {
    for (payload_size, count, per_group) in [(96 * 1024, 5, 2), (300 * 1024, 2, 1)] {
        let configured = Limits::new(payload_size, count, payload_size * count, 1, count).unwrap();
        let wire_size = payload_size + 4;
        let (stream, mut remote) = tokio::io::duplex(wire_size * count);
        let observation = Arc::new(Mutex::new(IoObservation::default()));
        let observed = ObservedStream {
            stream,
            observation: observation.clone(),
        };
        let mut session = Session::stream(observed, peer(), configured);
        let mut frames: VecDeque<_> = (0..count)
            .map(|index| Frame::new(vec![u8::try_from(index).unwrap(); payload_size], None))
            .collect();
        let expected = wire(
            &frames
                .iter()
                .map(|frame| frame.payload.as_slice())
                .collect::<Vec<_>>(),
        );
        let submitted = session
            .sender()
            .try_send(&mut frames, SendContext::default());
        assert_eq!(submitted.accepted, count);
        let completed = submitted.completion.unwrap().wait().await.unwrap();
        assert!(
            completed
                .frames
                .iter()
                .all(|frame| matches!(frame.outcome, SendOutcome::LocalComplete))
        );
        let mut expected_groups = vec![per_group * wire_size; count / per_group];
        if !count.is_multiple_of(per_group) {
            expected_groups.push((count % per_group) * wire_size);
        }
        {
            let observed = observation.lock().unwrap();
            assert_eq!(observed.flush_sizes, expected_groups);
            assert!(observed.maximum_write_request <= configured.stream_buffer_bytes());
            assert!(
                observed
                    .flush_sizes
                    .iter()
                    .all(|size| *size <= configured.stream_buffer_bytes())
            );
        }
        let mut received = vec![0; expected.len()];
        remote.read_exact(&mut received).await.unwrap();
        assert_eq!(received, expected);
        session.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn cancellation_during_cooperative_staging_discards_bytes_without_poisoning_stream() {
    let configured = Limits::new(1, 1026, 1026, 2, 1024).unwrap();
    let (stream, mut remote) = tokio::io::duplex(configured.stream_buffer_bytes());
    let observation = Arc::new(Mutex::new(IoObservation::default()));
    let observed = ObservedStream {
        stream,
        observation: observation.clone(),
    };
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
async fn bounded_read_ahead_decodes_coalesced_frames_from_one_raw_read() {
    let configured = Limits::new(128, 8, 512 * 1024, 2, 8).unwrap();
    assert_eq!(configured.stream_buffer_bytes(), 256 * 1024);
    let (stream, mut remote) = tokio::io::duplex(512);
    let observation = Arc::new(Mutex::new(IoObservation::default()));
    let observed = ObservedStream {
        stream,
        observation: observation.clone(),
    };
    let mut session = Session::stream(observed, peer(), configured);
    remote.write_all(&wire(&[b"one", b"two"])).await.unwrap();
    assert_eq!(session.receiver().recv().await.unwrap().payload, b"one");
    assert_eq!(session.receiver().recv().await.unwrap().payload, b"two");
    assert_eq!(observation.lock().unwrap().read_requests, [64 * 1024]);
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
async fn inline_partial_write_handoff_preserves_fifo_and_original_buffers() {
    let (stream, mut remote) = tokio::io::duplex(3);
    let observation = Arc::new(Mutex::new(IoObservation::default()));
    let observed = ObservedStream {
        stream,
        observation: observation.clone(),
    };
    let configured = Limits::new(64, 8, 512, 4, 4).unwrap();
    let mut session = Session::stream(observed, peer(), configured);
    let mut first = batch(&[b"alpha", b"beta"]);
    let pointers: Vec<_> = first.iter().map(|frame| frame.payload.as_ptr()).collect();
    let first = session
        .sender()
        .try_send(&mut first, SendContext::default());
    // The first three wire bytes were written synchronously; this is a real
    // partial-frame cursor, not just an operation waiting to be started.
    assert_eq!(observation.lock().unwrap().bytes_since_flush, 3);
    let mut second = batch(&[b"gamma", b"delta"]);
    let second = session
        .sender()
        .try_send(&mut second, SendContext::default());
    let mut third = batch(&[b"epsilon"]);
    let third = session
        .sender()
        .try_send(&mut third, SendContext::default());
    assert_eq!(observation.lock().unwrap().bytes_since_flush, 3);
    let expected = wire(&[b"alpha", b"beta", b"gamma", b"delta", b"epsilon"]);
    let mut received = vec![0; expected.len()];
    timeout(Duration::from_secs(2), remote.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received, expected); // no repeated header, payload or reordered batch
    let completed = first.completion.unwrap().wait().await.unwrap();
    for (index, frame) in completed.frames.iter().enumerate() {
        assert_eq!(frame.frame.payload.as_ptr(), pointers[index]);
        assert!(matches!(frame.outcome, SendOutcome::LocalComplete));
    }
    for completion in [second.completion.unwrap(), third.completion.unwrap()] {
        assert!(
            completion
                .wait()
                .await
                .unwrap()
                .frames
                .iter()
                .all(|frame| matches!(frame.outcome, SendOutcome::LocalComplete))
        );
    }
    assert_eq!(
        observation.lock().unwrap().flush_sizes,
        [
            wire(&[b"alpha", b"beta"]).len(),
            wire(&[b"gamma", b"delta"]).len(),
            wire(&[b"epsilon"]).len()
        ]
    );
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn deadline_after_inline_pending_wakes_a_carrier_with_no_further_io_readiness() {
    let (stream, mut remote) = tokio::io::duplex(1);
    let observation = Arc::new(Mutex::new(IoObservation::default()));
    let observed = ObservedStream {
        stream,
        observation: observation.clone(),
    };
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
    let io = Arc::new(super::io::Shared::stream(
        InvalidatesDuringPoll(token.clone()),
        peer(),
        limits(),
    ));
    *token.lock().unwrap() = Some(io.stop.clone());
    let mut sender = super::Sender::start(io, limits());
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
async fn ready_datagram_batch_completes_before_background_scheduling() {
    struct ReadyDatagram(Arc<Mutex<Vec<Vec<u8>>>>);
    impl crate::raw::Datagram for ReadyDatagram {
        fn local_addr(&self) -> io::Result<SocketAddr> {
            Ok(peer())
        }
        fn poll_send_to(
            &self,
            _: &mut Context<'_>,
            payload: &[u8],
            _: SocketAddr,
        ) -> Poll<io::Result<usize>> {
            self.0.lock().unwrap().push(payload.to_vec());
            Poll::Ready(Ok(payload.len()))
        }
        fn poll_recv_from(
            &self,
            _: &mut Context<'_>,
            _: &mut [u8],
        ) -> Poll<io::Result<crate::raw::DatagramRead>> {
            Poll::Pending
        }
    }
    let sent = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::datagram(ReadyDatagram(sent.clone()), limits());
    let mut frames = VecDeque::from([
        Frame::new(b"one".to_vec(), Some(peer())),
        Frame::new(b"two".to_vec(), Some(peer())),
    ]);
    let submitted = session
        .sender()
        .try_send(&mut frames, SendContext::default());
    assert_eq!(*sent.lock().unwrap(), [b"one".to_vec(), b"two".to_vec()]);
    let mut completed = Box::pin(submitted.completion.unwrap().wait());
    let Poll::Ready(Ok(completed)) = futures_util::poll!(&mut completed) else {
        panic!("ready datagrams should not need an owner task hop");
    };
    assert!(
        completed
            .frames
            .iter()
            .all(|frame| matches!(frame.outcome, SendOutcome::LocalComplete))
    );
    assert_eq!(
        session.sender().available_budget().operations,
        limits().max_operations()
    );
    session.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn released_payload_budgets_do_not_make_a_notifying_worker_idle() {
    use std::{fmt, sync::Barrier, task::Waker};

    struct ErrorDropGate {
        noticed: Option<tokio::sync::oneshot::Sender<()>>,
        release: Arc<Barrier>,
    }
    impl fmt::Debug for ErrorDropGate {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("fixture error")
        }
    }
    impl fmt::Display for ErrorDropGate {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("fixture error")
        }
    }
    impl std::error::Error for ErrorDropGate {}
    impl Drop for ErrorDropGate {
        fn drop(&mut self) {
            let _ = self.noticed.take().unwrap().send(());
            self.release.wait();
        }
    }
    struct Control {
        ready: AtomicBool,
        failed: AtomicBool,
        waker: Mutex<Option<Waker>>,
        noticed: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: Arc<Barrier>,
        later_sends: AtomicUsize,
    }
    struct Endpoint(Arc<Control>);
    impl crate::raw::Datagram for Endpoint {
        fn local_addr(&self) -> io::Result<SocketAddr> {
            Ok(peer())
        }
        fn poll_send_to(
            &self,
            cx: &mut Context<'_>,
            payload: &[u8],
            _: SocketAddr,
        ) -> Poll<io::Result<usize>> {
            if !self.0.failed.load(Ordering::SeqCst) {
                *self.0.waker.lock().unwrap() = Some(cx.waker().clone());
                if !self.0.ready.load(Ordering::SeqCst) {
                    return Poll::Pending;
                }
                self.0.failed.store(true, Ordering::SeqCst);
                return Poll::Ready(Err(io::Error::other(ErrorDropGate {
                    noticed: self.0.noticed.lock().unwrap().take(),
                    release: self.0.release.clone(),
                })));
            }
            self.0.later_sends.fetch_add(1, Ordering::SeqCst);
            Poll::Ready(Ok(payload.len()))
        }
        fn poll_recv_from(
            &self,
            _: &mut Context<'_>,
            _: &mut [u8],
        ) -> Poll<io::Result<crate::raw::DatagramRead>> {
            Poll::Pending
        }
    }
    struct ReleaseOnDrop(Arc<Barrier>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            self.0.wait();
        }
    }

    let (noticed, notification) = tokio::sync::oneshot::channel();
    let release = Arc::new(Barrier::new(2));
    let control = Arc::new(Control {
        ready: AtomicBool::new(false),
        failed: AtomicBool::new(false),
        waker: Mutex::new(None),
        noticed: Mutex::new(Some(noticed)),
        release: release.clone(),
        later_sends: AtomicUsize::new(0),
    });
    let mut session = Session::datagram(Endpoint(control.clone()), limits());
    let mut frames = VecDeque::from([Frame::new(b"first".to_vec(), Some(peer()))]);
    drop(
        session
            .sender()
            .try_send(&mut frames, SendContext::default())
            .completion,
    );
    control.ready.store(true, Ordering::SeqCst);
    if let Some(waker) = control.waker.lock().unwrap().take() {
        waker.wake();
    }
    notification.await.unwrap();
    let release_on_drop = ReleaseOnDrop(release);
    // The worker is disposing the unobserved completion, AFTER reservation
    // release but BEFORE its writer lease release. Budget-only idleness would
    // incorrectly perform the next raw send on this caller thread.
    assert_eq!(
        session.sender().available_budget(),
        BudgetAvailable {
            frames: 4,
            bytes: 256,
            operations: 3
        }
    );
    let mut frames = VecDeque::from([Frame::new(b"second".to_vec(), Some(peer()))]);
    let second = session
        .sender()
        .try_send(&mut frames, SendContext::default());
    assert_eq!(second.accepted, 1);
    assert_eq!(control.later_sends.load(Ordering::SeqCst), 0);
    drop(release_on_drop);
    assert!(matches!(
        second.completion.unwrap().wait().await.unwrap().frames[0].outcome,
        SendOutcome::LocalComplete
    ));
    assert_eq!(control.later_sends.load(Ordering::SeqCst), 1);
    session.shutdown().await.unwrap();
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

#[tokio::test]
async fn always_ready_short_writes_obey_poll_quantum_and_resume_without_duplicate_bytes() {
    let observation = Arc::new(Mutex::new(ShortWriteObservation::default()));
    let stream = AlwaysShortWriter {
        observation: observation.clone(),
        cancel_on_write: None,
        vectored: false,
    };
    let configured = Limits::new(512, 1, 512, 1, 1).unwrap();
    let mut session = Session::stream(stream, peer(), configured);
    let payload = vec![37; 512];
    let expected = wire(&[&payload]);
    let mut frames = VecDeque::from([Frame::new(payload, None)]);
    let accepted = session
        .sender()
        .try_send(&mut frames, SendContext::default());
    let writes = observation.lock().unwrap().writes;
    assert!(writes > 0 && writes <= 64);
    let completed = accepted.completion.unwrap().wait().await.unwrap();
    assert!(matches!(
        completed.frames[0].outcome,
        SendOutcome::LocalComplete
    ));
    assert_eq!(observation.lock().unwrap().bytes, expected);
    assert_eq!(observation.lock().unwrap().flushes, 1);
    session.shutdown().await.unwrap();
}
