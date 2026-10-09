//! Real held-key Control/Server authority; explicitly supplied transport facts
//! below test measurement ordering, not packet transport or OS installation.
use super::*;
use meshway_control::Control;
use meshway_control_client::*;
use meshway_model::{
    state::{Capability, ProbeOutcome, StreamKind, TransportRole},
    *,
};
use meshway_protocol::*;
mod preparing;

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

struct Fixture {
    app: meshway_server::App,
    server: tokio::task::JoinHandle<()>,
    stop: tokio::sync::oneshot::Sender<()>,
    controls: Vec<Control>,
    owner: Owner,
    subscription: ObservationSubscriptionId,
    task: ObservationTask,
    created_at: Instant,
    accepted_at: Instant,
    installed_at: Instant,
    _directory: tempfile::TempDir,
}
impl Fixture {
    async fn open() -> Self {
        Self::open_route(false).await
    }
    async fn open_route(relay: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let app = meshway_server::App::open(meshway_server::Settings::new(root.join("server")))
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = meshway_server::http::router(&app);
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
            meshway_control_client::http_client_builder()
                .unwrap()
                .no_proxy(),
            16 * 1024 * 1024,
        )
        .unwrap();
        let session = client
            .register(&RegisterRequest {
                email: "diagnostics@example.test".into(),
                password: WireSecret::new("diagnostics held-key fixture password".into()),
            })
            .await
            .unwrap();
        let network = client
            .create_network(
                &CreateNetworkRequest {
                    request_id: id(),
                    name: "diagnostics".into(),
                },
                session.token.expose(),
            )
            .await
            .unwrap();
        let mut controls = Vec::new();
        let names = if relay {
            vec!["probe-source", "probe-target", "probe-relay"]
        } else {
            vec!["probe-source", "probe-target"]
        };
        for name in names {
            let mut control = Control::open(
                &root.join(name),
                &origin,
                meshway_control_client::http_client_builder()
                    .unwrap()
                    .no_proxy(),
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
                    session.token.expose(),
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
                session.token.expose(),
            )
            .await
            .unwrap();
        let mut cfg = source.configuration;
        let a = controls[0].status().binding.unwrap().node_id;
        let b = controls[1].status().binding.unwrap().node_id;
        cfg.access = vec![meshway_configuration::AccessRule {
            remark: String::new(),
            src: vec![format!("node:{a}")],
            dst: vec![format!("node:{b}")],
            ip: vec!["icmp:*".into()],
            bidirectional: false,
        }];
        if relay {
            cfg.routing.mesh = vec![meshway_configuration::MeshRoute {
                remark: String::new(),
                src: vec![format!("node:{a}")],
                dst: vec![format!("node:{b}")],
                via: vec![format!(
                    "node:{}",
                    controls[2].status().binding.unwrap().node_id
                )],
            }];
        }
        for node in cfg.nodes.values_mut() {
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
                        meshway_configuration::canonical_json(&cfg)
                            .unwrap()
                            .expose()
                            .to_owned(),
                    ),
                },
                session.token.expose(),
            )
            .await
            .unwrap();
        for control in &mut controls {
            control.refresh_configuration().await.unwrap();
        }
        let subscription = id();
        let instance = controls[0].execution_snapshot().unwrap().instance();
        let created_at = Instant::now();
        let first = controls[0]
            .create_node_observation(&CreateNodeObservation {
                instance,
                subscription_id: subscription,
                target: NodeObservationTarget::Name {
                    name: "probe-target".into(),
                },
                service: meshway_configuration::execution::ObservationServiceScope::IcmpEcho,
                selection: ObservationPath::Auto,
                metrics: vec![MetricKind::Rtt, MetricKind::ProbeLoss],
                interval_ms: 1000,
            })
            .await
            .unwrap();
        let accepted_at = Instant::now();
        for control in &mut controls {
            control.refresh_configuration().await.unwrap();
        }
        let snapshot = controls[0].execution_snapshot().unwrap();
        let task = controls[0]
            .get_observation_tasks(&snapshot)
            .await
            .unwrap()
            .items
            .into_iter()
            .find(|task| task.id == first.task.id)
            .unwrap();
        let mut owner = Owner::default();
        owner
            .synchronize(snapshot, std::slice::from_ref(&task))
            .unwrap();
        let installed_at = Instant::now();
        Self {
            app,
            server,
            stop,
            controls,
            owner,
            subscription,
            task,
            created_at,
            accepted_at,
            installed_at,
            _directory: directory,
        }
    }
    fn measurement(&mut self, start: Instant) -> (EventId, Digest) {
        let task = self.owner.tasks.get_mut(&self.task.id).unwrap();
        task.circuit = Some([7; 32]);
        let measurement = Measurement::new(task, start).unwrap();
        let result = (measurement.id, measurement.challenge);
        task.measurement = Some(measurement);
        result
    }
    fn channel(&self) -> ProbeChannel {
        ProbeChannel {
            link_id: [1; 32].into(),
            socket_id: [2; 32].into(),
            peer_node_id: self.task.target_node_id,
            transport: meshway_configuration::TransportProtocol::NativeUdp,
            local_address: "192.0.2.1:1000".parse().unwrap(),
            remote_address: "192.0.2.2:2000".parse().unwrap(),
            acquisition: meshway_model::observations::LinkAcquisition::PublishedEndpoint,
        }
    }
    fn sample(&self) -> &meshway_model::observations::ObservationSample {
        let output = self.owner.outputs().next().unwrap();
        match &output.result.result {
            ObservationUpdate::Sample { sample, .. } => sample,
            _ => panic!("expected sample"),
        }
    }
    async fn close(self) {
        for control in self.controls {
            let _ = control.shutdown().await;
        }
        self.app.stream_shutdown()();
        self.stop.send(()).unwrap();
        self.server.await.unwrap();
        self.app.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn acquisition_wait_never_creates_a_measurement_and_has_a_separate_setup_limit() {
    let mut fixture = Fixture::open().await;
    let snapshot = fixture.controls[0].execution_snapshot().unwrap();
    let identity = fixture.controls[0]
        .dataplane_identity(snapshot.clone())
        .unwrap();
    let mut secure = SecureNode::new(identity, Default::default(), BTreeMap::new()).unwrap();
    let task = fixture.owner.tasks.get_mut(&fixture.task.id).unwrap();
    task.preparing = Instant::now() - DEADLINE - Duration::from_secs(2);
    task.next = Instant::now();
    assert!(
        fixture
            .owner
            .actions(&mut secure, &BTreeMap::new(), MAX_TASKS)
            .is_empty()
    );
    assert_eq!(fixture.owner.outputs().count(), 0);
    assert!(fixture.owner.tasks[&fixture.task.id].measurement.is_none());

    let before = fixture.owner.tasks[&fixture.task.id].preparing;
    fixture.owner.suspend();
    fixture
        .owner
        .synchronize(snapshot.clone(), std::slice::from_ref(&fixture.task))
        .unwrap();
    assert_eq!(
        fixture.owner.tasks[&fixture.task.id].preparing, before,
        "renew/reinstall cannot restart an unlimited preparation budget"
    );
    let task = fixture.owner.tasks.get_mut(&fixture.task.id).unwrap();
    task.preparing = Instant::now() - SETUP_TIMEOUT;
    task.next = Instant::now();
    assert!(
        fixture
            .owner
            .actions(&mut secure, &BTreeMap::new(), MAX_TASKS)
            .is_empty()
    );
    let output = fixture.owner.outputs().next().unwrap();
    assert!(matches!(
        output.result.result,
        ObservationUpdate::Phase {
            phase: ReportedObservationPhase::Failed
        }
    ));
    assert!(fixture.owner.tasks[&fixture.task.id].measurement.is_none());
    assert!(snapshot.ensure_current().is_ok());
    fixture.owner.acknowledge(fixture.task.id);
    assert!(
        fixture
            .owner
            .actions(&mut secure, &BTreeMap::new(), MAX_TASKS)
            .is_empty()
    );
    assert_eq!(fixture.owner.outputs().count(), 0);
    fixture.close().await;
}

#[tokio::test]
async fn reply_before_completion_uses_captured_transport_time_and_final_window_once() {
    let mut fixture = Fixture::open().await;
    let start = Instant::now();
    let (mid, challenge) = fixture.measurement(start);
    let channel = fixture.channel();
    tokio::time::sleep(Duration::from_millis(10)).await;
    fixture.owner.reply(
        [7; 32],
        mid,
        challenge,
        channel.clone(),
        start + Duration::from_millis(4),
    );
    assert_eq!(fixture.owner.outputs().count(), 0);
    let instance = fixture.controls[0].execution_snapshot().unwrap().instance();
    let lease_deadline = fixture.controls[0]
        .node_observation_lease(fixture.subscription)
        .unwrap()
        .expires_at();
    fixture.controls[0]
        .get_node_observation(
            fixture.subscription,
            &NodeObservationQuery {
                instance_id: instance.instance_id,
                generation: instance.generation,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        fixture.controls[0]
            .node_observation_lease(fixture.subscription)
            .unwrap()
            .expires_at(),
        lease_deadline,
        "replaying a subscription view cannot extend its first monotonic deadline"
    );
    fixture.owner.sent(
        fixture.task.id,
        mid,
        Ok((channel.clone(), start + Duration::from_millis(1))),
    );
    let sample = fixture.sample().clone();
    assert_eq!(
        sample.probe.as_ref().unwrap().duration_us.unwrap().get(),
        3000
    );
    assert_eq!(sample.probe_counts.as_ref().unwrap().sent, 1);
    assert!(!sample.partial);
    fixture.owner.reply(
        [7; 32],
        mid,
        challenge,
        channel.clone(),
        start + Duration::from_millis(4),
    );
    fixture
        .owner
        .sent(fixture.task.id, mid, Ok((channel, start)));
    assert_eq!(*fixture.sample(), sample);
    assert_eq!(fixture.owner.outputs().count(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn local_failure_and_late_reply_do_not_fabricate_loss_or_zero_rtt() {
    let mut fixture = Fixture::open().await;
    let start = Instant::now();
    let (mid, challenge) = fixture.measurement(start);
    fixture.owner.sent(fixture.task.id, mid, Err(()));
    fixture.owner.reply(
        [7; 32],
        mid,
        challenge,
        fixture.channel(),
        start + Duration::from_millis(1),
    );
    let sample = fixture.sample();
    assert_eq!(sample.probe.as_ref().unwrap().outcome, ProbeOutcome::Failed);
    assert!(sample.probe.as_ref().unwrap().duration_us.is_none());
    assert!(sample.probe.as_ref().unwrap().channels.is_none());
    assert_eq!(sample.probe_counts.as_ref().unwrap().sent, 0);
    assert_eq!(sample.probe_counts.as_ref().unwrap().timeout, 0);
    fixture.close().await;
}

#[tokio::test]
async fn task_binding_shared_consumers_pause_and_scope_revocation_are_exact() {
    let mut fixture = Fixture::open().await;
    let snapshot = fixture.controls[0].execution_snapshot().unwrap();
    let mut wrong = fixture.task.clone();
    wrong.instance_id = id();
    assert!(
        fixture
            .owner
            .synchronize(snapshot.clone(), &[wrong])
            .is_err()
    );
    let peer = fixture.controls[1].execution_snapshot().unwrap();
    assert!(
        fixture
            .owner
            .synchronize(peer, std::slice::from_ref(&fixture.task))
            .is_err()
    );
    assert!(matches!(
        fixture
            .owner
            .synchronize(snapshot.clone(), &vec![fixture.task.clone(); MAX_TASKS + 1]),
        Err(Error::Capacity)
    ));
    assert_eq!(fixture.owner.tasks.len(), 1);
    let identity = fixture.controls[0]
        .dataplane_identity(snapshot.clone())
        .unwrap();
    let mut secure = SecureNode::new(identity, Default::default(), BTreeMap::new()).unwrap();
    assert!(
        fixture
            .owner
            .actions(&mut secure, &BTreeMap::new(), 0)
            .is_empty()
    );
    assert!(fixture.owner.tasks[&fixture.task.id].measurement.is_none());
    let instance = snapshot.instance();
    let shared = fixture.controls[0]
        .create_node_observation(&CreateNodeObservation {
            instance: instance.clone(),
            subscription_id: id(),
            target: NodeObservationTarget::Name {
                name: "probe-target".into(),
            },
            service: fixture.task.service.clone(),
            selection: fixture.task.selection.clone(),
            metrics: fixture.task.metrics.clone(),
            interval_ms: 1000,
        })
        .await
        .unwrap();
    assert_eq!(shared.task.id, fixture.task.id);
    fixture
        .owner
        .synchronize(snapshot.clone(), std::slice::from_ref(&fixture.task))
        .unwrap();
    assert_eq!(fixture.owner.tasks.len(), 1);
    let (mid, challenge) = fixture.measurement(Instant::now());
    fixture.owner.set_paused(&[fixture.task.id].into());
    fixture
        .owner
        .reply([7; 32], mid, challenge, fixture.channel(), Instant::now());
    fixture.owner.sent(
        fixture.task.id,
        mid,
        Ok((fixture.channel(), Instant::now())),
    );
    assert_eq!(fixture.owner.outputs().count(), 0);
    let cancelled = fixture.controls[0]
        .cancel_node_observation(fixture.subscription, &instance)
        .await
        .unwrap();
    let surviving = cancelled.task.unwrap();
    assert_eq!(surviving.id, fixture.task.id);
    assert_ne!(surviving.configuration, snapshot.reference());
    assert!(matches!(
        fixture.controls[0]
            .report_observation_result(
                &snapshot,
                &fixture.task,
                &ObservationResult {
                    instance_id: instance.instance_id,
                    configuration: snapshot.reference(),
                    result: ObservationUpdate::Phase {
                        phase: ReportedObservationPhase::Preparing
                    },
                }
            )
            .await,
        Err(meshway_control::Error::StateScopeChanged)
    ));
    assert!(
        snapshot.ensure_current().is_ok(),
        "observation cancellation does not fence business Control"
    );
    fixture.owner.set_paused(&Default::default());
    fixture.controls[0].stop().await.unwrap();
    assert_eq!(fixture.owner.outputs().count(), 0);
    assert!(
        fixture
            .owner
            .synchronize(snapshot, std::slice::from_ref(&fixture.task))
            .is_err()
    );
    fixture.close().await;
}

#[tokio::test]
async fn timed_out_admitted_send_has_one_loss_and_a_late_reply_cannot_revise_it() {
    // C: the admitted probe's actual DEADLINE must elapse before its late
    // receipt/reply. No setup or HTTP work occurs inside that deadline.
    let mut fixture = Fixture::open().await;
    let start = Instant::now();
    let (mid, challenge) = fixture.measurement(start);
    tokio::time::sleep_until((start + DEADLINE).into()).await;
    fixture
        .owner
        .reply([7; 32], mid, challenge, fixture.channel(), Instant::now());
    fixture.owner.sent(
        fixture.task.id,
        mid,
        Ok((fixture.channel(), start + Duration::from_millis(1))),
    );
    let first = fixture.sample().clone();
    assert_eq!(first.probe.as_ref().unwrap().outcome, ProbeOutcome::Timeout);
    assert_eq!(first.probe_counts.as_ref().unwrap().sent, 1);
    assert_eq!(first.probe_counts.as_ref().unwrap().timeout, 1);
    assert_eq!(first.probe_counts.as_ref().unwrap().pending, 0);
    assert!(first.probe.as_ref().unwrap().duration_us.is_none());
    fixture
        .owner
        .reply([7; 32], mid, challenge, fixture.channel(), Instant::now());
    assert_eq!(*fixture.sample(), first);
    fixture.close().await;
}

#[tokio::test]
async fn local_target_denial_and_wrong_instance_do_not_fence_business_control() {
    let mut fixture = Fixture::open().await;
    let snapshot = fixture.controls[0].execution_snapshot().unwrap();
    let mut request = CreateNodeObservation {
        instance: snapshot.instance(),
        subscription_id: id(),
        target: NodeObservationTarget::Name {
            name: "missing-node".into(),
        },
        service: fixture.task.service.clone(),
        selection: ObservationPath::Auto,
        metrics: fixture.task.metrics.clone(),
        interval_ms: 1000,
    };
    assert!(
        fixture.controls[0]
            .create_node_observation(&request)
            .await
            .is_err()
    );
    assert!(fixture.controls[0].status().authenticated);
    assert!(snapshot.ensure_current().is_ok());
    request.instance.instance_id = id();
    assert!(matches!(
        fixture.controls[0].create_node_observation(&request).await,
        Err(meshway_control::Error::Lifecycle)
    ));
    assert!(snapshot.ensure_current().is_ok());
    fixture.close().await;
}

#[tokio::test]
async fn preparing_high_water_replaces_same_snapshot_segment_and_cancels_old_publication() {
    let mut fixture = Fixture::open().await;
    let snapshot = fixture.controls[0].execution_snapshot().unwrap();
    let start = Instant::now();
    let (mid, challenge) = fixture.measurement(start);
    tokio::time::sleep(Duration::from_millis(10)).await;
    fixture.owner.reply(
        [7; 32],
        mid,
        challenge,
        fixture.channel(),
        start + Duration::from_millis(4),
    );
    fixture.owner.sent(
        fixture.task.id,
        mid,
        Ok((fixture.channel(), start + Duration::from_millis(1))),
    );
    let output = fixture.owner.outputs().next().unwrap().clone();
    let end = fixture.sample().window_end;
    let path = fixture.owner.tasks[&fixture.task.id].path.clone();
    let mut running = fixture.task.clone();
    running.phase = ObservationPhase::Running;
    running.path = Some(path);
    running.path_segment = Some(Sequence::new(1).unwrap());
    running.latest = Some(fixture.sample().clone());
    fixture
        .owner
        .synchronize(snapshot.clone(), &[running.clone()])
        .unwrap();
    assert!(output.is_current());

    let mut preparing = running;
    preparing.phase = ObservationPhase::Preparing;
    preparing.path = None;
    preparing.latest = None;
    fixture
        .owner
        .synchronize(snapshot.clone(), &[preparing.clone()])
        .unwrap();
    assert!(snapshot.ensure_current().is_ok());
    assert!(!output.is_current());
    assert_eq!(fixture.owner.tasks[&fixture.task.id].segment.get(), 2);
    assert_eq!(fixture.owner.tasks[&fixture.task.id].last_end, Some(end));
    assert_eq!(fixture.owner.outputs().count(), 0);
    fixture.owner.synchronize(snapshot, &[preparing]).unwrap();
    assert_eq!(
        fixture.owner.tasks[&fixture.task.id].segment.get(),
        2,
        "reading the same preparing state does not allocate another segment"
    );
    fixture.close().await;
}

#[tokio::test]
async fn shared_cancel_keeps_a_finished_window_backpressured_until_acknowledged() {
    let mut fixture = Fixture::open().await;
    let start = Instant::now();
    let (mid, _) = fixture.measurement(start);
    fixture.owner.sent(fixture.task.id, mid, Err(()));
    let output = fixture.owner.outputs().next().unwrap().clone();
    fixture.owner.set_paused(&[fixture.task.id].into());
    assert!(output.is_current());
    assert_eq!(fixture.owner.outputs().count(), 0);
    fixture.owner.set_paused(&Default::default());
    assert_eq!(fixture.sample().measurement_id, mid);
    fixture.owner.acknowledge(fixture.task.id);
    assert_eq!(fixture.owner.outputs().count(), 0);
    fixture.close().await;
}
