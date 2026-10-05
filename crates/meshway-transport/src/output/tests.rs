use super::*;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    task::Poll,
};

#[tokio::test]
async fn shutdown_releases_active_scope_while_the_session_is_still_owned() {
    #[derive(Debug)]
    struct Lifetime(CancellationToken);
    impl crate::SendLifetime for Lifetime {
        fn is_current(&self) -> bool {
            !self.0.is_cancelled()
        }
        fn invalidated(self: Arc<Self>) -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
            Box::pin(async move { self.0.cancelled().await })
        }
    }
    let lifetime = Arc::new(Lifetime(CancellationToken::new()));
    let released = Arc::downgrade(&lifetime);
    let context = SendContext {
        lifetime: Some(lifetime),
        ..SendContext::default()
    };
    let cancel = context.cancellation.clone();
    let (local, _remote) = tokio::io::duplex(1);
    let mut session = crate::Session::stream(
        local,
        "127.0.0.1:443".parse().unwrap(),
        crate::Limits::new(16, 1, 16, 1, 1).unwrap(),
    );
    let mut frames = std::collections::VecDeque::from([crate::Frame::new(vec![7; 16], None)]);
    let admitted = session.sender().try_send(&mut frames, context);
    assert!(released.upgrade().is_some());
    cancel.cancel();
    // Inline Pending left ownership in Session, not in this thread's next poll.
    assert!(check_current().is_ok());
    session.shutdown().await.unwrap();
    admitted.completion.unwrap().wait().await.unwrap();
    assert!(released.upgrade().is_none());
}

#[tokio::test]
async fn synchronous_scope_is_restored_after_nested_polls_and_unwind() {
    let outer = OutputScope::new(CancellationToken::new());
    let invalid = SendContext::default();
    invalid.cancellation.cancel();
    outer.activate(&invalid);
    let inner = OutputScope::new(CancellationToken::new());
    assert!(check_current().is_ok());
    {
        let _outer = outer.enter();
        assert!(check_current().is_err());
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                let _inner = inner.enter();
                assert!(check_current().is_ok());
                panic!("poll unwind");
            }))
            .is_err()
        );
        assert!(check_current().is_err());
    }
    assert!(check_current().is_ok());
    // A later poll on another runtime thread installs its own restriction;
    // nothing depends on the thread on which the scope was created.
    std::thread::spawn(move || {
        assert!(check_current().is_ok());
        let _scope = outer.enter();
        assert!(check_current().is_err());
    })
    .join()
    .unwrap();
    assert!(check_current().is_ok());
}

#[derive(Default)]
struct OutputCalls {
    writes: usize,
    flushes: usize,
    shutdowns: usize,
}

struct Primitive {
    calls: Arc<Mutex<OutputCalls>>,
    cancel: CancellationToken,
}

impl AsyncWrite for Primitive {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.calls.lock().unwrap().writes += 1;
        self.cancel.cancel();
        Poll::Ready(Ok(bytes.len().min(1)))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.calls.lock().unwrap().flushes += 1;
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.calls.lock().unwrap().shutdowns += 1;
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn primitive_output_rechecks_within_one_provider_poll_and_idle_controls_remain_usable() {
    let context = SendContext::default();
    let scope = OutputScope::new(CancellationToken::new());
    scope.activate(&context);
    let calls = Arc::new(Mutex::new(OutputCalls::default()));
    let mut output = CheckedOutput::new(Primitive {
        calls: calls.clone(),
        cancel: context.cancellation.clone(),
    });
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    {
        let _poll = scope.enter();
        assert!(matches!(
            Pin::new(&mut output).poll_write(&mut cx, b"ab"),
            Poll::Ready(Ok(1))
        ));
        assert!(matches!(
            Pin::new(&mut output).poll_write(&mut cx, b"b"),
            Poll::Ready(Err(_))
        ));
        assert!(matches!(
            Pin::new(&mut output).poll_write_vectored(&mut cx, &[io::IoSlice::new(b"b")]),
            Poll::Ready(Err(_))
        ));
        assert!(matches!(
            Pin::new(&mut output).poll_flush(&mut cx),
            Poll::Ready(Err(_))
        ));
        assert!(matches!(
            Pin::new(&mut output).poll_shutdown(&mut cx),
            Poll::Ready(Err(_))
        ));
    }
    assert_eq!(calls.lock().unwrap().writes, 1);
    assert_eq!(calls.lock().unwrap().flushes, 0);
    assert_eq!(calls.lock().unwrap().shutdowns, 0);

    let idle = OutputScope::new(CancellationToken::new());
    let _poll = idle.enter();
    assert!(matches!(
        Pin::new(&mut output).poll_flush(&mut cx),
        Poll::Ready(Ok(()))
    ));
    idle.activate(&SendContext {
        deadline: Some(tokio::time::Instant::now()),
        ..SendContext::default()
    });
    assert!(matches!(
        Pin::new(&mut output).poll_write(&mut cx, b"expired"),
        Poll::Ready(Err(_))
    ));
    assert_eq!(calls.lock().unwrap().writes, 1);
}
