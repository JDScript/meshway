use super::*;
use meshway_authority::ExecutionSnapshot;
use meshway_control::Control;

#[derive(Clone, Copy)]
pub(super) enum Case {
    Continue,
    Expire,
    Reinstall,
    UnknownCleanup,
}

#[tokio::test]
#[ignore = "requires isolated real Server, Control and NAT namespaces"]
async fn control_handoff_retains_ct_across_three_ticks_without_old_io() {
    super::relay::run(Some(Case::Continue)).await;
}

#[tokio::test]
#[ignore = "requires isolated real Server, Control and NAT namespaces"]
async fn control_handoff_without_successor_cleans_at_original_retention_deadline() {
    super::relay::run(Some(Case::Expire)).await;
}

#[tokio::test]
#[ignore = "requires isolated real Server, Control and NAT namespaces"]
async fn stopped_role_probe_reconciles_verified_cleanup_before_reinstall() {
    super::relay::run(Some(Case::Reinstall)).await;
}

#[tokio::test]
#[ignore = "requires isolated real Server, Control and NAT namespaces"]
async fn stopped_role_unknown_cleanup_never_reports_ready_or_loses_owner() {
    super::relay::run(Some(Case::UnknownCleanup)).await;
}

struct Guard(ExecutionSnapshot);
impl ExitSendGuard for Guard {
    fn check(&self) -> Result<(), Invalidated> {
        self.0.ensure_current().map_err(|_| Invalidated)
    }
}

pub(super) async fn exercise(
    case: Case,
    host: &mut crate::Host,
    controls: &mut [Control],
    role: &ExitRole,
    previous: &ExecutionSnapshot,
) {
    if matches!(case, Case::UnknownCleanup) {
        let (_, ledger) = storage::Store::open(host.kernel.store.directory())
            .await
            .unwrap();
        let ledger = ledger.unwrap();
        let mut commands = command::Commands::default();
        objects::mutate(
            &mut commands,
            &serde_json::json!({"nftables": [{"add": {"chain": {
                "family": "inet", "table": ledger.program.filter(), "name": "foreign_sentinel"
            }}}]}),
        )
        .await
        .unwrap();
        assert!(role.retire().await.is_err());
        for _ in 0..2 {
            let observation = host
                .probe_exit(previous.instance().instance_id, previous.reference())
                .await;
            assert_eq!(
                observation.status,
                meshway_model::exit::ExitStatus::Unavailable {
                    reason: meshway_model::exit::ExitUnavailableReason::OwnershipConflict,
                }
            );
            assert!(
                host.kernel.exit.is_some(),
                "failed cleanup must keep its owner"
            );
            assert!(host.install_exit_role(role.plan.clone()).await.is_err());
        }
        assert_eq!(
            objects::present(&mut commands, &ledger.program)
                .await
                .unwrap(),
            2
        );
        return;
    }
    if matches!(case, Case::Reinstall) {
        role.retire().await.unwrap();
        let capability = host
            .probe_exit(previous.instance().instance_id, previous.reference())
            .await;
        assert_eq!(capability.status, meshway_model::exit::ExitStatus::Ready);
        let next = host.install_exit_role(role.plan.clone()).await.unwrap();
        assert!(next.is_active());
        next.retire().await.unwrap();
        return;
    }
    let permit = previous
        .configuration()
        .unwrap()
        .permits
        .values()
        .find(|permit| permit.exit.is_some())
        .unwrap();
    let tuple = OriginalTuple {
        source: permit.source_addresses.ipv4.unwrap(),
        destination: "198.18.71.11".parse().unwrap(),
        transport: ExitTransport::Udp {
            source_port: 17524,
            destination_port: 42424,
        },
    };
    let clock = Arc::new(Clock::new().unwrap());
    let scope = ExitScope {
        binding: Digest::from_bytes([72; 32]),
        revoked: CancellationToken::new(),
    };
    let incarnation = id(221);
    role.install(incarnation, tuple, deadline(&clock, 20_000), scope.clone())
        .await
        .unwrap();
    let mut reader = role.reply_reader().unwrap();
    role.send(
        incarnation,
        udp(tuple, b"handoff-first"),
        Guard(previous.clone()),
    )
    .await
    .unwrap();
    let ct = role.confirm(incarnation).await.unwrap();
    assert!(matches!(ct, CtConfirmation::Confirmed { .. }));
    let reply = tokio::time::timeout(Duration::from_secs(2), reader.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply.packet[28..], b"from=198.18.71.20|handoff-first");
    role.send(
        incarnation,
        udp(tuple, b"handoff-queued"),
        Guard(previous.clone()),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(40)).await;
    controls[0].refresh_configuration().await.unwrap();
    controls[0].acknowledge_saved().await.unwrap();
    let lease = controls[0].renew_instance().await.unwrap();
    let instance = controls[0].execution_snapshot().unwrap().instance();
    controls[0]
        .renew_demand(
            &serde_json::from_value(serde_json::json!({
                "instance": instance,
                "requestId": "44444444-4444-4444-8444-444444444444",
                "expiresAt": meshway_model::timestamp::serialize(&lease.expires_at_ms, serde_json::value::Serializer).unwrap(),
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    controls[2].refresh_configuration().await.unwrap();
    controls[2].acknowledge_saved().await.unwrap();
    let next = controls[2].execution_snapshot().unwrap();
    assert!(previous.ensure_current().is_err());
    assert!(next.continuation_from(previous).is_ok());
    assert!(previous.retention_deadline().is_ok());
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(
        role.is_active(),
        "legal handoff must retain the role across three ticks"
    );
    assert!(
        role.send(
            incarnation,
            udp(tuple, b"must-not-leave"),
            Guard(previous.clone())
        )
        .await
        .is_err()
    );
    assert!(
        role.renew(incarnation, deadline(&clock, 20_000), scope.clone())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(40), reader.recv())
            .await
            .is_err(),
        "old queued replies cannot acquire successor authority"
    );
    let (_, ledger) = storage::Store::open(host.kernel.store.directory())
        .await
        .unwrap();
    let ledger = ledger.unwrap();
    let mut commands = command::Commands::default();
    let retained = conntrack::lookup_owned(&mut commands, tuple, ledger.program.mark)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ct,
        CtConfirmation::Confirmed {
            incarnation,
            conntrack_id: retained.id
        }
    );
    if matches!(case, Case::Expire) {
        tokio::time::sleep_until(previous.retention_deadline().unwrap().into()).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while role.is_active() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            role.retire().await.unwrap();
        })
        .await
        .unwrap();
        assert!(
            conntrack::lookup_owned(&mut commands, tuple, ledger.program.mark)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            objects::present(&mut commands, &ledger.program)
                .await
                .unwrap(),
            0
        );
        return;
    }
    host.install(next.clone()).await.unwrap();
    assert!(
        role.send(
            incarnation,
            udp(tuple, b"must-not-leave"),
            Guard(previous.clone())
        )
        .await
        .is_err()
    );
    role.renew(incarnation, deadline(&clock, 25_000), scope)
        .await
        .unwrap();
    assert_eq!(role.confirm(incarnation).await.unwrap(), ct);
    role.send(incarnation, udp(tuple, b"handoff-next"), Guard(next))
        .await
        .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(2), reader.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply.packet[28..], b"from=198.18.71.20|handoff-next");
    role.retire_flow(incarnation).await.unwrap();
}
