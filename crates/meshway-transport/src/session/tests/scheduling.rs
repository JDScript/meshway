use super::*;

#[tokio::test]
async fn fitting_stream_group_has_one_flush_and_preserves_all_original_buffers() {
    let (observed, mut remote, observation) = observed_stream(128);
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
        let (observed, mut remote, observation) = observed_stream(wire_size * count);
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
async fn inline_partial_write_handoff_preserves_fifo_and_original_buffers() {
    let (observed, mut remote, observation) = observed_stream(3);
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
