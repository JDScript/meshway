//! Integration tests with real SQLite job ordering and explicitly test-only
//! execution facts. Production providers are composed by the execution owner.
use super::service::*;
use super::stream::{StreamPoll, StreamSource};
use super::*;
use crate::effect::{Effect, EffectSink};
use crate::{
    App, Settings,
    observations::{
        TaskAdmission,
        service::{ObservationOperation, ObservationProvider},
    },
};
use meshway_model::{
    ObservationSubscriptionId, SessionId,
    observations::{MetricKind, ObservationSample, TrafficWindow},
};
use meshway_protocol::{events::StreamEvent, observations::*};
use rusqlite::Transaction;

#[derive(Clone)]
struct Fixture {
    admission: ReportAdmission,
    scope: VisibilityScope,
}
impl StateProvider for Fixture {
    type Configuration = Sequence;
    type Entities = Sequence;
    type Progress = Sequence;
    fn view(
        &self,
        _: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<ProviderView<Self>, Error> {
        if !matches!(identity, ReadIdentity::UserSession { digest, network } if digest[0] == 42 && *network == self.scope.network_id)
            || self.scope.expires_at <= now
        {
            return Err(Error::InvalidSession);
        }
        Ok(ViewContext {
            scope: self.scope.clone(),
            configuration: Sequence::MIN,
            entities: Sequence::MIN,
            progress: None,
            member_ids: Default::default(),
        })
    }
    fn report(
        &self,
        _: &Transaction<'_>,
        authorization: &str,
        _: DateTime<Utc>,
    ) -> Result<ReportAdmission, Error> {
        if authorization != "fixture-node" {
            return Err(Error::InvalidSession);
        }
        Ok(self.admission.clone())
    }
}
impl ObservationProvider for Fixture {
    fn scope(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<VisibilityScope, Error> {
        Ok(self.view(tx, identity, now)?.scope)
    }
    fn report(
        &self,
        tx: &Transaction<'_>,
        authorization: &str,
        now: DateTime<Utc>,
    ) -> Result<ReportAdmission, Error> {
        StateProvider::report(self, tx, authorization, now)
    }
    fn observation(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        _: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<TaskAdmission, Error> {
        self.view(tx, identity, now)?;
        Ok(TaskAdmission {
            configuration: self.admission.execution.clone().unwrap(),
            paths: self.admission.current().unwrap().paths.clone(),
            node_expires_at: self.admission.lease_expires_at,
            observation_id: None,
            unsupported_metrics: vec![],
            report: self.admission.clone(),
            session_id: session(),
            authorization_digest: self.scope.authorization_digest,
            session_expires_at: self.scope.expires_at,
        })
    }
    fn observation_session(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<SessionId, Error> {
        self.view(tx, identity, now)?;
        Ok(session())
    }
}
fn session() -> SessionId {
    uuid::Uuid::from_u128(200).to_string().parse().unwrap()
}
fn subscription(n: u128) -> ObservationSubscriptionId {
    uuid::Uuid::from_u128(n).to_string().parse().unwrap()
}
fn request(fixture: &Fixture, n: u128) -> CreateObservation {
    CreateObservation {
        service: meshway_configuration::execution::ObservationServiceScope::IcmpEcho,
        subscription_id: subscription(n),
        source_node_id: fixture.admission.node_id,
        target_node_id: fixture.admission.current().unwrap().paths[0][1],
        selection: ObservationPath::Auto,
        metrics: vec![MetricKind::Traffic],
        interval_ms: 1000,
    }
}
fn result(fixture: &Fixture, n: u128) -> ObservationResult {
    let now = Utc::now();
    ObservationResult {
        instance_id: fixture.admission.instance_id,
        configuration: fixture.admission.execution.clone().unwrap(),
        result: ObservationUpdate::Sample {
            path: fixture.admission.current().unwrap().paths[0].clone(),
            path_segment: Sequence::new(1).unwrap(),
            sample: Box::new(ObservationSample {
                measurement_id: uuid::Uuid::from_u128(n).to_string().parse().unwrap(),
                version: meshway_model::Version::MIN,
                window_start: now,
                window_end: now + Duration::nanoseconds(1),
                partial: false,
                probe: None,
                probe_counts: None,
                traffic: Some(TrafficWindow {
                    tx_bytes: Sequence::MIN,
                    rx_bytes: Sequence::MIN,
                    counter_generation: Sequence::new(1).unwrap(),
                }),
            }),
        },
    }
}
type Event = StreamEvent<Sequence, Sequence, Sequence>;
async fn drain(service: &StateService<Fixture>, stream: &str) -> Vec<(Option<String>, Event)> {
    let mut events = vec![];
    for _ in 0..32 {
        match service.poll(stream).await.unwrap() {
            StreamPoll::Frame(frame) => {
                events.push((frame.id, serde_json::from_str(&frame.data).unwrap()))
            }
            StreamPoll::Pending => return events,
            StreamPoll::Closed => panic!("stream closed"),
        }
    }
    panic!("stream did not drain")
}

#[tokio::test]
async fn shared_core_independent_tabs_cancel_pending_and_replay_without_stopping_other_demand() {
    let temp = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        temp.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let (admission, report, scope, _) = super::tests::setup();
    let fixture = Fixture { admission, scope };
    let identity = super::tests::identity(fixture.scope.network_id);
    let reader = app.state_reader();
    let state = app.state_service(fixture.clone()).unwrap();
    let other_view = app.state_service(fixture.clone()).unwrap();
    let observations = app.observation_service(fixture.clone());
    state
        .accept_report("fixture-node".into(), report)
        .await
        .unwrap();
    assert_eq!(
        other_view
            .query(identity.clone(), None)
            .await
            .unwrap()
            .nodes[&fixture.admission.node_id]
            .report_sequence,
        Some(Sequence::new(1).unwrap())
    );
    let first = observations
        .create_observation(identity.clone(), request(&fixture, 100))
        .await
        .unwrap();
    let second = observations
        .create_observation(identity.clone(), request(&fixture, 101))
        .await
        .unwrap();
    assert_eq!(first.id, second.id);
    assert_ne!(first.subscription_id, second.subscription_id);
    let stream = state.open_stream(identity.clone(), None).await.unwrap();
    let initial = drain(&state, &stream).await;
    let ready = initial
        .iter()
        .find_map(|(id, event)| matches!(event, Event::Ready { .. }).then(|| id.clone().unwrap()))
        .unwrap();
    observations
        .observation_result("fixture-node".into(), first.id, result(&fixture, 500))
        .await
        .unwrap();
    let StreamPoll::Frame(first_frame) = state.poll(&stream).await.unwrap() else {
        panic!()
    };
    assert!(matches!(
        serde_json::from_str::<Event>(&first_frame.data).unwrap(),
        Event::ObservationSample { .. }
    ));
    observations
        .observations(
            identity.clone(),
            Some(subscription(100)),
            ObservationOperation::Cancel,
        )
        .await
        .unwrap();
    let after = drain(&state, &stream).await;
    assert!(
        after
            .iter()
            .any(|(_, event)| matches!(event, Event::Reset { .. }))
    );
    assert!(!after.iter().any(|(_, event)| matches!(event, Event::ObservationSample { observation } if observation.subscription_id == Some(subscription(100)))));
    assert!(after.iter().any(|(_, event)| matches!(event, Event::ObservationStatus { observation } if observation.subscription_id == Some(subscription(100)) && observation.phase == ObservationPhase::Cancelled && observation.latest.is_none())));
    assert_eq!(
        observations
            .node_tasks("fixture-node".into())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    let resumed = state
        .open_stream(identity.clone(), Some(ready))
        .await
        .unwrap();
    let replayed = drain(&state, &resumed).await;
    assert!(!replayed.iter().any(|(_, event)| matches!(event, Event::ObservationSample { observation } if observation.subscription_id == Some(subscription(100)))));
    state.runtime.apply(Effect::SessionRevoked {
        session_id: session(),
    });
    assert!(
        observations
            .node_tasks("fixture-node".into())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    app.shutdown().await.unwrap();
    assert!(matches!(
        reader.location(&fixture.admission, Utc::now()),
        Err(Error::Unavailable(_))
    ));
}

#[tokio::test]
async fn resume_at_owner_and_global_capacity_reuses_existing_slot() {
    let temp = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        temp.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let (admission, _, scope, _) = super::tests::setup();
    let fixture = Fixture { admission, scope };
    let state = app.state_service(fixture.clone()).unwrap();
    let identity = super::tests::identity(fixture.scope.network_id);
    let stream = state.open_stream(identity.clone(), None).await.unwrap();
    let initial = drain(&state, &stream).await;
    let ready = initial
        .into_iter()
        .find_map(|(id, event)| matches!(event, Event::Ready { .. }).then_some(id).flatten())
        .unwrap();
    for _ in 0..3 {
        state.open_stream(identity.clone(), None).await.unwrap();
    }
    assert_eq!(
        state.open_stream(identity.clone(), None).await.unwrap_err(),
        Error::StateLimit
    );
    let resumed = state
        .open_stream(identity.clone(), Some(ready.clone()))
        .await
        .unwrap();
    assert!(matches!(
        state.poll(&stream).await.unwrap(),
        StreamPoll::Closed
    ));
    assert!(matches!(
        state.poll(&resumed).await.unwrap(),
        StreamPoll::Pending
    ));
    for n in (0..64).filter(|n| *n != 42) {
        let mut digest = [42; 32];
        digest[31] = n;
        for _ in 0..4 {
            state
                .open_stream(
                    ReadIdentity::UserSession {
                        digest,
                        network: fixture.scope.network_id,
                    },
                    None,
                )
                .await
                .unwrap();
        }
    }
    let mut digest = [42; 32];
    digest[31] = 99;
    assert_eq!(
        state
            .open_stream(
                ReadIdentity::UserSession {
                    digest,
                    network: fixture.scope.network_id
                },
                None
            )
            .await
            .unwrap_err(),
        Error::StateLimit
    );
    state.open_stream(identity, Some(ready)).await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn terminal_phase_without_path_or_sample_is_delivered_and_cannot_restart() {
    let temp = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        temp.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let (admission, _, scope, _) = super::tests::setup();
    let fixture = Fixture { admission, scope };
    let identity = super::tests::identity(fixture.scope.network_id);
    let state = app.state_service(fixture.clone()).unwrap();
    let observations = app.observation_service(fixture.clone());
    let task = observations
        .create_observation(identity.clone(), request(&fixture, 100))
        .await
        .unwrap();
    let stream = state.open_stream(identity.clone(), None).await.unwrap();
    drain(&state, &stream).await;
    let terminal = ObservationResult {
        instance_id: fixture.admission.instance_id,
        configuration: fixture.admission.execution.clone().unwrap(),
        result: ObservationUpdate::Phase {
            phase: ReportedObservationPhase::Unsupported,
        },
    };
    let task = observations
        .observation_result("fixture-node".into(), task.id, terminal)
        .await
        .unwrap();
    assert!(task.path.is_none() && task.latest.is_none());
    let events = drain(&state, &stream).await;
    assert!(events.iter().any(|(_, event)| matches!(event, Event::ObservationStatus { observation } if observation.phase == ObservationPhase::Unsupported && observation.path.is_none() && observation.latest.is_none())));
    assert!(
        observations
            .node_tasks("fixture-node".into())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        observations
            .observation_result("fixture-node".into(), task.id, result(&fixture, 501))
            .await
            .is_err()
    );
    assert!(
        observations
            .observations(
                identity,
                Some(subscription(100)),
                ObservationOperation::Renew
            )
            .await
            .is_err()
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn same_session_networks_keep_separate_identity_and_only_session_revocation_cancels_both() {
    let temp = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        temp.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let (admission, _, scope, _) = super::tests::setup();
    let first = Fixture { admission, scope };
    let mut second = first.clone();
    second.scope.network_id = uuid::Uuid::from_u128(900).to_string().parse().unwrap();
    second.admission.network_id = second.scope.network_id;
    let a = app.observation_service(first.clone());
    let b = app.observation_service(second.clone());
    let state = app.state_service(first.clone()).unwrap();
    let identity_a = super::tests::identity(first.scope.network_id);
    let identity_b = super::tests::identity(second.scope.network_id);
    a.create_observation(identity_a.clone(), request(&first, 100))
        .await
        .unwrap();
    b.create_observation(identity_b, request(&second, 101))
        .await
        .unwrap();
    assert_eq!(
        a.node_tasks("fixture-node".into())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(
        b.node_tasks("fixture-node".into())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    a.observations(
        identity_a,
        Some(subscription(100)),
        ObservationOperation::Cancel,
    )
    .await
    .unwrap();
    assert_eq!(
        b.node_tasks("fixture-node".into())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    state.runtime.apply(Effect::SessionRevoked {
        session_id: session(),
    });
    assert!(
        b.node_tasks("fixture-node".into())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn expiry_invalidates_queued_samples_and_preserves_terminal_then_removes_it() {
    let temp = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        temp.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let (admission, _, scope, _) = super::tests::setup();
    let fixture = Fixture { admission, scope };
    let identity = super::tests::identity(fixture.scope.network_id);
    let state = app.state_service(fixture.clone()).unwrap();
    let observations = app.observation_service(fixture.clone());
    let task = observations
        .create_observation(identity.clone(), request(&fixture, 100))
        .await
        .unwrap();
    observations
        .create_observation(identity.clone(), request(&fixture, 101))
        .await
        .unwrap();
    let stream = state.open_stream(identity.clone(), None).await.unwrap();
    let initial = drain(&state, &stream).await;
    let ready = initial
        .into_iter()
        .find_map(|(id, event)| matches!(event, Event::Ready { .. }).then_some(id).flatten())
        .unwrap();
    observations
        .observation_result("fixture-node".into(), task.id, result(&fixture, 500))
        .await
        .unwrap();
    assert!(matches!(
        state.poll(&stream).await.unwrap(),
        StreamPoll::Frame(_)
    ));
    let resumed = state.open_stream(identity, Some(ready)).await.unwrap();
    // Advance the task clock at the same serial runtime boundary, with replay
    // and a second subscriber's sample already queued.
    state
        .runtime
        .lock()
        .unwrap()
        .observations
        .expire(Utc::now() + Duration::seconds(31));
    let events = drain(&state, &resumed).await;
    assert!(
        !events
            .iter()
            .any(|(_, event)| matches!(event, Event::ObservationSample { .. }))
    );
    assert!(events.iter().any(|(_, event)| matches!(event, Event::ObservationStatus { observation } if observation.phase == ObservationPhase::Expired && observation.latest.is_none())));
    state
        .runtime
        .lock()
        .unwrap()
        .observations
        .expire(Utc::now() + Duration::seconds(91));
    let events = drain(&state, &resumed).await;
    assert_eq!(
        events
            .iter()
            .filter(|(_, event)| matches!(event, Event::ObservationRemoved { .. }))
            .count(),
        2
    );
    app.shutdown().await.unwrap();
}
