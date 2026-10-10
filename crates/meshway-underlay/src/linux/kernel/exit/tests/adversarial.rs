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
    let mut insert = vec!["-I".into()];
    // conntrack-tools infers the family from addresses for INSERT; unlike
    // LIST/DELETE, this pinned command rejects the explicit --family option.
    insert.extend(tuple.filter().into_iter().skip(2));
    insert.extend([
        "--timeout".into(),
        "60".into(),
        "--mark".into(),
        "0".into(),
        "--status".into(),
        "SEEN_REPLY".into(),
    ]);
    let created = backend
        .commands
        .run(command::Tool::Conntrack, insert.clone(), Vec::new())
        .await
        .unwrap();
    assert!(
        created.success,
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
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
    assert_eq!(
        conntrack::lookup(&mut backend.commands, tuple)
            .await
            .unwrap()
            .unwrap()
            .mark,
        0
    );
    let mut delete = vec!["-D".into()];
    delete.extend(tuple.filter());
    delete.extend(["--mark".into(), "0/4294967295".into()]);
    assert!(
        backend
            .commands
            .run(command::Tool::Conntrack, delete.clone(), Vec::new())
            .await
            .unwrap()
            .success
    );
    backend
        .install(id(100), tuple, cutoff(30_000), scope.clone())
        .await
        .unwrap();
    assert!(
        backend
            .commands
            .run(command::Tool::Conntrack, insert, Vec::new())
            .await
            .unwrap()
            .success
    );
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
    assert_eq!(
        conntrack::lookup(&mut backend.commands, tuple)
            .await
            .unwrap()
            .unwrap()
            .mark,
        0
    );
    assert!(
        backend
            .commands
            .run(command::Tool::Conntrack, delete, Vec::new())
            .await
            .unwrap()
            .success
    );
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
        "X3_CONFLICTS=wrong-mark-install,unowned-established-drop,confirm-failed,foreign-ct-retained,revoked-renew-rejected,revoked-during-renew-rejected"
    );
}
