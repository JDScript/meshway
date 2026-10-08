use super::*;

#[tokio::test]
async fn owned_receipt_future_outlives_manager_borrow_and_rejects_drop_before_first_poll() {
    let mut fixture = support::Fixture::builder().build().await;
    let ids = fixture.ids();
    let faults = Arc::new(Faults::default());
    let factory = Factory::new(&mut fixture, faults.clone());
    let mut b = fixture.manager(1, factory.clone()).start().await;
    let send = {
        let a = fixture.manager(0, factory.clone()).start().await;
        linked(&a, 1, TransportProtocol::NativeUdp).await;
        let mut secure = meshway_dataplane::secure::SecureNode::new(
            fixture.identities[0].clone(),
            Default::default(),
            a.carriers().unwrap(),
        )
        .unwrap();
        let pending = secure
            .start(fixture.demands[0], vec![ids[0], ids[1]])
            .unwrap();
        // Return both owners so dropping SecureNode cannot be the reason the
        // captured send fails. Only the Manager leaves this lexical scope.
        (a.send_with_receipt(pending), secure)
    };
    let (send, _secure) = send;
    let writes = faults.writes.load(Ordering::SeqCst);
    let mut sends = JoinSet::new();
    sends.spawn(send);
    assert!(matches!(
        sends.join_next().await.unwrap().unwrap(),
        Err(Error::Cancelled)
    ));
    assert_eq!(faults.writes.load(Ordering::SeqCst), writes);
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn owned_receipt_future_rejects_retained_channel_old_scope_before_first_poll() {
    let mut fixture = support::Fixture::builder().build().await;
    let ids = fixture.ids();
    let faults = Arc::new(Faults::default());
    let factory = Factory::new(&mut fixture, faults.clone());
    let mut b = fixture.manager(1, factory.clone()).start().await;
    let mut a = fixture.manager(0, factory.clone()).start().await;
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    let original = a.snapshot().unwrap().links[0].clone();
    let old_scope = a.shared.links.lock().unwrap()[&ids[1]].scope.clone();
    let mut secure = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[0].clone(),
        Default::default(),
        a.carriers().unwrap(),
    )
    .unwrap();
    let pending = secure
        .start(fixture.demands[0], vec![ids[0], ids[1]])
        .unwrap();
    let delayed = a.send_with_receipt(pending);
    fixture.controls[0].renew_instance().await.unwrap();
    let next = fixture.controls[0]
        .dataplane_identity(fixture.controls[0].execution_snapshot().unwrap())
        .unwrap();
    assert!(a.can_reconcile(&next));
    a.reconcile(next).await.unwrap();
    assert_eq!(a.snapshot().unwrap().links[0], original);
    assert!(old_scope.check().is_err());
    let writes = faults.writes.load(Ordering::SeqCst);
    let mut sends = JoinSet::new();
    sends.spawn(delayed);
    assert!(sends.join_next().await.unwrap().unwrap().is_err());
    assert_eq!(faults.writes.load(Ordering::SeqCst), writes);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn pipelined_owned_sends_keep_independent_circuit_cancellation_and_receipts() {
    let mut fixture = support::Fixture::builder().build().await;
    let ids = fixture.ids();
    let faults = Arc::new(Faults::default());
    let factory = Factory::new(&mut fixture, faults.clone());
    let mut b = fixture.manager(1, factory.clone()).start().await;
    let mut a = fixture.manager(0, factory.clone()).start().await;
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    linked(&b, 1, TransportProtocol::NativeUdp).await;
    let mut secure = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[0].clone(),
        Default::default(),
        a.carriers().unwrap(),
    )
    .unwrap();
    let first = secure
        .start(fixture.demands[0], vec![ids[0], ids[1]])
        .unwrap();
    let closed = first.circuit();
    let second = secure
        .start(fixture.demands[0], vec![ids[0], ids[1]])
        .unwrap();
    let sends = [a.send_with_receipt(first), a.send_with_receipt(second)];
    let before = faults.writes.load(Ordering::SeqCst);
    secure.close(closed);
    let outcomes = tokio::time::timeout(
        Duration::from_secs(2),
        futures_util::future::join_all(sends),
    )
    .await
    .unwrap();
    assert!(outcomes[0].is_err());
    assert!(outcomes[1].is_ok());
    assert_eq!(faults.writes.load(Ordering::SeqCst) - before, 1);
    let (_, received) = frame(&mut b).await;
    assert!(!received.payload.is_empty());
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}
