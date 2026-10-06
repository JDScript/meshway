//! Real SQLite/account authorization and HTTP SSE transport tests. The narrow
//! provider is intentionally test-only; no production fallback authorizes Nodes.
use super::service::*;
use super::*;
use crate::effect::{Effect, EffectSink};
use crate::{
    App, Settings, http,
    secrets::{Purpose, Sealer},
};
use meshway_configuration::NetworkConfiguration;
use meshway_model::{MembershipId, MembershipRole};
use meshway_protocol::{NetworkSummary, events::StreamEvent};
use rusqlite::{OptionalExtension, Transaction, params};
use std::{sync::Arc, time::Duration as StdDuration};
use tokio::sync::oneshot;

fn rid() -> meshway_model::RequestId {
    meshway_model::RequestId::new(crate::accounts::random_uuid().unwrap()).unwrap()
}

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
            let (mut admission, _, mut scope, now) = super::tests::setup();
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

struct SqlProvider {
    sealer: Arc<Sealer>,
}
impl StateProvider for SqlProvider {
    type Configuration = NetworkConfiguration;
    type Entities = NetworkSummary;
    type Progress = Sequence;
    fn view(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<ViewContext<Self::Configuration, Self::Entities, Self::Progress>, Error> {
        let ReadIdentity::UserSession { digest, network } = identity else {
            return Err(Error::InvalidSession);
        };
        let session:Option<(Vec<u8>,i64)>=tx.query_row("SELECT u.id,s.expires_at FROM user_sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=?1 AND s.revoked_at IS NULL AND s.expires_at>?2 AND s.credential_version=u.credential_version AND u.disabled_at IS NULL",params![digest.as_slice(),now.timestamp_millis()],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        let (user, expires) = session.ok_or(Error::InvalidSession)?;
        let (name,created,revision,membership,role,config):(String,i64,i64,Vec<u8>,String,Vec<u8>)=tx.query_row("SELECT n.name,n.created_at,n.current_revision,m.id,m.role,c.configuration FROM networks n JOIN memberships m ON m.network_id=n.id JOIN configuration_versions c ON c.network_id=n.id AND c.revision=n.current_revision WHERE n.id=?1 AND m.user_id=?2 AND m.revoked_at IS NULL",params![network.as_uuid().as_bytes().as_slice(),user],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).optional()?.ok_or(Error::NotFound)?;
        let revision = Revision::new(
            revision
                .try_into()
                .map_err(|_| Error::decode("networks.current_revision"))?,
        )
        .map_err(|_| Error::decode("networks.current_revision"))?;
        let role = match role.as_str() {
            "owner" => MembershipRole::Owner,
            "admin" => MembershipRole::Admin,
            "member" => MembershipRole::Member,
            _ => return Err(Error::decode("memberships.role")),
        };
        let membership_id =
            MembershipId::new(uuid::Uuid::from_slice(&membership).unwrap()).unwrap();
        let plaintext = self
            .sealer
            .open(*network, revision, Purpose::Configuration, &config)?;
        let configuration = meshway_configuration::restore_canonical_json(
            std::str::from_utf8(plaintext.as_bytes()).unwrap(),
        )
        .map_err(|_| Error::decode("configuration_versions.configuration"))?;
        let mut authorization = membership;
        authorization.extend_from_slice(format!("{role:?}").as_bytes());
        let scope = VisibilityScope {
            network_id: *network,
            configuration_revision: revision,
            authorization_digest: crate::resources::content_digest(&authorization),
            observation_authorization_digest: crate::resources::content_digest(&authorization),
            visible: configuration.nodes.keys().copied().collect(),
            hostnames: configuration
                .nodes
                .iter()
                .map(|(id, node)| (*id, node.hostname.clone()))
                .collect(),
            full: configuration.nodes.keys().copied().collect(),
            endpoint_resources: None,
            observation_paths: Default::default(),
            mesh_addresses: Default::default(),
            expires_at: DateTime::from_timestamp_millis(expires).unwrap(),
            current: BTreeMap::new(),
        };
        Ok(ViewContext {
            scope,
            configuration,
            entities: NetworkSummary {
                id: *network,
                name,
                created_at: DateTime::from_timestamp_millis(created).unwrap(),
                current_revision: revision,
                membership_id,
                role,
                actions: crate::store::authorization::network_actions(role),
            },
            progress: None,
            member_ids: std::collections::BTreeSet::from([membership_id]),
        })
    }
    fn report(
        &self,
        _: &Transaction<'_>,
        _: &str,
        _: DateTime<Utc>,
    ) -> Result<ReportAdmission, Error> {
        Err(Error::InvalidSession)
    }
}
type TestEvent = StreamEvent<NetworkConfiguration, NetworkSummary, Sequence>;

#[test]
fn sse_export_declares_bytes_and_a_separate_json_event_schema() {
    use utoipa::OpenApi as _;
    let mut spec = http::state::StateApi::openapi();
    let path = "/api/v1/networks/{networkId}/events";
    http::events::declare_sse::<TestEvent>(&mut spec, path).unwrap();
    let json = serde_json::to_value(spec).unwrap();
    assert_eq!(json["openapi"], "3.1.0");
    assert_eq!(
        json["paths"][path]["get"]["responses"]["200"]["content"]["text/event-stream"]["schema"]["type"],
        "string"
    );
    assert_eq!(
        json["paths"][path]["get"]["x-meshway-sse"]["format"],
        "json-data"
    );
    let reference = json["paths"][path]["get"]["x-meshway-sse"]["eventSchema"]["$ref"]
        .as_str()
        .unwrap();
    assert!(json.pointer(reference.trim_start_matches('#')).is_some());
    let header = json["paths"][path]["get"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|parameter| parameter["name"] == "Last-Event-ID")
        .unwrap();
    assert_eq!(header["required"], false);
    assert_eq!(header["schema"]["type"], "string");
    assert_eq!(
        json["components"]["schemas"]["GeoPosition"]["properties"]["latitude"]["minimum"],
        -90
    );
}

#[tokio::test]
async fn pending_snapshot_is_discarded_on_role_change_and_shutdown_closes_streams() {
    use super::stream::{StreamPoll, StreamSource};
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
            "scope@example.com".into(),
            crate::accounts::Secret::new("correct horse battery".into()),
            request(),
        )
        .await
        .unwrap();
    let network = app
        .networks()
        .create(issued.token.expose(), "scope".into(), request())
        .await
        .unwrap();
    let service = app
        .state_service(SqlProvider {
            sealer: app.sealer(),
        })
        .unwrap();
    let identity = ReadIdentity::UserSession {
        digest: crate::accounts::token_digest(issued.token.expose()).unwrap(),
        network: network.id,
    };
    let subscription = service
        .open_stream(identity.clone(), None, rid())
        .await
        .unwrap();
    assert!(matches!(
        service.poll(&subscription).await.unwrap(),
        StreamPoll::Frame(_)
    ));
    let network_id = network.id;
    service
        .handle
        .execute(move |connection| {
            connection
                .execute(
                    "UPDATE memberships SET role='member',version=version+1 WHERE network_id=?1",
                    [network_id.as_uuid().as_bytes().as_slice()],
                )
                .unwrap();
        })
        .await
        .unwrap();
    assert!(matches!(
        service.poll(&subscription).await.unwrap(),
        StreamPoll::Closed
    ));
    let subscription = service
        .open_stream(identity.clone(), None, rid())
        .await
        .unwrap();
    app.shutdown().await.unwrap();
    assert!(service.poll(&subscription).await.is_err());
    assert!(service.open_stream(identity, None, rid()).await.is_err());
}

#[tokio::test]
async fn streams_wait_for_commits_and_deadlines_instead_of_polling() {
    use super::stream::{StreamPoll, StreamSource};
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let issued = app
        .accounts()
        .register(
            "wake@example.com".into(),
            crate::accounts::Secret::new("correct horse battery".into()),
            rid(),
        )
        .await
        .unwrap();
    let network = app
        .networks()
        .create(issued.token.expose(), "wake".into(), rid())
        .await
        .unwrap();
    let service = app
        .state_service(SqlProvider {
            sealer: app.sealer(),
        })
        .unwrap();
    let identity = ReadIdentity::UserSession {
        digest: crate::accounts::token_digest(issued.token.expose()).unwrap(),
        network: network.id,
    };
    let subscription = service
        .open_stream(identity.clone(), None, rid())
        .await
        .unwrap();
    while matches!(
        service.poll(&subscription).await.unwrap(),
        StreamPoll::Frame(_)
    ) {}
    // Silence: nothing wakes the stream.
    assert!(
        tokio::time::timeout(StdDuration::from_millis(300), service.wait(&subscription))
            .await
            .is_err()
    );
    // The session now ends in two seconds; the next projection picks that up
    // as its deadline.
    let digest = crate::accounts::token_digest(issued.token.expose()).unwrap();
    let ends = (Utc::now() + Duration::milliseconds(2000)).timestamp_millis();
    service
        .handle
        .execute(move |connection| {
            connection
                .execute(
                    "UPDATE user_sessions SET expires_at=?1 WHERE token_hash=?2",
                    params![ends, digest.as_slice()],
                )
                .unwrap();
        })
        .await
        .unwrap();
    // A committed change for this network wakes it at once, with a frame to show.
    let started = std::time::Instant::now();
    let renamed = app
        .networks()
        .rename(
            issued.token.expose(),
            network.id,
            "woken".into(),
            network.name.clone(),
            rid(),
        )
        .await
        .unwrap();
    assert_eq!(renamed.name, "woken");
    tokio::time::timeout(StdDuration::from_secs(1), service.wait(&subscription))
        .await
        .expect("a commit wakes the subscription");
    assert!(started.elapsed() < StdDuration::from_millis(500));
    assert!(matches!(
        service.poll(&subscription).await.unwrap(),
        StreamPoll::Frame(_)
    ));
    while matches!(
        service.poll(&subscription).await.unwrap(),
        StreamPoll::Frame(_)
    ) {}
    // A commit for another network does not.
    let other = app
        .networks()
        .create(issued.token.expose(), "elsewhere".into(), rid())
        .await
        .unwrap();
    assert_ne!(other.id, network.id);
    assert!(
        tokio::time::timeout(StdDuration::from_millis(300), service.wait(&subscription))
            .await
            .is_err()
    );
    // The session's own expiry is a deadline the subscription computed itself.
    tokio::time::timeout(StdDuration::from_secs(3), service.wait(&subscription))
        .await
        .expect("the session deadline wakes the subscription");
    assert!(Utc::now().timestamp_millis() >= ends);
    assert!(matches!(
        service.poll(&subscription).await.unwrap(),
        StreamPoll::Frame(_) | StreamPoll::Closed
    ));
    // Shutdown wakes every waiter.
    let fresh = app
        .accounts()
        .login(
            "wake@example.com".into(),
            crate::accounts::Secret::new("correct horse battery".into()),
            rid(),
        )
        .await
        .unwrap();
    let identity = ReadIdentity::UserSession {
        digest: crate::accounts::token_digest(fresh.token.expose()).unwrap(),
        network: network.id,
    };
    let subscription = service.open_stream(identity, None, rid()).await.unwrap();
    while matches!(
        service.poll(&subscription).await.unwrap(),
        StreamPoll::Frame(_)
    ) {}
    let waiting = tokio::spawn({
        let service = service.clone();
        let subscription = subscription.clone();
        async move { tokio::time::timeout(StdDuration::from_secs(1), service.wait(&subscription)).await }
    });
    tokio::time::sleep(StdDuration::from_millis(50)).await;
    app.shutdown().await.unwrap();
    waiting
        .await
        .unwrap()
        .expect("shutdown wakes the subscription");
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
            let (mut admission, _, mut scope, now) = super::tests::setup();
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
            assert!(super::validation::validate_probe(&admission, probe, now).is_err());
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
struct Frames {
    response: reqwest::Response,
    buffer: Vec<u8>,
}
impl Frames {
    async fn next(&mut self) -> Option<(Option<String>, TestEvent)> {
        tokio::time::timeout(StdDuration::from_secs(5), async {
            loop {
                if let Some(index) = self.buffer.windows(2).position(|pair| pair == b"\n\n") {
                    let bytes: Vec<_> = self.buffer.drain(..index + 2).collect();
                    let text = std::str::from_utf8(&bytes).unwrap();
                    let id = text
                        .lines()
                        .find_map(|line| line.strip_prefix("id: "))
                        .map(str::to_owned);
                    let data = text
                        .lines()
                        .filter_map(|line| line.strip_prefix("data: "))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if data.is_empty() {
                        continue;
                    }
                    return Some((id, serde_json::from_str(&data).unwrap()));
                }
                let chunk = self.response.chunk().await.unwrap()?;
                // Deliberately consume arbitrary byte fragments, including
                // UTF-8 boundaries; framing never assumes one HTTP chunk/frame.
                for piece in chunk.chunks(3) {
                    self.buffer.extend_from_slice(piece);
                }
            }
        })
        .await
        .expect("stream must make bounded progress")
    }
}
async fn open(client: &reqwest::Client, url: &str, token: &str, cursor: Option<&str>) -> Frames {
    let mut request = client.get(url).bearer_auth(token);
    if let Some(cursor) = cursor {
        request = request.header("Last-Event-ID", cursor);
    }
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    Frames {
        response,
        buffer: vec![],
    }
}
#[tokio::test]
async fn http_snapshot_cursor_replay_commit_cancellation_and_logout() {
    let temp = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        temp.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let accounts = app.accounts();
    let issued = accounts
        .register(
            "owner@example.com".into(),
            crate::accounts::Secret::new("correct horse battery".into()),
            meshway_model::RequestId::new(crate::accounts::random_uuid().unwrap()).unwrap(),
        )
        .await
        .unwrap();
    let token = issued.token.expose().to_owned();
    let network = app
        .networks()
        .create(
            &token,
            "初始 network".into(),
            meshway_model::RequestId::new(crate::accounts::random_uuid().unwrap()).unwrap(),
        )
        .await
        .unwrap();
    let service = app
        .state_service(SqlProvider {
            sealer: app.sealer(),
        })
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/api/v1/networks/{}/events",
        listener.local_addr().unwrap(),
        network.id
    );
    let router =
        http::state::user_router(service.clone()).merge(http::state::node_router(service.clone()));
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut stream = open(&client, &url, &token, None).await;
    assert!(matches!(
        stream.next().await.unwrap(),
        (None, TestEvent::ConfigSnapshot { .. })
    ));
    assert!(matches!(
        stream.next().await.unwrap(),
        (None, TestEvent::EntitiesSnapshot { .. })
    ));
    assert!(matches!(
        stream.next().await.unwrap(),
        (None, TestEvent::StateSnapshot { .. })
    ));
    let (ready, event) = stream.next().await.unwrap();
    assert!(matches!(event, TestEvent::Ready { .. }));
    let ready = ready.unwrap();
    let runtime = Arc::clone(&service.runtime);
    let handle = service.handle.clone();
    let network_id = network.id;
    let (committed, committed_rx) = oneshot::channel();
    let mutation = tokio::spawn(async move {
        handle
            .execute(move |connection| {
                let tx = connection.transaction().unwrap();
                tx.execute(
                    "UPDATE networks SET name='committed after client cancellation' WHERE id=?1",
                    [network_id.as_uuid().as_bytes().as_slice()],
                )
                .unwrap();
                tx.commit().unwrap();
                runtime.apply(Effect::Authorization { network_id });
                let _ = committed.send(());
            })
            .await
    });
    committed_rx.await.unwrap();
    mutation.abort();
    let (changed, event) = stream.next().await.unwrap();
    let changed = changed.unwrap();
    match event {
        TestEvent::EntitiesDelta {
            base_cursor,
            updated: entities,
            ..
        } => {
            assert_eq!(base_cursor, ready);
            assert_eq!(entities.name, "committed after client cancellation");
        }
        _ => panic!("expected entity replacement"),
    }
    drop(stream);
    let mut resumed = open(&client, &url, &token, Some(&ready)).await;
    let (replayed, event) = resumed.next().await.unwrap();
    assert_eq!(replayed, Some(changed));
    assert!(matches!(event, TestEvent::EntitiesDelta { .. }));
    accounts
        .logout(
            &token,
            meshway_model::RequestId::new(crate::accounts::random_uuid().unwrap()).unwrap(),
        )
        .await
        .unwrap();
    loop {
        let (_, event) = resumed.next().await.expect("explicit session closure");
        if matches!(
            event,
            TestEvent::Closed {
                reason: meshway_protocol::events::ClosedReason::SessionExpired
            }
        ) {
            break;
        }
        assert!(matches!(event, TestEvent::StateDelta { .. }));
    }
    assert!(resumed.next().await.is_none());
    drop(resumed);
    stop.send(()).unwrap();
    server.await.unwrap();
    app.shutdown().await.unwrap();
}
