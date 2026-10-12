use super::*;

#[tokio::test]
async fn refreshed_rule_set_bindings_are_read_without_any_publication() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    let s = Server::open(&path, remote.clone()).await;
    let owner = s.register("bindings-owner@example.com").await;
    let token = owner.token.expose();
    let network = s.create(token).await.id;
    let source = s.source(token, network).await;
    let mut input = source.configuration.clone();
    input.tags_owners.insert("tag:test".into(), vec![]);
    input.rule_sets.insert("remote".into(), remote.definition());
    // A rule matching on the set: only referenced sets reach a netmap.
    input
        .routing
        .egress
        .rules
        .push(meshway_configuration::EgressRule {
            remark: String::new(),
            matcher: meshway_configuration::RouteMatch {
                domain: vec![],
                domain_suffix: vec![],
                ip_cidr: vec![],
                rule_sets: vec!["remote".into()],
            },
            action: meshway_configuration::RuleAction::Direct,
        });
    let applied = s.apply(token, network, source.revision, &input).await;
    // The version's own binding came with the apply; a refresh with new
    // content binds a different artifact to the same version.
    remote.set(200, "payload:\n - 8.8.8.0/24\n");
    let refreshed = s.refresh(token, network).await;
    assert!(refreshed.problem.is_none(), "{:?}", refreshed.problem);
    let artifact = refreshed.artifact.unwrap().id;

    // The current binding has its own storage and no execution publication table.
    let revision = applied.revision;
    let bound = s
        .app
        .executor
        .handle()
        .transact(rusqlite::TransactionBehavior::Immediate, move |tx| {
            assert!(!tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='execution_publications')",
                [],
                |row| row.get::<_, bool>(0)
            )?);
            let initial = BTreeMap::new();
            assert!(crate::store::resources::bindings::current(tx, network, revision)?.is_some());
            let bound = crate::resources::bindings::at_revision(tx, network, revision, &initial)?;
            let stored = crate::store::resources::bindings::current(tx, network, revision)?;
            assert_eq!(stored.as_ref(), Some(&bound));
            Ok((bound, Vec::new()))
        })
        .await
        .unwrap();
    assert_eq!(bound["remote"], artifact);

    // The next version retains the binding and its netmaps carry the content.
    let (node, ..) = enroll(&s, token, network, 61).await;
    let sealer = s.app.sealer();
    let node_id = node.node_id;
    let netmap = s
        .app
        .executor
        .handle()
        .read(move |tx| {
            let current = crate::store::netmap::current(tx, network, node_id)?.unwrap();
            crate::store::netmap::load(tx, &sealer, network, node_id, current.current)
        })
        .await
        .unwrap();
    assert_eq!(
        netmap.netmap.projection.rule_sets["remote"].artifact,
        artifact
    );
    s.close().await;
}
