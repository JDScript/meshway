use super::*;

#[tokio::test]
async fn scope_ready_work_keeps_final_fences_and_pending_work_is_polled_once_per_turn() {
    let fixture = support::Fixture::builder().build().await;
    let snapshot = fixture.identities[0].snapshot().clone();
    let scope = Scope {
        deadline: snapshot.expires_at().into(),
        plan: crate::plan::Plan::new(&snapshot).unwrap(),
        snapshot,
        cancellation: CancellationToken::new(),
        edge: None,
        health: None,
    };
    let polls = AtomicUsize::new(0);
    let mut work = Box::pin(scope.run(std::future::poll_fn(|cx| {
        if polls.fetch_add(1, Ordering::SeqCst) == 0 {
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    })));
    assert!(futures_util::poll!(work.as_mut()).is_pending());
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    scope.cancellation.cancel();
    assert!(matches!(
        futures_util::poll!(work.as_mut()),
        Poll::Ready(Err(_))
    ));
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    drop(work);

    let mut scope = Scope {
        cancellation: CancellationToken::new(),
        ..scope
    };
    assert!(
        scope
            .run(async {
                scope.cancellation.cancel();
                Ok(())
            })
            .await
            .is_err()
    );
    scope.cancellation = CancellationToken::new();
    scope.deadline = tokio::time::Instant::now() + Duration::from_millis(10);
    assert!(
        tokio::time::timeout(
            Duration::from_secs(2),
            scope.run(std::future::pending::<Result<(), Error>>())
        )
        .await
        .unwrap()
        .is_err()
    );
    fixture.close().await;
}
