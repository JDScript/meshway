use super::*;
use crate::state::facts::PeerFactsSource;
use meshway_model::exit::ExitStatus;

#[tokio::test]
async fn state_reports_record_facts_and_never_compile_or_change_netmaps() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(false).await;
    let [a, b, c] = lab.nodes;
    let before = [lab.netmap(a).await, lab.netmap(b).await];
    let netmaps_before = lab.references().await;
    for sequence in 1..=5 {
        let address = format!("10.1.0.{sequence}:23456");
        lab.report_endpoints(1, sequence, &[address.as_str()]).await;
    }
    // Facts and observations never change any netmap.
    assert_eq!(lab.references().await, netmaps_before);
    assert_eq!(lab.netmap(a).await.reference(), before[0].reference());
    assert_eq!(lab.netmap(b).await.reference(), before[1].reference());
    let facts = lab.app.peer_facts();
    let view = facts.peer_facts(&before[0], None, None).unwrap().unwrap();
    assert_eq!(view.base, None);
    assert_eq!(view.netmap_serial, before[0].netmap.serial);
    let udp = &view.updated[&b].listeners["udp"];
    assert_eq!(udp.phase, ListenerPhase::Listening);
    assert_eq!(
        udp.local_endpoints
            .iter()
            .map(|endpoint| endpoint.address.to_string())
            .collect::<Vec<_>>(),
        ["10.1.0.5:23456"]
    );
    // c sees nobody: its complete facts are empty, and b's netmap never
    // lists c, so b learns nothing about c either.
    let isolated = lab.netmap(c).await;
    assert!(
        facts
            .peer_facts(&isolated, None, None)
            .unwrap()
            .unwrap()
            .updated
            .is_empty()
    );
    lab.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn endpoint_changes_wake_only_viewers_that_see_the_reporter() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(false).await;
    let [a, b, c] = lab.nodes;
    let (na, nb, nc) = (
        lab.netmap(a).await,
        lab.netmap(b).await,
        lab.netmap(c).await,
    );
    let facts = lab.app.peer_facts();
    let first = facts.peer_facts(&na, None, None).unwrap().unwrap();
    assert!(first.updated.is_empty());
    let cursor = first.cursor;
    let wake_a = facts.watch(&na).unwrap();
    let wake_c = facts.watch(&nc).unwrap();
    lab.report_endpoints(1, 1, &["10.1.0.1:23456"]).await;
    assert!(wake_a.has_changed());
    assert!(!wake_c.has_changed());
    let next = facts
        .peer_facts(&na, Some(na.reference()), Some(cursor))
        .unwrap()
        .unwrap();
    assert_eq!(next.base, Some(cursor));
    assert_eq!(next.updated.keys().copied().collect::<Vec<_>>(), [b]);
    let freshness = facts
        .peer_facts(&na, Some(na.reference()), Some(next.cursor))
        .unwrap()
        .unwrap();
    assert_eq!(freshness.cursor, next.cursor);
    // Unchanged facts travel compactly: their freshness bound only.
    assert!(freshness.updated.is_empty());
    assert_eq!(freshness.refreshed[&b], next.updated[&b].expires_at);
    // A cursor from another netmap, or none held, restarts complete facts.
    let restarted = facts
        .peer_facts(&na, Some(nb.reference()), Some(next.cursor))
        .unwrap()
        .unwrap();
    assert_eq!(restarted.base, None);
    assert!(restarted.updated.contains_key(&b));
    // An identical report renews freshness without advancing the cursor or
    // waking anyone; the next ordinary response piggybacks its new deadline.
    let wake_a = facts.watch(&na).unwrap();
    lab.report_endpoints(1, 2, &["10.1.0.1:23456"]).await;
    assert!(!wake_a.has_changed());
    let refreshed = facts
        .peer_facts(&na, Some(na.reference()), Some(next.cursor))
        .unwrap()
        .unwrap();
    assert_eq!(refreshed.cursor, next.cursor);
    assert!(refreshed.updated.is_empty());
    assert!(refreshed.refreshed[&b] >= next.updated[&b].expires_at);
    // b's endpoint details reach a because a may dial b's listener; a has no
    // reported facts, and b's own netmap may not dial a, so nothing leaks.
    let to_b = facts.peer_facts(&nb, None, None).unwrap().unwrap();
    assert!(to_b.updated.is_empty());
    lab.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn expired_facts_stay_last_known_and_are_never_withdrawn_by_expiry() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(false).await;
    let [a, b, _] = lab.nodes;
    let na = lab.netmap(a).await;
    lab.report_endpoints(1, 1, &["10.1.0.1:23456"]).await;
    let facts = lab.app.peer_facts();
    let full = facts.peer_facts(&na, None, None).unwrap().unwrap();
    let last = full.updated[&b].clone();
    {
        let mut runtime = lab.app.state_runtime.lock().unwrap();
        // Past b's freshness bound: nothing is removed and nobody is woken;
        // the stale facts stay for the reporter to refresh or withdraw.
        let after = last.expires_at + chrono::Duration::minutes(10);
        let wake = runtime.facts.watch(&na);
        let stale = meshway_protocol::PeerFacts {
            observed_at: after - chrono::Duration::minutes(2),
            expires_at: after - chrono::Duration::minutes(1),
            ..last.clone()
        };
        assert!(!runtime.facts.record(lab.network, stale, after).unwrap());
        assert!(!wake.has_changed(), "an expired report replaces nothing");
        let later = runtime
            .facts
            .delta(&na, Some(na.reference()), Some(full.cursor))
            .unwrap()
            .unwrap();
        assert!(later.removed.is_empty());
        assert_eq!(
            later.refreshed[&b], last.expires_at,
            "a freshness-only answer keeps them"
        );
        assert_eq!(later.cursor, full.cursor);
        let restarted = runtime.facts.delta(&na, None, None).unwrap().unwrap();
        assert_eq!(restarted.updated[&b], last);
    }
    lab.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn retiring_a_node_withdraws_its_facts() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(false).await;
    let [a, b, _] = lab.nodes;
    let na = lab.netmap(a).await;
    lab.report_endpoints(1, 1, &["10.1.0.1:23456"]).await;
    let facts = lab.app.peer_facts();
    let full = facts.peer_facts(&na, None, None).unwrap().unwrap();
    assert!(full.updated.contains_key(&b));
    let wake = facts.watch(&na).unwrap();
    let revision = lab
        .app
        .configurations()
        .read(&lab.owner, lab.network, None)
        .await
        .unwrap()
        .revision;
    lab.app
        .configurations()
        .apply(
            &lab.owner,
            lab.network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::DeleteNode { node_id: b }],
            },
            rid(),
        )
        .await
        .unwrap();
    assert!(wake.has_changed());
    let after = facts
        .peer_facts(&na, Some(na.reference()), Some(full.cursor))
        .unwrap()
        .unwrap();
    assert_eq!(after.removed, [b]);
    assert!(after.updated.is_empty());
    lab.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn exit_readiness_is_only_a_fact() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(true).await;
    let [a, b, _] = lab.nodes;
    let before = lab.netmap(a).await;
    let role = lab.netmap(b).await.netmap.exit_role.clone().unwrap();
    let netmaps_before = lab.references().await;
    lab.report_exit(1, 1, ExitStatus::Ready).await;
    let facts = lab.app.peer_facts();
    let ready = facts.peer_facts(&before, None, None).unwrap().unwrap();
    let exit = ready.updated[&b].exit.clone().unwrap();
    assert_eq!(exit.status, ExitStatus::Ready);
    assert_eq!(exit.role_generation, role.generation);
    lab.report_exit(1, 2, ExitStatus::Probing).await;
    let probing = facts
        .peer_facts(&before, Some(before.reference()), Some(ready.cursor))
        .unwrap()
        .unwrap();
    assert_eq!(
        probing.updated[&b].exit.as_ref().unwrap().status,
        ExitStatus::Probing
    );
    // Facts and observations never change any netmap.
    assert_eq!(lab.references().await, netmaps_before);
    assert_eq!(lab.netmap(a).await.reference(), before.reference());
    lab.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn unrelated_configuration_revision_does_not_withdraw_current_exit_readiness() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(true).await;
    let [_, exit, isolated] = lab.nodes;
    let before = lab.netmap(exit).await.reference();
    lab.report_exit(1, 1, ExitStatus::Ready).await;
    let current = lab
        .app
        .configurations()
        .read(&lab.owner, lab.network, None)
        .await
        .unwrap();
    let mut config = current.document.configuration.clone();
    config.nodes.get_mut(&isolated).unwrap().remark = "unrelated isolated node".into();
    let applied = lab
        .app
        .configurations()
        .apply(
            &lab.owner,
            lab.network,
            meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(canonical_json(&config).unwrap().expose().into()),
            },
            rid(),
        )
        .await
        .unwrap();
    assert_eq!(lab.netmap(exit).await.reference(), before);
    let network = lab.network;
    let revision = applied.revision;
    assert!(revision > before.revision);
    let inputs = lab
        .app
        .executor
        .handle()
        .read(move |tx| {
            crate::state::exit::inputs(tx, network, revision, chrono::Utc::now().timestamp_millis())
        })
        .await
        .unwrap();
    assert!(inputs.ready_until_ms.contains_key(&exit));
    lab.app.shutdown().await.unwrap();
}
