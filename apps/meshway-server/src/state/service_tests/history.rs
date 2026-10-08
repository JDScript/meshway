use super::*;

#[tokio::test]
async fn history_budget_counts_visible_sources_before_pagination_and_preserves_cursor_scope() {
    use crate::observations::history;
    use meshway_model::{
        EventId,
        observations::{ObservationEvent, ObservationPayload},
    };
    use meshway_protocol::observations::*;
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let user = app
        .accounts()
        .register(
            "history-budget@example.test".into(),
            crate::accounts::Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let network = app
        .networks()
        .create(user.token.expose(), "history budget".into(), rid())
        .await
        .unwrap()
        .id;
    let service = app.node_state_service().unwrap();
    service
        .handle
        .execute(move |db| {
            let tx = db.transaction().unwrap();
            let (mut admission, _, mut scope, now) = super::super::tests::setup();
            admission.network_id = network;
            scope.network_id = network;
            let visible = admission.node_id;
            let hidden = NodeId::new(crate::accounts::random_uuid().unwrap()).unwrap();
            scope.visible = BTreeSet::from([visible]);
            scope.full = scope.visible.clone();
            let event = |index: usize| ObservationEvent {
                event_id: EventId::new(crate::accounts::random_uuid().unwrap()).unwrap(),
                configuration: admission.configurations[0].clone(),
                occurred_at: now - Duration::milliseconds(index as i64),
                payload: ObservationPayload::CollectionGap {
                    dropped: Sequence::new(1).unwrap(),
                    from: now - Duration::seconds(30),
                    to: now - Duration::seconds(20),
                },
            };
            // Populate through the same history acceptance and retention path. The
            // synthetic admission isolates the query's source-scope contract here.
            let mut other = admission.clone();
            other.node_id = hidden;
            for start in (0..=history::MAX_QUERY_ROWS).step_by(256) {
                let events = (start..(start + 256).min(history::MAX_QUERY_ROWS + 1))
                    .map(event)
                    .collect();
                history::accept(
                    &tx,
                    &other,
                    HistoryReport {
                        instance_id: other.instance_id,
                        events,
                    },
                    now,
                )
                .unwrap();
            }
            history::accept(
                &tx,
                &admission,
                HistoryReport {
                    instance_id: admission.instance_id,
                    events: vec![event(0), event(1)],
                },
                now,
            )
            .unwrap();
            let query = HistoryQuery {
                from: now - Duration::minutes(1),
                to: now + Duration::seconds(1),
                source_node_id: None,
                peer_node_id: None,
                mode: HistoryMode::Raw,
                bucket_seconds: None,
                cursor: None,
                limit: 1,
                kinds: None,
                link_id: None,
            };
            let (first, next) = history::query(&tx, &scope, &query, None, now).unwrap();
            let HistoryItems::Raw { items } = first.result else {
                panic!()
            };
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].source_node_id, visible);
            let next = next.unwrap();
            let (second, end) =
                history::query(&tx, &scope, &query, Some(next.clone()), now).unwrap();
            assert!(end.is_none());
            let HistoryItems::Raw { items: second } = second.result else {
                panic!()
            };
            assert_eq!(second.len(), 1);
            assert_ne!(second[0].event.event_id, items[0].event.event_id);
            let mut forbidden = query.clone();
            forbidden.source_node_id = Some(hidden);
            assert!(matches!(
                history::query(&tx, &scope, &forbidden, None, now),
                Err(Error::InvalidRequest)
            ));
            let mut expanded = scope.clone();
            expanded.visible.insert(hidden);
            expanded.full.insert(hidden);
            expanded.authorization_digest =
                crate::resources::content_digest(b"expanded history authority");
            assert!(matches!(
                history::query(&tx, &expanded, &query, Some(next), now),
                Err(Error::InvalidCursor)
            ));
            assert!(matches!(
                history::query(&tx, &expanded, &query, None, now),
                Err(Error::StateLimit)
            ));
            tx.commit().unwrap();
        })
        .await
        .unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn real_history_dedup_pagination_coverage_and_artifact_immutability() {
    use meshway_model::{
        EventId,
        observations::{ObservationEvent, ObservationPayload},
        state::{ProbeKind, ProbeObservation, ProbeOutcome},
    };
    use meshway_protocol::observations::*;
    let temp = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        temp.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let request =
        || meshway_model::RequestId::new(crate::accounts::random_uuid().unwrap()).unwrap();
    let issued = app
        .accounts()
        .register(
            "history@example.com".into(),
            crate::accounts::Secret::new("correct horse battery".into()),
            request(),
        )
        .await
        .unwrap();
    let network = app
        .networks()
        .create(issued.token.expose(), "history".into(), request())
        .await
        .unwrap();
    let service = app
        .state_service(SqlProvider {
            sealer: app.sealer(),
        })
        .unwrap();
    service
        .handle
        .execute(move |connection| {
            let (mut admission, _, mut scope, now) = super::super::tests::setup();
            admission.network_id = network.id;
            scope.network_id = network.id;
            let reference = admission.configurations[0].clone();
            let event_id = EventId::new(crate::accounts::random_uuid().unwrap()).unwrap();
            let event = ObservationEvent {
                event_id,
                configuration: reference.clone(),
                occurred_at: now,
                payload: ObservationPayload::ProbeCompleted {
                    probe: ProbeObservation {
                        channels: None,
                        measurement_id: event_id,
                        kind: ProbeKind::Path,
                        link_id: None,
                        transport: None,
                        path: admission.current().unwrap().paths[0].clone(),
                        publication_id: reference.execution_publication,
                        started_at: now - Duration::milliseconds(12),
                        completed_at: Some(now),
                        outcome: ProbeOutcome::Success,
                        duration_us: Some(Sequence::new(12_000).unwrap()),
                    },
                },
            };
            let batch = HistoryReport {
                instance_id: admission.instance_id,
                events: vec![event.clone()],
            };
            // P1 completed while it was installed. P2 is now executing; only
            // history may use P1's immutable context during the admission window.
            let mut next_context = admission.contexts[0].clone();
            next_context.configuration.execution_publication = meshway_model::PublicationId::new(crate::accounts::random_uuid().unwrap()).unwrap();
            next_context.configuration.digest = crate::resources::content_digest(b"P2");
            admission.execution = Some(next_context.configuration.clone());
            admission.contexts[0].installation_allowed = false;
            admission.configurations.push(next_context.configuration.clone());
            admission.contexts.push(next_context);
            let ObservationPayload::ProbeCompleted { probe } = &event.payload else { panic!() };
            assert!(super::super::validation::validate_probe(&admission, probe, now).is_err());
            let tx = connection.transaction().unwrap();
            assert_eq!(
                crate::observations::history::accept(&tx, &admission, batch.clone(), now)
                    .unwrap()
                    .accepted,
                1
            );
            assert_eq!(
                crate::observations::history::accept(
                    &tx,
                    &admission,
                    batch.clone(),
                    now + Duration::seconds(1)
                )
                .unwrap()
                .duplicates,
                1
            );
            let mut changed = batch.clone();
            changed.events[0].configuration.digest =
                crate::resources::content_digest(b"wrong configuration");
            assert!(crate::observations::history::accept(&tx, &admission, changed, now).is_err());
            let query = HistoryQuery {
                from: now - Duration::minutes(1),
                to: now + Duration::seconds(1),
                source_node_id: None,
                peer_node_id: None,
                mode: HistoryMode::Raw,
                bucket_seconds: None,
                cursor: None,
                limit: 1,
                kinds: None,
                link_id: None,
            };
            let second = ObservationEvent {
                event_id: EventId::new(crate::accounts::random_uuid().unwrap()).unwrap(),
                configuration: reference,
                occurred_at: now,
                payload: ObservationPayload::LinkOpened {
                    peer_node_id: admission.current().unwrap().paths[0][1],
                    link_id: "actual-channel".into(),
                    transport: "native_udp".into(),
                },
            };
            crate::observations::history::accept(
                &tx,
                &admission,
                HistoryReport {
                    instance_id: admission.instance_id,
                    events: vec![second],
                },
                now,
            )
            .unwrap();
            let (page, next) =
                crate::observations::history::query(&tx, &scope, &query, None, now).unwrap();
            assert!(next.is_some());
            let (second_page, last) =
                crate::observations::history::query(&tx, &scope, &query, next, now).unwrap();
            assert!(last.is_none());
            let HistoryItems::Raw {
                items: second_items,
            } = second_page.result
            else {
                panic!()
            };
            assert_eq!(second_items.len(), 1);
            assert_ne!(second_items[0].event.event_id, event_id);
            let HistoryItems::Raw { items } = page.result else {
                panic!()
            };
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].event.event_id, event_id);
            let mut buckets = query.clone();
            buckets.mode = HistoryMode::Buckets;
            buckets.bucket_seconds = Some(60);
            let (page, _) =
                crate::observations::history::query(&tx, &scope, &buckets, None, now).unwrap();
            let HistoryItems::Buckets { items } = page.result else {
                panic!()
            };
            assert_eq!(items[0].sample_count, 1);
            assert_eq!(items[0].mean_success_duration_us, Some(12_000.0));
            let gap = ObservationEvent {
                event_id: EventId::new(crate::accounts::random_uuid().unwrap()).unwrap(),
                configuration: admission.configurations[0].clone(), occurred_at: now,
                payload: ObservationPayload::CollectionGap {
                    from: now - Duration::seconds(30), to: now - Duration::seconds(1), dropped: Sequence::new(7).unwrap(),
                },
            };
            crate::observations::history::accept(&tx, &admission, HistoryReport { instance_id: admission.instance_id, events: vec![gap] }, now).unwrap();
            for mode in [HistoryMode::Raw, HistoryMode::Buckets, HistoryMode::RecentLinks] {
                let mut filtered = query.clone();
                // Gap notification itself is later than the selected window;
                // its original interval still intersects it.
                filtered.to = now - Duration::seconds(2);
                filtered.mode = mode;
                filtered.bucket_seconds = (mode == HistoryMode::Buckets).then_some(60);
                filtered.kinds = Some(vec![HistoryKind::ProbeCompleted]);
                filtered.link_id = Some("not-a-gap-link".into());
                let (page, _) = crate::observations::history::query(&tx, &scope, &filtered, None, now).unwrap();
                assert!(matches!(&page.coverage[0], HistoryCoverage::CollectionGap { dropped, .. } if dropped.get() == 7));
                let mut restricted = scope.clone();
                restricted.full.clear();
                let (page, _) = crate::observations::history::query(&tx, &restricted, &filtered, None, now).unwrap();
                assert!(page.coverage.is_empty());
            }
            // Model a capacity-retired payload while its retry admission is still
            // valid; retry must keep the tombstone rather than recreate a point.
            tx.execute("UPDATE observation_events SET payload=NULL", [])
                .unwrap();
            assert_eq!(
                crate::observations::history::accept(
                    &tx,
                    &admission,
                    batch,
                    now + Duration::seconds(2)
                )
                .unwrap()
                .duplicates,
                1
            );
            let (page, _) =
                crate::observations::history::query(&tx, &scope, &query, None, now).unwrap();
            let HistoryItems::Raw { items } = page.result else {
                panic!()
            };
            assert!(items.is_empty());
            let data = meshway_protocol::resources::RuleSetData {
                ip_cidr: vec!["10.0.0.0/8".into()],
                domain_suffix: vec![],
                domain: vec![],
            };
            let artifact =
                crate::resources::store_artifact(&tx, network.id, "inline", &data, now).unwrap();
            let again = crate::resources::store_artifact(
                &tx,
                network.id,
                "inline",
                &data,
                now + Duration::seconds(1),
            )
            .unwrap();
            assert_eq!(artifact.id, again.id);
            assert_eq!(
                crate::resources::load_artifact(&tx, network.id, artifact.id).unwrap(),
                data
            );
            assert!(
                tx.execute("UPDATE dataset_artifacts SET data=x'00'", [])
                    .is_err()
            );
            tx.execute(
                "UPDATE observation_events SET payload=?1 WHERE event_id=?2",
                params![serde_json::to_vec(&event).unwrap(), event_id.to_string()],
            )
            .unwrap();
            crate::observations::history::prune(&tx, network.id, now + Duration::days(8)).unwrap();
            let (page, _) =
                crate::observations::history::query(&tx, &scope, &query, None, now).unwrap();
            assert!(!page.coverage.is_empty());
            tx.commit().unwrap();
        })
        .await
        .unwrap();
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}
