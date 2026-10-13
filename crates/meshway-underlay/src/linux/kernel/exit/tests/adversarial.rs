use super::*;

pub(super) async fn conflicts(
    backend: &mut backend::Backend,
    tun: &meshway_tun::Tun,
    tuple: OriginalTuple,
) {
    let tuple = OriginalTuple {
        transport: ExitTransport::Udp {
            source_port: 17100,
            destination_port: tuple.fields().2,
        },
        ..tuple
    };
    // A foreign (mark 0), seen-reply entry under the projection's tuple.
    conntrack::insert_foreign(tuple, 60).await.unwrap();
    let scope = ExitScope {
        binding: Digest::from_bytes([10; 32]),
        revoked: CancellationToken::new(),
    };
    assert!(
        backend
            .install(id(100), tuple, cutoff(30_000), scope.clone())
            .await
            .is_err()
    );
    assert_eq!(conntrack::lookup(tuple).await.unwrap().unwrap().mark, 0);
    conntrack::delete_foreign(tuple).await.unwrap();
    backend
        .install(id(100), tuple, cutoff(30_000), scope.clone())
        .await
        .unwrap();
    conntrack::insert_foreign(tuple, 60).await.unwrap();
    tun.send(&udp(tuple, b"unowned-established")).await.unwrap();
    let mut packet = vec![0; 1280];
    assert!(
        tokio::time::timeout(Duration::from_millis(150), recv_ipv4(tun, &mut packet))
            .await
            .is_err()
    );
    assert_eq!(
        backend.confirm(id(100)).await.unwrap(),
        CtConfirmation::ConfirmFailed {
            incarnation: id(100)
        }
    );
    assert_eq!(conntrack::lookup(tuple).await.unwrap().unwrap().mark, 0);
    conntrack::delete_foreign(tuple).await.unwrap();
    backend.retire(id(100)).await.unwrap();
    let revoked = OriginalTuple {
        transport: ExitTransport::Udp {
            source_port: 17101,
            destination_port: tuple.fields().2,
        },
        ..tuple
    };
    backend
        .install(id(101), revoked, cutoff(30_000), scope.clone())
        .await
        .unwrap();
    scope.revoked.cancel();
    backend.maintain().await.unwrap();
    tun.send(&udp(revoked, b"revoked")).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), recv_ipv4(tun, &mut packet))
            .await
            .is_err()
    );
    assert!(
        backend
            .renew(
                id(101),
                cutoff(30_000),
                ExitScope {
                    binding: scope.binding,
                    revoked: CancellationToken::new()
                }
            )
            .await
            .is_err()
    );
    let racing = OriginalTuple {
        transport: ExitTransport::Udp {
            source_port: 17110,
            destination_port: tuple.fields().2,
        },
        ..tuple
    };
    let original = ExitScope {
        binding: Digest::from_bytes([12; 32]),
        revoked: CancellationToken::new(),
    };
    backend
        .install(id(110), racing, cutoff(30_000), original.clone())
        .await
        .unwrap();
    tun.send(&udp(racing, b"renew-before-revocation"))
        .await
        .unwrap();
    assert!(matches!(
        backend.confirm(id(110)).await.unwrap(),
        CtConfirmation::Confirmed { .. }
    ));
    tokio::time::timeout(Duration::from_secs(2), recv_ipv4(tun, &mut packet))
        .await
        .unwrap()
        .unwrap();
    let mut renewal = Box::pin(backend.renew(
        id(110),
        cutoff(30_000),
        ExitScope {
            binding: original.binding,
            revoked: CancellationToken::new(),
        },
    ));
    assert!(futures_util::poll!(renewal.as_mut()).is_pending());
    original.revoked.cancel();
    assert!(renewal.await.is_err());
    tun.send(&udp(racing, b"revoked-during-renew"))
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), recv_ipv4(tun, &mut packet))
            .await
            .is_err()
    );
    backend.retire(id(110)).await.unwrap();
    println!(
        "X3_CONFLICTS=wrong-mark-install,unowned-established-drop,confirm-failed,foreign-ct-kept,revoked-renew-rejected,revoked-during-renew-rejected"
    );
}
