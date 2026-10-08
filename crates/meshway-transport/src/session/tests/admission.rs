use super::*;

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
async fn saturated_business_batch_preserves_control_admission_budget() {
    let (stream, _peer) = tokio::io::duplex(1);
    let mut session = Session::stream(
        stream,
        "127.0.0.1:1234".parse().unwrap(),
        Limits::new(1024, 4, 4096, 4, 4).unwrap(),
    );
    session
        .sender()
        .reserve_control(crate::BudgetAvailable {
            frames: 1,
            bytes: 1024,
            operations: 1,
        })
        .unwrap();
    let scope = crate::ChannelScope::new();
    let mut data: VecDeque<_> = batch(&[b"first", b"second", b"third", b"suffix"])
        .into_iter()
        .map(|frame| crate::ScopedFrame {
            frame,
            scope: scope.clone(),
            context: SendContext::default(),
        })
        .collect();
    let admitted = session
        .sender()
        .submit_batch(&mut data, SendContext::default());
    assert_eq!(admitted.accepted, 3);
    assert_eq!(data.len(), 1);
    assert_eq!(
        session
            .sender()
            .submit_batch(&mut data, SendContext::default())
            .accepted,
        0
    );
    let control = session
        .sender()
        .try_send(&mut batch(&[b"health"]), SendContext::default());
    assert_eq!(control.accepted, 1);
    session.sender().shutdown().await.unwrap();
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
