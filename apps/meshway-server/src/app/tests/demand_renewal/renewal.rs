use super::*;

#[tokio::test]
async fn absolute_renewal_preserves_complete_targets_and_replays_without_writes() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &signer_a).await;
    let bt = session(&service, network, b, &bk, &signer_b).await;
    let ai = acquire(&app, &at).await;
    let bi = acquire(&app, &bt).await;
    let one = demand(&ai, b);
    let mut two = demand(&bi, a);
    two.duration_seconds = 60;
    service
        .demand(at.token.expose(), one.clone())
        .await
        .unwrap();
    service
        .demand(bt.token.expose(), two.clone())
        .await
        .unwrap();
    let ai = service
        .renew(at.token.expose(), instance(&ai))
        .await
        .unwrap();
    let before = configuration(&app, &at, &ai).await;
    let before_slice = &before.full.unwrap().configuration;
    let old_permit = before_slice.permits[&one.request_id].clone();
    let other_permit = before_slice.permits[&two.request_id].clone();
    let renew = renewal(&one, ai.expires_at_ms);
    let renewed = service
        .renew_demand(at.token.expose(), renew.clone())
        .await
        .unwrap();
    let DemandDecision::Permitted { permit } = &renewed.decision else {
        panic!("permitted renewal")
    };
    assert_eq!(permit.expires_at_ms, ai.expires_at_ms);
    assert!(permit.expires_at_ms > old_permit.expires_at_ms);
    assert_eq!(permit.paths, old_permit.paths);
    assert_ne!(
        renewed.publication,
        before.lease.target.unwrap().execution_publication
    );
    for (token, lease) in [(&at, &ai), (&bt, &bi)] {
        let current = configuration(&app, token, lease).await;
        let slice = current.full.unwrap().configuration;
        assert_eq!(slice.permits.len(), 2);
        assert_eq!(slice.permits[&one.request_id], *permit);
        assert_eq!(slice.permits[&two.request_id], other_permit);
        assert_eq!(current.lease.expires_at_ms, lease.expires_at_ms);
    }
    let before_retry = snapshot(&app, network).await;
    let event_sequence = app.state_runtime.lock().unwrap().hub.events.sequence();
    for expiry in [renew.expires_at_ms, old_permit.expires_at_ms, 0] {
        assert_eq!(
            service
                .renew_demand(at.token.expose(), renewal(&one, expiry))
                .await
                .unwrap(),
            renewed
        );
    }
    // The original create hash remains usable, including after a newer renewal.
    assert_eq!(
        service
            .demand(at.token.expose(), one.clone())
            .await
            .unwrap(),
        renewed
    );
    assert_eq!(snapshot(&app, network).await, before_retry);
    assert_eq!(
        app.state_runtime.lock().unwrap().hub.events.sequence(),
        event_sequence
    );
    let mut conflicting = one;
    conflicting.duration_seconds += 1;
    assert_eq!(
        service.demand(at.token.expose(), conflicting).await,
        Err(Error::BindingConflict)
    );
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn renewal_is_instance_bounded_and_cannot_resurrect_missing_cancelled_or_expired_intents() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &signer_a).await;
    let bt = session(&service, network, b, &bk, &signer_b).await;
    let ai = acquire(&app, &at).await;
    let bi = acquire(&app, &bt).await;
    let create = demand(&ai, b);
    service
        .demand(at.token.expose(), create.clone())
        .await
        .unwrap();
    let before = snapshot(&app, network).await;
    for expires in [ai.expires_at_ms + 1, u64::MAX] {
        assert_eq!(
            service
                .renew_demand(at.token.expose(), renewal(&create, expires))
                .await,
            Err(Error::InvalidRequest)
        );
    }
    let mut wrong = renewal(&create, ai.expires_at_ms);
    wrong.instance.generation = Version::new(ai.generation.get() + 1).unwrap();
    assert_eq!(
        service.renew_demand(at.token.expose(), wrong).await,
        Err(Error::BindingConflict)
    );
    let mut foreign = renewal(&create, bi.expires_at_ms);
    foreign.instance = instance(&bi);
    assert_eq!(
        service.renew_demand(bt.token.expose(), foreign).await,
        Err(Error::BindingConflict)
    );
    let missing = demand(&ai, b);
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&missing, ai.expires_at_ms))
            .await,
        Err(Error::BindingConflict)
    );
    assert_eq!(snapshot(&app, network).await, before);
    service
        .cancel_demand(
            at.token.expose(),
            CancelDemandRequest {
                instance: instance(&ai),
                request_id: create.request_id,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&create, ai.expires_at_ms))
            .await,
        Err(Error::BindingConflict)
    );
    let expired = demand(&ai, b);
    service
        .demand(at.token.expose(), expired.clone())
        .await
        .unwrap();
    let request_id = expired.request_id;
    app.executor
        .handle()
        .execute(move |db| {
            db.execute(
                "UPDATE execution_demands SET expires_at=0 WHERE id=?1",
                [request_id.as_uuid().as_bytes().as_slice()],
            )
            .unwrap();
        })
        .await
        .unwrap();
    // The storage owner serializes cleanup and renewal in either accepted order.
    let (renewed, cleaned) = tokio::join!(
        service.renew_demand(at.token.expose(), renewal(&expired, ai.expires_at_ms)),
        service.renew(bt.token.expose(), instance(&bi)),
    );
    assert_eq!(renewed, Err(Error::BindingConflict));
    cleaned.unwrap();
    for (token, lease) in [(&at, &ai), (&bt, &bi)] {
        assert!(
            configuration(&app, token, lease)
                .await
                .full
                .unwrap()
                .configuration
                .permits
                .is_empty()
        );
    }
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&expired, 0))
            .await,
        Err(Error::BindingConflict)
    );
    let records = snapshot(&app, network).await;
    assert!(
        records
            .demands
            .iter()
            .all(|record| record.cancelled.is_some())
    );
    let fresh = demand(&ai, b);
    service
        .demand(at.token.expose(), fresh.clone())
        .await
        .unwrap();
    let next = acquire(&app, &at).await;
    assert_eq!(
        service
            .renew_demand(at.token.expose(), renewal(&fresh, next.expires_at_ms))
            .await,
        Err(Error::BindingConflict)
    );
    let mut replaced = renewal(&fresh, next.expires_at_ms);
    replaced.instance = instance(&next);
    assert_eq!(
        service.renew_demand(at.token.expose(), replaced).await,
        Err(Error::BindingConflict)
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn renewal_and_replay_remove_expired_siblings_from_every_participant() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &signer_a).await;
    let bt = session(&service, network, b, &bk, &signer_b).await;
    let ai = acquire(&app, &at).await;
    let bi = acquire(&app, &bt).await;
    let live = demand(&ai, b);
    service
        .demand(at.token.expose(), live.clone())
        .await
        .unwrap();
    for _ in 0..2 {
        let sibling = demand(&bi, a);
        service
            .demand(bt.token.expose(), sibling.clone())
            .await
            .unwrap();
        let request_id = sibling.request_id;
        app.executor
            .handle()
            .execute(move |db| {
                db.execute(
                    "UPDATE execution_demands SET expires_at=0 WHERE id=?1",
                    [request_id.as_uuid().as_bytes().as_slice()],
                )
                .unwrap();
            })
            .await
            .unwrap();
        // First iteration extends the deadline; the second repeats it exactly.
        let response = service
            .renew_demand(at.token.expose(), renewal(&live, ai.expires_at_ms))
            .await
            .unwrap();
        assert!(matches!(
            response.decision,
            DemandDecision::Permitted { .. }
        ));
        for (token, lease) in [(&at, &ai), (&bt, &bi)] {
            let target = configuration(&app, token, lease).await.full.unwrap();
            assert_eq!(target.configuration.permits.len(), 1);
            assert!(target.configuration.permits.contains_key(&live.request_id));
            assert!(
                !target
                    .configuration
                    .decisions
                    .contains_key(&sibling.request_id)
            );
        }
        assert_eq!(
            service
                .renew_demand(bt.token.expose(), renewal(&sibling, bi.expires_at_ms))
                .await,
            Err(Error::BindingConflict)
        );
    }
    app.shutdown().await.unwrap();
}
