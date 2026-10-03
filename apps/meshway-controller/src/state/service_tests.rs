//! Real SQLite/account authorization and HTTP SSE transport tests. The narrow
//! provider is intentionally test-only; no production fallback authorizes Nodes.
use super::service::*;
use super::stream::CommittedChange;
use super::*;
use crate::{
    App, Settings, http,
    secrets::{Purpose, Sealer},
};
use meshway_configuration::NetworkConfiguration;
use meshway_model::{MembershipId, MembershipRole};
use meshway_protocol::{NetworkActions, NetworkSummary, events::StreamEvent};
use rusqlite::{OptionalExtension, Transaction, params};
use std::{sync::Arc, time::Duration as StdDuration};
use tokio::sync::oneshot;

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
    ) -> Result<ViewContext<Self::Configuration, Self::Entities, Self::Progress>, StateError> {
        let ReadIdentity::UserSession { digest, network } = identity else {
            return Err(StateError::Unauthorized);
        };
        let session:Option<(Vec<u8>,i64)>=tx.query_row("SELECT u.id,s.expires_at FROM user_sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=?1 AND s.revoked_at IS NULL AND s.expires_at>?2 AND s.credential_version=u.credential_version AND u.disabled_at IS NULL",params![digest.as_slice(),now.timestamp_millis()],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        let (user, expires) = session.ok_or(StateError::Unauthorized)?;
        let (name,created,revision,membership,role,config):(String,i64,i64,Vec<u8>,String,Vec<u8>)=tx.query_row("SELECT n.name,n.created_at,n.current_revision,m.id,m.role,c.configuration FROM networks n JOIN memberships m ON m.network_id=n.id JOIN configuration_versions c ON c.network_id=n.id AND c.revision=n.current_revision WHERE n.id=?1 AND m.user_id=?2 AND m.revoked_at IS NULL",params![network.as_uuid().as_bytes().as_slice(),user],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).optional()?.ok_or(StateError::NotFound)?;
        let revision = Revision::new(revision.try_into().map_err(|_| StateError::Storage)?)
            .map_err(|_| StateError::Storage)?;
        let role = match role.as_str() {
            "owner" => MembershipRole::Owner,
            "admin" => MembershipRole::Admin,
            "member" => MembershipRole::Member,
            _ => return Err(StateError::Storage),
        };
        let membership_id =
            MembershipId::new(uuid::Uuid::from_slice(&membership).unwrap()).unwrap();
        let plaintext = self
            .sealer
            .open(*network, revision, Purpose::Configuration, &config)
            .map_err(|_| StateError::Storage)?;
        let configuration = meshway_configuration::restore_canonical_json(
            std::str::from_utf8(plaintext.as_bytes()).unwrap(),
        )
        .map_err(|_| StateError::Storage)?;
        let mut authorization = membership;
        authorization.extend_from_slice(format!("{role:?}").as_bytes());
        let scope = VisibilityScope {
            network_id: *network,
            configuration_revision: revision,
            authorization_digest: crate::resources::content_digest(&authorization),
            visible: configuration.nodes.keys().copied().collect(),
            full: configuration.nodes.keys().copied().collect(),
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
                actions: NetworkActions {
                    rename: role != MembershipRole::Member,
                    edit_configuration: role != MembershipRole::Member,
                    manage_members: role != MembershipRole::Member,
                },
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
    ) -> Result<ReportAdmission, StateError> {
        Err(StateError::Unauthorized)
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
    let subscription = service.open_stream(identity.clone(), None).await.unwrap();
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
    let subscription = service.open_stream(identity.clone(), None).await.unwrap();
    app.shutdown().await.unwrap();
    assert!(service.poll(&subscription).await.is_err());
    assert!(service.open_stream(identity, None).await.is_err());
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
                        measurement_id: event_id,
                        kind: ProbeKind::Path,
                        link_id: None,
                        transport: None,
                        path: admission.paths[0].clone(),
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
                    peer_node_id: admission.paths[0][1],
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
    let router = http::router(&app).merge(http::state::router(service.clone()));
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
    let publisher = service.publisher();
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
                publisher.publish_committed(CommittedChange::Authorization { network_id });
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
    assert!(resumed.next().await.is_none());
    drop(resumed);
    stop.send(()).unwrap();
    server.await.unwrap();
    app.shutdown().await.unwrap();
}
