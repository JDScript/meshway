use super::*;

pub(super) async fn conflicts(
    backend: &mut backend::Backend,
    tun: &meshway_tun::Tun,
    tuple: OriginalTuple,
    clock: &Arc<Clock>,
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
            .install(100, tuple, deadline(clock, 30_000), scope.clone())
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
        .install(100, tuple, deadline(clock, 30_000), scope.clone())
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
        backend.confirm(100).await.unwrap(),
        CtConfirmation::ConfirmFailed { incarnation: 100 }
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
    backend.retire(100).await.unwrap();
    let revoked = OriginalTuple {
        transport: ExitTransport::Udp {
            source_port: 17101,
            destination_port: tuple.fields().2,
        },
        ..tuple
    };
    backend
        .install(101, revoked, deadline(clock, 30_000), scope.clone())
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
                101,
                deadline(clock, 30_000),
                ExitScope {
                    binding: scope.binding,
                    revoked: CancellationToken::new()
                }
            )
            .await
            .is_err()
    );
    println!(
        "X3_CONFLICTS=wrong-mark-install,unowned-established-drop,confirm-failed,foreign-ct-retained,revoked-renew-rejected"
    );
}
