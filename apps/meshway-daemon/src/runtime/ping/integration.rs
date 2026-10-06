//! Real held-key Server/Control confirmations and deliberately faulted HTTP.
//! Sample payloads below test stream forwarding, not packet measurement.
use super::*;
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::IntoResponse,
};
use meshway_control::{Control, ObservationLease};
use meshway_control_client::*;
use meshway_model::{observations::*, state::*, *};
use meshway_protocol::{observations::*, *};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

fn id<T: TryFrom<uuid::Uuid>>() -> T
where
    T::Error: std::fmt::Debug,
{
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).unwrap();
    uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .try_into()
        .unwrap()
}

#[derive(Default)]
struct Fault {
    creates: AtomicUsize,
    create_gate: Arc<CreateGate>,
    blocked: AtomicBool,
    renewing: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[derive(Default)]
pub(in crate::runtime) struct CreateGate {
    held: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl CreateGate {
    pub async fn entered(&self) {
        self.entered.notified().await;
    }
    pub fn release(&self) {
        self.held.store(false, Ordering::Release);
        self.release.notify_one();
    }
}
async fn fault(
    State(fault): State<Arc<Fault>>,
    request: Request,
    next: Next,
) -> axum::response::Response {
    if request.method() == axum::http::Method::POST
        && request.uri().path() == "/agent-api/v1/observations"
    {
        fault.creates.fetch_add(1, Ordering::AcqRel);
        if fault.create_gate.held.load(Ordering::Acquire) {
            fault.create_gate.entered.notify_one();
            fault.create_gate.release.notified().await;
        }
    }
    if fault.blocked.load(Ordering::Acquire) {
        if request.uri().path().starts_with("/agent-api/v1/tasks/") {
            return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        if request
            .uri()
            .path()
            .starts_with("/agent-api/v1/observations/")
            && request.uri().path().ends_with("/renew")
        {
            fault.renewing.notify_one();
            fault.release.notified().await;
            return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    next.run(request).await
}

pub(in crate::runtime) struct Fixture {
    app: meshway_server::App,
    server: tokio::task::JoinHandle<()>,
    stop: tokio::sync::oneshot::Sender<()>,
    pub(in crate::runtime) controls: Vec<Control>,
    pub(in crate::runtime) request: CreateNodeObservation,
    old_status: meshway_control::StatusSnapshot,
    task: ObservationTask,
    lease: ObservationLease,
    fault: Arc<Fault>,
    _directory: tempfile::TempDir,
}
impl Fixture {
    pub(in crate::runtime) async fn open() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let app = meshway_server::App::open(meshway_server::Settings::new(root.join("server")))
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let faults = Arc::new(Fault::default());
        let router = meshway_server::http::router(&app)
            .layer(axum::middleware::from_fn_with_state(faults.clone(), fault));
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let client = Client::new(
            &origin,
            reqwest::Client::builder().no_proxy(),
            16 * 1024 * 1024,
        )
        .unwrap();
        let user = client
            .register(&RegisterRequest {
                email: "lease-stream@example.test".into(),
                password: WireSecret::new("lease stream test password".into()),
            })
            .await
            .unwrap();
        let network = client
            .create_network(
                &CreateNetworkRequest {
                    request_id: id(),
                    name: "lease-stream".into(),
                },
                user.token.expose(),
            )
            .await
            .unwrap();
        let mut controls = Vec::new();
        for name in ["lease-source", "lease-target"] {
            let mut control = Control::open(
                &root.join(name),
                &origin,
                reqwest::Client::builder().no_proxy(),
            )
            .unwrap();
            let pending = control.begin_enrollment(name).await.unwrap();
            client
                .approve_device_authorization(
                    &ApproveDeviceAuthorizationPath {
                        enrollment_request_id: pending.id,
                    },
                    &ApproveEnrollmentRequest {
                        network_id: network.network.id,
                    },
                    user.token.expose(),
                )
                .await
                .unwrap();
            control.poll_enrollment().await.unwrap();
            control.redeem_enrollment().await.unwrap();
            control.authenticate().await.unwrap();
            control
                .acquire_instance(BTreeMap::from([
                    (
                        "feature:path-observation".into(),
                        Capability::Feature { version: 1 },
                    ),
                    (
                        "transport:native_udp".into(),
                        Capability::Transport {
                            roles: vec![TransportRole::Dial, TransportRole::Listen],
                            streams: vec![StreamKind::Udp],
                        },
                    ),
                ]))
                .await
                .unwrap();
            controls.push(control);
        }
        let source = client
            .get_configuration_source(
                &GetConfigurationSourcePath {
                    network_id: network.network.id,
                },
                &GetConfigurationSourceQuery::default(),
                user.token.expose(),
            )
            .await
            .unwrap();
        let mut configuration = source.configuration;
        let a = controls[0].status().binding.unwrap().node_id;
        let b = controls[1].status().binding.unwrap().node_id;
        configuration.access = vec![meshway_configuration::AccessRule {
            remark: String::new(),
            src: vec![format!("node:{a}")],
            dst: vec![format!("node:{b}")],
            ip: vec!["icmp:*".into()],
            bidirectional: false,
        }];
        for node in configuration.nodes.values_mut() {
            use meshway_configuration::transport::*;
            node.listeners.insert(
                "udp".into(),
                PreparedListener::prepare(Listener::NativeUdp(NativeUdpListener {
                    enabled: true,
                    remark: String::new(),
                    listen: "0.0.0.0".parse().unwrap(),
                    listen_port: 23456.try_into().unwrap(),
                    endpoints: Vec::new(),
                    bind_interface: String::new(),
                    socket: Default::default(),
                }))
                .unwrap(),
            );
        }
        client
            .apply_configuration(
                &ApplyConfigurationPath {
                    network_id: network.network.id,
                },
                &ConfigurationRequest::Document {
                    expected_revision: source.revision,
                    request_id: id(),
                    source_jsonc: WireSecret::new(
                        meshway_configuration::canonical_json(&configuration)
                            .unwrap()
                            .expose()
                            .to_owned(),
                    ),
                },
                user.token.expose(),
            )
            .await
            .unwrap();
        for control in &mut controls {
            control.refresh_configuration().await.unwrap();
        }
        let request = CreateNodeObservation {
            instance: controls[0].execution_snapshot().unwrap().instance(),
            subscription_id: id(),
            target: NodeObservationTarget::Name {
                name: "lease-target".into(),
            },
            service: meshway_configuration::execution::ObservationServiceScope::IcmpEcho,
            selection: ObservationPath::Auto,
            metrics: vec![MetricKind::Rtt, MetricKind::ProbeLoss],
            interval_ms: 30_000,
        };
        let old_status = controls[0].snapshot();
        let response = controls[0].create_node_observation(&request).await.unwrap();
        let lease = controls[0]
            .node_observation_lease(request.subscription_id)
            .unwrap();
        controls[0].refresh_configuration().await.unwrap();
        let snapshot = controls[0].execution_snapshot().unwrap();
        let task = controls[0]
            .get_observation_tasks(&snapshot)
            .await
            .unwrap()
            .items
            .into_iter()
            .find(|task| task.id == response.task.id)
            .unwrap();
        Self {
            app,
            server,
            stop,
            controls,
            request,
            old_status,
            task,
            lease,
            fault: faults,
            _directory: directory,
        }
    }
    pub(in crate::runtime) async fn close(self) {
        self.fault.release.notify_waiters();
        for control in self.controls {
            let _ = control.shutdown().await;
        }
        self.app.stream_shutdown()();
        self.stop.send(()).unwrap();
        self.server.await.unwrap();
        self.app.shutdown().await.unwrap();
    }
    pub(in crate::runtime) fn creates(&self) -> usize {
        self.fault.creates.load(Ordering::Acquire)
    }
    pub(in crate::runtime) fn hold_create(&self) -> Arc<CreateGate> {
        self.fault.create_gate.held.store(true, Ordering::Release);
        self.fault.create_gate.clone()
    }
    pub(in crate::runtime) fn assert_stale_queue(
        &self,
        desired: watch::Receiver<crate::runtime::intent::Desired>,
    ) {
        use crate::runtime::{network, status};
        assert!(!self.old_status.status().authenticated);
        assert!(self.old_status.status().instance.is_none());
        assert!(source_instance(&self.controls[0]).is_ok());
        let mut report = status::Report::initial(desired.borrow().revision);
        report.phase = Phase::Running;
        report.control = Some(self.old_status.clone());
        let mut network = network::Report::stopped();
        network.plane = DataPlane::Ready;
        let (_report, report) = watch::channel(report);
        let (_network, network) = watch::channel(network);
        let view = status::View {
            desired,
            report,
            network,
            server: "http://localhost".into(),
            network_mode: NetworkMode::Tun,
        };
        let mut status = view.status();
        assert_eq!(status.phase, Phase::Synchronizing);
        assert!(status.instance.is_none());
        assert!(status.applied.is_none());
        assert!(can_queue(&status));
        status.data_plane = DataPlane::Failed;
        assert!(!can_queue(&status));
        status.data_plane = DataPlane::Preparing;
        for phase in [
            Phase::Down,
            Phase::Blocked,
            Phase::Stopping,
            Phase::Starting,
        ] {
            status.phase = phase;
            assert!(!can_queue(&status));
        }
        status.phase = Phase::Synchronizing;
        status.desired_up = false;
        assert!(!can_queue(&status));
        status.desired_up = true;
        status.network_mode = NetworkMode::ControlOnly;
        assert_eq!(admission(&status), Err(PingError::Unsupported));
    }
    fn sample(&self) -> PingEvent {
        let now = chrono::Utc::now();
        PingEvent::Sample {
            observation_id: self.task.id,
            sample: ObservationSample {
                measurement_id: id(),
                version: Version::MIN,
                window_start: now - chrono::Duration::milliseconds(1),
                window_end: now,
                partial: false,
                probe: None,
                traffic: None,
                probe_counts: Some(ProbeWindow {
                    sent: 1,
                    replied_within_deadline: 1,
                    timeout: 0,
                    pending: 0,
                }),
            },
        }
    }
}

struct Stream {
    input: mpsc::Sender<PingEvent>,
    events: mpsc::Receiver<PingEvent>,
    leases: watch::Sender<Option<ObservationLease>>,
    _progress: watch::Sender<Progress>,
    _stop: watch::Sender<bool>,
    _desired: watch::Sender<crate::runtime::intent::Desired>,
    cancellation: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl Stream {
    fn open(lease: ObservationLease) -> Self {
        let (client, events) = mpsc::channel(8);
        let (input, incoming) = mpsc::channel(8);
        let (leases, confirmed) = watch::channel(Some(lease));
        let (progress, progress_rx) = watch::channel(Progress::Waiting);
        let (stop, stopped) = watch::channel(false);
        let cancellation = CancellationToken::new();
        let (desired, intent) = test_intent();
        let task = tokio::spawn(forward(
            client,
            incoming,
            cancellation.clone(),
            stopped,
            confirmed,
            progress_rx,
            intent,
        ));
        Self {
            input,
            events,
            leases,
            _progress: progress,
            _stop: stop,
            _desired: desired,
            cancellation,
            task,
        }
    }
    async fn next(&mut self) -> PingEvent {
        tokio::time::timeout(Duration::from_secs(3), self.events.recv())
            .await
            .expect("progress prevents the client's 15 second idle timeout")
            .expect("typed terminal before stream closure")
    }
    async fn wait_until(&mut self, deadline: tokio::time::Instant) {
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                event = self.next() => assert!(matches!(event, PingEvent::Preparing { .. })),
            }
        }
    }
}

#[tokio::test]
async fn blocked_report_and_held_renew_end_at_confirmed_local_lease_with_typed_failure() {
    let mut fixture = Fixture::open().await;
    let original = fixture.lease.expires_at();
    let mut stream = Stream::open(fixture.lease.clone());
    stream.input.send(fixture.sample()).await.unwrap();
    loop {
        if matches!(stream.next().await, PingEvent::Sample { .. }) {
            break;
        }
    }
    let start = tokio::time::Instant::now();
    stream.wait_until(start + Duration::from_secs(10)).await;
    let mut shared_request = fixture.request.clone();
    shared_request.subscription_id = id();
    let shared = fixture.controls[0]
        .create_node_observation(&shared_request)
        .await
        .unwrap();
    assert_eq!(shared.task.id, fixture.task.id);
    let shared_lease = fixture.controls[0]
        .node_observation_lease(shared_request.subscription_id)
        .unwrap();
    fixture.controls[0].refresh_configuration().await.unwrap();
    let snapshot = fixture.controls[0].execution_snapshot().unwrap();
    let task = fixture.controls[0]
        .get_observation_tasks(&snapshot)
        .await
        .unwrap()
        .items
        .remove(0);
    fixture.fault.blocked.store(true, Ordering::Release);
    assert!(
        fixture.controls[0]
            .report_observation_result(
                &snapshot,
                &task,
                &ObservationResult {
                    instance_id: task.instance_id,
                    configuration: task.configuration.clone(),
                    result: ObservationUpdate::Phase {
                        phase: ReportedObservationPhase::Preparing
                    }
                }
            )
            .await
            .is_err()
    );
    stream.wait_until(start + Duration::from_secs(20)).await;
    let renewal = RenewNodeObservation {
        instance: fixture.request.instance.clone(),
        expires_at_ms: u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap() + 30_000,
    };
    {
        let pending =
            fixture.controls[0].renew_node_observation(fixture.request.subscription_id, &renewal);
        tokio::pin!(pending);
        loop {
            tokio::select! {
                result = &mut pending => panic!("renew should still be held at local expiry: {result:?}"),
                event = stream.next() => match event {
                    PingEvent::Preparing { .. } => {},
                    PingEvent::Failed { code: PingError::Timeout } => break,
                    event => panic!("waiting cannot manufacture a sample: {event:?}"),
                }
            }
        }
    }
    fixture.fault.release.notify_waiters();
    fixture.fault.blocked.store(false, Ordering::Release);
    stream.task.await.unwrap();
    assert!(stream.cancellation.is_cancelled());
    assert_eq!(fixture.lease.expires_at(), original);
    assert!(!fixture.lease.is_current());
    assert!(shared_lease.is_current());
    assert!(snapshot.ensure_current().is_ok());
    assert!(
        fixture.controls[0]
            .get_observation_tasks(&snapshot)
            .await
            .unwrap()
            .items
            .iter()
            .any(|task| task.id == fixture.task.id)
    );
    fixture.close().await;
}

#[tokio::test]
async fn thirty_second_interval_remains_live_without_extra_samples_or_idle_timeout() {
    let mut fixture = Fixture::open().await;
    assert_eq!(fixture.task.interval_ms, 30_000);
    let mut stream = Stream::open(fixture.lease.clone());
    stream.input.send(fixture.sample()).await.unwrap();
    loop {
        if matches!(stream.next().await, PingEvent::Sample { .. }) {
            break;
        }
    }
    let start = tokio::time::Instant::now();
    stream.wait_until(start + Duration::from_secs(11)).await;
    let renewal = RenewNodeObservation {
        instance: fixture.request.instance.clone(),
        expires_at_ms: u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap() + 30_000,
    };
    fixture.controls[0]
        .renew_node_observation(fixture.request.subscription_id, &renewal)
        .await
        .unwrap();
    stream.leases.send_replace(Some(
        fixture.controls[0]
            .node_observation_lease(fixture.request.subscription_id)
            .unwrap(),
    ));
    stream.wait_until(start + Duration::from_secs(31)).await;
    stream.input.send(fixture.sample()).await.unwrap();
    loop {
        if matches!(stream.next().await, PingEvent::Sample { .. }) {
            break;
        }
    }
    stream
        .input
        .send(PingEvent::Completed {
            sent: 2,
            success: 2,
            timeout: 0,
            failed: 0,
        })
        .await
        .unwrap();
    loop {
        if matches!(
            stream.next().await,
            PingEvent::Completed {
                sent: 2,
                success: 2,
                ..
            }
        ) {
            break;
        }
    }
    stream.task.await.unwrap();
    fixture.close().await;
}
