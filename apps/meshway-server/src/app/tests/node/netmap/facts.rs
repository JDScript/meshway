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
async fn reported_channels_reach_viewers_and_wake_them_on_change() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(false).await;
    let [a, b, _] = lab.nodes;
    let na = lab.netmap(a).await;
    let facts = lab.app.peer_facts();
    lab.report_endpoints(1, 1, &["10.1.0.1:23456"]).await;
    let first = facts.peer_facts(&na, None, None).unwrap().unwrap();
    assert!(first.updated[&b].connected.is_empty());
    // b's channel with a is content: it wakes a and travels in full.
    let wake = facts.watch(&na).unwrap();
    lab.report_channels(1, 2, &["10.1.0.1:23456"], &[a]).await;
    assert!(wake.has_changed());
    let next = facts
        .peer_facts(&na, Some(na.reference()), Some(first.cursor))
        .unwrap()
        .unwrap();
    assert_eq!(next.updated[&b].connected, [a]);
    // The same channel again only renews freshness.
    let wake = facts.watch(&na).unwrap();
    lab.report_channels(1, 3, &["10.1.0.1:23456"], &[a]).await;
    assert!(!wake.has_changed());
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

#[tokio::test]
async fn exit_resolver_availability_is_a_fact_of_a_serving_netmap() {
    use meshway_model::exit::{ExitDns, ExitDnsStatus, ExitDnsUnavailableReason};
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(true).await;
    let [a, b, _] = lab.nodes;
    // Only `a` uses exit `b`: `b` serves `a`'s DNS (a global exit rule would
    // also make `b` itself require an exit).
    let current = lab
        .app
        .configurations()
        .read(&lab.owner, lab.network, None)
        .await
        .unwrap();
    let mut config = current.document.configuration.clone();
    config.routing.egress.rules.clear();
    config.routing.egress.profiles.insert(
        "via-b".into(),
        meshway_configuration::RoutingProfile {
            rules: vec![],
            fallback: meshway_configuration::RouteAction::Exit {
                nodes: vec![format!("node:{b}")],
                via: vec![],
            },
        },
    );
    config
        .routing
        .egress
        .assignments
        .push(meshway_configuration::ProfileAssignment {
            target: vec![format!("node:{a}")],
            allowed: vec!["via-b".into()],
            default: "via-b".into(),
            locked: false,
        });
    lab.app
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
    let serving = lab.netmap(b).await;
    assert!(
        matches!(
            serving.netmap.dns.egress,
            meshway_configuration::netmap::DnsPlan::ServeExitResolver { .. }
        ),
        "{:?}",
        serving.netmap.dns.egress
    );
    let unavailable = ExitDns {
        netmap: serving.reference(),
        status: ExitDnsStatus::Unavailable {
            reason: ExitDnsUnavailableReason::MeshServer,
        },
    };
    let netmaps_before = lab.references().await;
    lab.report_exit_dns(1, 1, unavailable.clone())
        .await
        .unwrap();
    // The owner sees the exact reason in the Node's report.
    let identity =
        crate::state::service::ReadIdentity::user_session(&lab.owner, lab.network).unwrap();
    let view = lab
        .app
        .user_state_service()
        .unwrap()
        .query(identity, Some(vec![b]))
        .await
        .unwrap();
    let Some(meshway_protocol::state::VisibleReport::Full { report }) = &view.nodes[&b].report
    else {
        panic!("full report for the owner")
    };
    assert_eq!(report.exit_dns.as_ref(), Some(&unavailable));
    // A fact, never an input: no netmap changes.
    assert_eq!(lab.references().await, netmaps_before);
    // A netmap that serves no source, or another Node's netmap, is rejected.
    let not_serving = ExitDns {
        netmap: lab.netmap(a).await.reference(),
        status: ExitDnsStatus::Serving,
    };
    assert!(
        lab.report_exit_dns(0, 1, not_serving.clone())
            .await
            .is_err()
    );
    assert!(lab.report_exit_dns(1, 2, not_serving).await.is_err());
    lab.app.shutdown().await.unwrap();
}

/// `a`'s sync naming what it holds, and how long the Server held it.
async fn timed_sync(
    lab: &Lab,
    held: Option<&meshway_protocol::netmap::SyncResponse>,
) -> (meshway_protocol::netmap::SyncResponse, std::time::Duration) {
    let request = meshway_protocol::netmap::SyncRequest {
        netmap: Some(lab.applied[0]),
        state: held.and_then(|held| held.peer_facts.as_ref().map(|facts| facts.cursor)),
        server_key_generation: None,
        traversal_requests: Default::default(),
        associations: held.and_then(|held| {
            held.associations
                .as_ref()
                .map(|update| update.overlay.digest().unwrap())
        }),
        lease: held.map(|held| held.lease),
    };
    let started = std::time::Instant::now();
    let answer = lab
        .app
        .netmaps()
        .sync(&lab.tokens[0], request)
        .await
        .unwrap();
    (answer, started.elapsed())
}

/// b's facts as a sees them, with `expires_at` set in the ledger directly
/// (a reporter's routine renewal: same content, later bound, no wake-up).
fn set_expiry(
    lab: &Lab,
    facts: &meshway_protocol::PeerFacts,
    expires_at: chrono::DateTime<chrono::Utc>,
) {
    let mut runtime = lab.app.state_runtime.lock().unwrap();
    runtime
        .facts
        .record(
            lab.network,
            meshway_protocol::PeerFacts {
                expires_at,
                ..facts.clone()
            },
            chrono::Utc::now(),
        )
        .unwrap();
}

#[tokio::test]
async fn a_hold_delivers_a_renewal_before_the_held_fact_expires() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let mut lab = lab(false).await;
    lab.app.netmap_timing.hold = std::time::Duration::from_secs(6);
    let [a, b, _] = lab.nodes;
    lab.report_endpoints(1, 1, &["10.1.0.1:23456"]).await;
    let na = lab.netmap(a).await;
    let reported = lab
        .app
        .peer_facts()
        .peer_facts(&na, None, None)
        .unwrap()
        .unwrap()
        .updated[&b]
        .clone();
    // b's fact reaches a with 8 s left.
    let start = chrono::Utc::now();
    set_expiry(&lab, &reported, start + chrono::Duration::seconds(8));
    let (first, elapsed) = timed_sync(&lab, None).await;
    assert!(elapsed < std::time::Duration::from_secs(2), "{elapsed:?}");
    assert_eq!(
        first.peer_facts.as_ref().unwrap().updated[&b].expires_at,
        start + chrono::Duration::seconds(8)
    );
    // b keeps reporting: its renewal wakes nobody, yet a gets it 5 s before
    // the fact it holds would expire, well within its 6 s nominal hold.
    let renewed = start + chrono::Duration::seconds(60);
    let (second, elapsed) = tokio::join!(timed_sync(&lab, Some(&first)), async {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        set_expiry(&lab, &reported, renewed);
    })
    .0;
    assert!(
        elapsed >= std::time::Duration::from_millis(2_000)
            && elapsed < std::time::Duration::from_millis(4_500),
        "{elapsed:?}"
    );
    let facts = second.peer_facts.as_ref().unwrap();
    assert!(facts.updated.is_empty());
    assert_eq!(facts.refreshed[&b], renewed);
    // Facts now outlast the nominal hold: it is not shortened.
    let (_, elapsed) = timed_sync(&lab, Some(&second)).await;
    assert!(
        elapsed >= std::time::Duration::from_millis(5_000),
        "{elapsed:?}"
    );
    lab.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_stopped_reporter_shortens_one_hold_only() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let mut lab = lab(false).await;
    lab.app.netmap_timing.hold = std::time::Duration::from_secs(6);
    let [a, b, _] = lab.nodes;
    lab.report_endpoints(1, 1, &["10.1.0.1:23456"]).await;
    let na = lab.netmap(a).await;
    let reported = lab
        .app
        .peer_facts()
        .peer_facts(&na, None, None)
        .unwrap()
        .unwrap()
        .updated[&b]
        .clone();
    let until = chrono::Utc::now() + chrono::Duration::seconds(8);
    set_expiry(&lab, &reported, until);
    let (first, _) = timed_sync(&lab, None).await;
    // b stops reporting: the answer 5 s before its bound carries the same
    // bound, the last chance to renew it ...
    let (second, elapsed) = timed_sync(&lab, Some(&first)).await;
    assert!(
        elapsed < std::time::Duration::from_millis(4_500),
        "{elapsed:?}"
    );
    assert_eq!(second.peer_facts.as_ref().unwrap().refreshed[&b], until);
    // ... and the next hold is the nominal one again: a stopped reporter
    // never drives a sync loop; b's fact then expires on a.
    let (third, elapsed) = timed_sync(&lab, Some(&second)).await;
    assert!(
        elapsed >= std::time::Duration::from_millis(5_000),
        "{elapsed:?}"
    );
    assert_eq!(third.peer_facts.as_ref().unwrap().refreshed[&b], until);
    lab.app.shutdown().await.unwrap();
}

/// Two STUN observers see different mappings and renew a few seconds apart,
/// each keeping its previous observation until it expires: the listed order
/// changes twice per renewal. Order is not content, so the renewal wakes no
/// held sync (OPS11: every viewer of such a reporter answered on each swap).
#[tokio::test]
async fn staggered_stun_renewals_wake_no_hold() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let mut lab = lab(false).await;
    lab.app.netmap_timing.hold = std::time::Duration::from_secs(3);
    let [_, b, _] = lab.nodes;
    let now = chrono::Utc::now();
    let seen = |address: &str, observer: &str, age: i64| EndpointCandidate {
        address: address.into(),
        origin: EndpointSource::Stun {
            observer: observer.into(),
        },
        observed_at: now - chrono::Duration::seconds(age),
        expires_at: now + chrono::Duration::seconds(120 - age),
    };
    let (x, y) = ("203.0.113.53:3478", "198.51.100.7:19302");
    let (via_x, via_y) = ("203.0.113.10:40001", "203.0.113.10:40002");
    lab.report_stun(
        1,
        1,
        vec![
            seen(via_x, x, 70),
            seen(via_y, y, 60),
            seen(via_x, x, 5),
            seen(via_y, y, 0),
        ],
    )
    .await;
    let (first, _) = timed_sync(&lab, None).await;
    let order = |facts: &meshway_protocol::PeerFacts| {
        facts.listeners["udp"]
            .stun_candidates
            .iter()
            .map(|candidate| candidate.address.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        order(&first.peer_facts.as_ref().unwrap().updated[&b]),
        [via_x, via_y]
    );
    // x's previous observation expires first, then y's.
    let (second, elapsed) = tokio::join!(timed_sync(&lab, Some(&first)), async {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        lab.report_stun(
            1,
            2,
            vec![seen(via_y, y, 60), seen(via_x, x, 5), seen(via_y, y, 0)],
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        lab.report_stun(1, 3, vec![seen(via_x, x, 5), seen(via_y, y, 0)])
            .await;
    })
    .0;
    assert!(
        elapsed >= std::time::Duration::from_millis(2_500),
        "{elapsed:?}"
    );
    let facts = second.peer_facts.as_ref().unwrap();
    assert_eq!(facts.base, Some(first.peer_facts.as_ref().unwrap().cursor));
    assert!(facts.updated.is_empty());
    assert!(facts.refreshed.contains_key(&b));
    // A real mapping change still wakes the hold at once.
    let (third, elapsed) = tokio::join!(timed_sync(&lab, Some(&second)), async {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        lab.report_stun(
            1,
            4,
            vec![seen(via_x, x, 5), seen("203.0.113.10:40003", y, 0)],
        )
        .await;
    })
    .0;
    assert!(
        elapsed < std::time::Duration::from_millis(2_000),
        "{elapsed:?}"
    );
    assert_eq!(
        order(&third.peer_facts.as_ref().unwrap().updated[&b]),
        [via_x, "203.0.113.10:40003"]
    );
    lab.app.shutdown().await.unwrap();
}
