use super::*;

fn scope() -> Scope {
    Scope {
        deadline: None,
        cancellation: CancellationToken::new(),
        edge: None,
        health: None,
    }
}

#[tokio::test]
async fn waiting_scope_has_no_lease_and_ends_only_by_its_owner_or_budget() {
    let scope = scope();
    let work = scope.run(std::future::pending::<Result<(), Error>>());
    tokio::pin!(work);
    assert!(futures_util::poll!(work.as_mut()).is_pending());
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(futures_util::poll!(work.as_mut()).is_pending());
    scope.check().unwrap();
    // The Manager's apply retires a channel by cancelling its scope.
    scope.cancellation.cancel();
    assert!(work.await.is_err());
    assert!(scope.check().is_err());
    let mut health = self::scope();
    health.health = Some(Arc::new(Mutex::new(
        tokio::time::Instant::now() + Duration::from_millis(20),
    )));
    assert!(
        tokio::time::timeout(
            Duration::from_secs(2),
            health.run(std::future::pending::<Result<(), Error>>())
        )
        .await
        .unwrap()
        .is_err()
    );
}

#[tokio::test]
async fn scope_ready_work_keeps_final_checks_and_pending_work_is_polled_once_per_turn() {
    let scope = scope();
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
    scope.deadline = Some(tokio::time::Instant::now() + Duration::from_millis(10));
    assert!(
        tokio::time::timeout(
            Duration::from_secs(2),
            scope.run(std::future::pending::<Result<(), Error>>())
        )
        .await
        .unwrap()
        .is_err()
    );
}
