//! Artifact retention: what any configuration version can restore stays;
//! what a refresh replaced stays while a netmap that references it can still
//! be under a lease, then it is collected with its records.
use super::*;

async fn stored(s: &Server, digest: &str) -> (bool, i64) {
    let digest: meshway_rules::Digest = digest.parse().unwrap();
    s.app
        .executor
        .handle()
        .read(move |tx| {
            let records: i64 = tx.query_row(
                "SELECT (SELECT count(*) FROM rule_set_records WHERE digest=?1)
                      + (SELECT count(*) FROM rule_set_record_heads WHERE digest=?1)",
                [digest.as_bytes().as_slice()],
                |row| row.get(0),
            )?;
            Ok((
                crate::store::resources::artifacts::exists(tx, &digest)?,
                records,
            ))
        })
        .await
        .unwrap()
}

async fn collect(s: &Server, at: i64) -> usize {
    let sealer = s.app.sealer();
    s.app
        .executor
        .handle()
        .write(move |tx| crate::resources::artifacts::collect(tx, &sealer, at))
        .await
        .unwrap()
}

#[tokio::test]
async fn replaced_artifacts_outlive_their_leases_and_restorable_ones_stay() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    remote.set(200, "payload:\n - 8.8.8.0/26\n");
    let s = Server::open(&path, remote.clone()).await;
    let owner = s.register("retention-owner@example.com").await;
    let token = owner.token.expose();
    let network = s.create(token).await.id;
    let source = s.source(token, network).await;
    let mut input = source.configuration.clone();
    input.tags_owners.insert("tag:test".into(), vec![]);
    input.rule_sets.insert("remote".into(), remote.definition());
    input.routing.egress.rules.push(EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            rule_sets: vec!["remote".into()],
            ..Default::default()
        },
        action: RuleAction::Direct,
    });
    s.apply(token, network, source.revision, &input).await;
    let initial = s.view(token, network).await.artifact.unwrap().digest;
    // A Node: every binding reaches its netmap.
    enroll(&s, token, network, 81).await;

    // Two refreshes: `replaced` is bound by no version any more, `current`
    // is the version's refreshed binding, `initial` its own.
    remote.set(200, "payload:\n - 8.8.8.0/25\n");
    let replaced = s.refresh(token, network).await.artifact.unwrap().digest;
    remote.set(200, "payload:\n - 8.8.8.0/24\n");
    let refreshed = s.refresh(token, network).await;
    // Enrollment made a version of its own; the refreshes bound to it.
    let revision = refreshed.configuration_revision;
    let current = refreshed.artifact.unwrap().digest;
    assert!(initial != replaced && replaced != current && initial != current);
    let replaced_at = chrono::Utc::now().timestamp_millis();

    // Within the lease window nothing is collected.
    assert_eq!(collect(&s, replaced_at).await, 0);
    let window = crate::store::resources::artifacts::MAX_LEASE_MS;
    assert_eq!(collect(&s, replaced_at + window - 1_000).await, 0);
    assert!(stored(&s, &replaced).await.0);

    // A later version without the set: both bindings of the earlier one
    // stay restorable.
    let source = s.source(token, network).await;
    let mut without = source.configuration.clone();
    without.routing.egress.rules.clear();
    without.rule_sets.clear();
    s.apply(token, network, source.revision, &without).await;

    // Past the longest lease the replaced artifact and its records go.
    let after = replaced_at + window + 1_000;
    assert_eq!(collect(&s, after).await, 1);
    assert_eq!(stored(&s, &replaced).await, (false, 0));
    for kept in [&initial, &current] {
        let (exists, records) = stored(&s, kept).await;
        assert!(exists && records > 0, "{kept}");
    }
    assert_eq!(collect(&s, after).await, 0);

    // Restoring the earlier version uses either binding, both still stored.
    for choice in [
        RuleSetBindingChoice::Initial,
        RuleSetBindingChoice::Refreshed,
    ] {
        let source = s.source(token, network).await;
        let restored: ConfigurationAppliedResponse = json(
            s.client
                .post(s.network(network, "configuration"))
                .bearer_auth(token)
                .json(&ConfigurationRequest::Restore {
                    expected_revision: source.revision,
                    request_id: rid(),
                    restore_from: revision,
                    rule_set_bindings: Some(choice),
                }),
        )
        .await;
        let expected = match choice {
            RuleSetBindingChoice::Initial => &initial,
            RuleSetBindingChoice::Refreshed => &current,
        };
        let view = s.view(token, network).await;
        assert_eq!(view.configuration_revision, restored.revision);
        assert_eq!(&view.artifact.unwrap().digest, expected, "{choice:?}");
    }
    s.close().await;
}
