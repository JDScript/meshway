use super::*;

#[tokio::test]
async fn new_epoch_dialer_waits_for_selected_owner_handoff_before_acquiring() {
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };

    let mut fixture = support::Fixture::builder().build().await;
    let ids = fixture.ids();
    let factory = Factory::new(&mut fixture, Arc::new(Faults::default()));
    let mut a = fixture.manager(0, factory.clone()).start().await;
    let mut b = fixture.manager(1, factory.clone()).start().await;
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    linked(&b, 1, TransportProtocol::NativeUdp).await;
    let original = a.snapshot().unwrap().links;
    let remote = b.snapshot().unwrap().links;
    let old_scope = a.shared.links.lock().unwrap()[&ids[1]].scope.clone();
    fixture.create_demand(0, 120).await;
    let next = fixture.refresh(0).await;
    assert!(a.can_reconcile(&next));
    assert!(
        old_scope.check().is_err(),
        "old output authority is already revoked"
    );

    // Poll acquisition FIRST after publication, before yielding to the real
    // selected endpoint owner. This fixes the interleaving, not random Node IDs
    // or the channel choice: a revoked old scope still occupies the owner slot.
    let mut updates = a.shared.epochs.subscribe();
    a.shared.epochs.send_replace(support::epoch(&next, 2));
    let shared = a.shared.latest(&mut updates).await.unwrap();
    assert!(
        a.shared.links.lock().unwrap()[&ids[1]]
            .scope
            .check()
            .is_err()
    );
    assert!(!workers::candidates(&shared, ids[1]).unwrap().is_empty());
    factory.blocked_dialers.lock().unwrap().insert(ids[0]);
    let before = factory.acquisitions.load(Ordering::SeqCst);
    let mut acquisition = Box::pin(workers::dial(shared, factory.clone(), ids[1]));
    poll_fn(|cx| {
        assert!(acquisition.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(
        factory.acquisitions.load(Ordering::SeqCst),
        before,
        "handoff is not a vacant slot: no speculative socket acquisition"
    );
    drop(acquisition);
    factory.blocked_dialers.lock().unwrap().remove(&ids[0]);

    a.reconcile(next).await.unwrap();
    assert_eq!(a.snapshot().unwrap().links, original);
    assert_eq!(b.snapshot().unwrap().links, remote);
    assert!(old_scope.check().is_err());
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn expired_business_candidate_gains_a_worker_scope_on_direct_publication() {
    // C: the requested 2 s business permit must really expire before starting
    // the old worker; sleep is derived from that issued permit's deadline.
    let mut fixture = support::Fixture::builder().build().await;
    let ids = fixture.ids();
    let instance = fixture.controls[0].execution_snapshot().unwrap().instance();
    fixture.controls[0]
        .cancel_demand(&meshway_protocol::CancelDemandRequest {
            instance,
            request_id: fixture.demands[0],
        })
        .await
        .unwrap();
    let short = fixture.create_demand(0, 2).await;
    let old = fixture.refresh(0).await;
    fixture.refresh(1).await;
    let expires = old.snapshot().configuration().unwrap().permits[&short].expires_at_ms;
    let delay = expires.saturating_sub(old.snapshot().now_ms().unwrap());
    tokio::time::sleep(Duration::from_millis(delay + 10)).await;
    old.snapshot().ensure_current().unwrap();
    let carrier = support::carriers(&old)[&ids[1]].clone();
    assert!(old.snapshot().admit_adjacency(ids[1], &carrier).is_err());
    let faults = Arc::new(Faults::default());
    let factory = Factory::new(&mut fixture, faults.clone());
    let mut a = Manager::start(old.clone(), factory.clone(), Default::default())
        .await
        .unwrap();
    let mut b = fixture.manager(1, factory.clone()).start().await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        faults.controls.lock().unwrap().is_empty(),
        "dormant worker cannot send without a business grant"
    );
    assert!(a.snapshot().unwrap().links.is_empty());

    fixture.create_demand(0, 120).await;
    for (index, manager) in [(0, &mut a), (1, &mut b)] {
        let next = fixture.refresh(index).await;
        assert!(
            manager.can_reconcile(&next),
            "same resources, direct publication"
        );
        manager.reconcile(next).await.unwrap();
    }
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    linked(&b, 1, TransportProtocol::NativeUdp).await;
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn retained_listener_rechecks_peer_capacity_before_accepting_a_publication() {
    let mut fixture = support::Fixture::builder().build().await;
    let instance = fixture.controls[2].execution_snapshot().unwrap().instance();
    fixture.controls[2]
        .cancel_demand(&meshway_protocol::CancelDemandRequest {
            instance,
            request_id: fixture.demands[1],
        })
        .await
        .unwrap();
    let identity = fixture.refresh(1).await;
    assert_eq!(identity.snapshot().configuration().unwrap().peers.len(), 1);
    let factory = Factory::new(&mut fixture, Arc::new(Faults::default()));
    let mut manager = Manager::start(
        identity,
        factory,
        Limits {
            candidates: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    fixture.create_demand(2, 120).await;
    let next = fixture.refresh(1).await;
    let old = next
        .snapshot()
        .continuation_from(manager.shared.identity.snapshot())
        .unwrap();
    assert_eq!(
        old.listeners,
        next.snapshot().configuration().unwrap().listeners
    );
    assert_eq!(
        old.dialers,
        next.snapshot().configuration().unwrap().dialers
    );
    assert_eq!(next.snapshot().configuration().unwrap().peers.len(), 2);
    assert!(!manager.can_reconcile(&next));
    assert!(matches!(
        manager.reconcile(next).await,
        Err(Error::Capacity)
    ));
    assert!(manager.snapshot().is_err());
    manager.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn same_target_renewal_rejects_a_still_live_previous_ready_scope() {
    let mut fixture = support::Fixture::builder().build().await;
    let ids = fixture.ids();
    let factory = Factory::new(&mut fixture, Arc::new(Faults::default()));
    let mut a = fixture.manager(0, factory.clone()).start().await;
    let mut b = fixture.manager(1, factory.clone()).start().await;
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    linked(&b, 1, TransportProtocol::NativeUdp).await;
    let previous = b.shared.clone();
    let (fact, sender) = {
        let links = previous.links.lock().unwrap();
        let link = &links[&ids[0]];
        (link.fact.clone(), link.sender.clone())
    };
    // A completed authentication can reach installation after the epoch swap;
    // its immutable original scope must not borrow the new owner's authority.
    let late_scope = previous.scope(Some((fact.peer, fact.carrier.clone())), false);
    tokio::time::sleep(Duration::from_millis(20)).await;
    fixture.controls[1].renew_instance().await.unwrap();
    let next = fixture.controls[1]
        .dataplane_identity(fixture.controls[1].execution_snapshot().unwrap())
        .unwrap();
    assert!(next.snapshot().same_execution(previous.identity.snapshot()));
    assert_ne!(
        next.snapshot().expires_at(),
        previous.identity.snapshot().expires_at()
    );
    b.reconcile(next).await.unwrap();
    late_scope.check().unwrap();
    assert!(matches!(
        install_ready(
            &previous,
            ReadyPeer {
                fact: fact.clone(),
                sender,
                scope: late_scope.clone(),
                supersedes: Replacement::Channel(fact.id),
                attempt: None,
            }
        ),
        Err(Error::Authority)
    ));
    assert!(late_scope.cancellation.is_cancelled());
    assert_eq!(b.snapshot().unwrap().links, vec![fact]);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn same_target_renewal_wakes_dormant_dialer_without_waiting_old_backoff() {
    let mut fixture = support::Fixture::builder().build().await;
    let ids = fixture.ids();
    let factory = Factory::new(&mut fixture, Arc::new(Faults::default()));
    factory.blocked_dialers.lock().unwrap().insert(ids[0]);
    let original = fixture.identities[0].clone();
    let mut a = Manager::start(
        original.clone(),
        factory.clone(),
        Limits {
            retry_backoff: Duration::from_secs(60),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut b = fixture.manager(1, factory.clone()).start().await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(a.snapshot().unwrap().links.is_empty());
    fixture.controls[0].renew_instance().await.unwrap();
    let next = fixture.controls[0]
        .dataplane_identity(fixture.controls[0].execution_snapshot().unwrap())
        .unwrap();
    original.snapshot().ensure_current().unwrap();
    assert!(next.snapshot().same_execution(original.snapshot()));
    factory.blocked_dialers.lock().unwrap().remove(&ids[0]);
    a.reconcile(next).await.unwrap();
    // linked has a twelve-second bound, well below the still-live old backoff.
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    linked(&b, 1, TransportProtocol::NativeUdp).await;
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}
