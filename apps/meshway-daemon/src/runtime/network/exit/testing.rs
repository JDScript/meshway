//! Real Server fixture, with independent enrolled Control owners. No authority
//! implementation or kernel forwarding logic is supplied by this test harness.
use meshway_control::Control;
use meshway_control_client::*;
use meshway_model::{NetworkId, NodeId, RequestId, state::*};
use meshway_protocol::*;
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};

// An isolated assembly test can feed its SecureNode's real projections to the
// existing driver. It does not create a second Host owner or synthesize ACKs.
static DRIVER: std::sync::Mutex<
    Option<tokio::sync::oneshot::Sender<tokio::sync::mpsc::Sender<super::Work>>>,
> = std::sync::Mutex::new(None);

pub(super) fn observe_driver()
-> tokio::sync::oneshot::Receiver<tokio::sync::mpsc::Sender<super::Work>> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    assert!(DRIVER.lock().unwrap().replace(sender).is_none());
    receiver
}

pub(super) fn driver_started(driver: tokio::sync::mpsc::Sender<super::Work>) {
    if let Some(observer) = DRIVER.lock().unwrap().take() {
        let _ = observer.send(driver);
    }
}

pub(in crate::runtime::network) fn id() -> RequestId {
    super::super::packets::request_id().unwrap()
}

pub(in crate::runtime::network) struct Fixture {
    pub controls: Vec<Control>,
    pub directory: tempfile::TempDir,
    pub client: Client,
    pub token: WireSecret,
    pub network: NetworkId,
    pub origin: String,
    pub external: SocketAddr,
    app: meshway_server::App,
    stop: tokio::sync::oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}
impl Fixture {
    pub async fn cli_status(
        &self,
        network: tokio::sync::watch::Receiver<super::super::Report>,
        ready: bool,
    ) {
        use crate::runtime::{
            intent::Desired,
            status::{Report, View},
        };
        use tokio::sync::{mpsc, watch};
        let revision = meshway_model::Version::MIN;
        let (_desired, desired) = watch::channel(Desired {
            revision,
            up: true,
            shutdown: false,
            retry_enrollment: false,
            enrollment_key: None,
            rotation: None,
        });
        let mut report = Report::initial(revision);
        report.control = Some(self.controls[2].snapshot());
        report.phase = meshway_ipc::Phase::ControlReady;
        let (_reported, report) = watch::channel(report);
        let (_events, events) = watch::channel(Default::default());
        let view = View {
            desired,
            report,
            server: self.origin.clone(),
            network_mode: meshway_ipc::NetworkMode::Tun,
            network,
            events,
        };
        assert_eq!(
            view.status()
                .exit
                .as_ref()
                .unwrap()
                .installed_role
                .is_some(),
            ready
        );
        let directory = Arc::new(
            meshway_private_storage::Directory::open(
                &self.directory.path().join("ipc"),
                "test.lock",
            )
            .unwrap(),
        );
        let path = directory.path().join("control.sock");
        let socket = crate::ipc::Socket::bind(directory, None).await.unwrap();
        let (commands, _commands) = mpsc::channel(1);
        let (pings, _pings) = mpsc::channel(1);
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(crate::ipc::serve(socket, commands, pings, view, stopped));
        let executable = std::env::var("MESHWAY_X4_CLI")
            .expect("isolated fixture mounts the current meshway CLI");
        let output = tokio::process::Command::new(executable)
            .arg("--socket")
            .arg(path)
            .arg("status")
            .output()
            .await
            .unwrap();
        stop.send_replace(true);
        task.await.unwrap().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = String::from_utf8(output.stdout).unwrap();
        if ready {
            assert!(output.contains("Exit: Ready"), "{output}");
            assert!(!output.contains("role=none"), "{output}");
            assert!(output.contains("flows=0"), "{output}");
        } else {
            assert!(output.contains("IpForwardDisabled"), "{output}");
            assert!(output.contains("role=none"), "{output}");
            assert!(output.contains("flows=unknown"), "{output}");
        }
        println!("X4_REAL_CLI_STATUS\n{output}");
    }
    pub async fn open() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let app = meshway_server::App::open(meshway_server::Settings::new(
            directory.path().join("server"),
        ))
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
        let user = client
            .register(&RegisterRequest {
                email: "exit-daemon@example.test".into(),
                password: WireSecret::new("exit daemon fixture password".into()),
            })
            .await
            .unwrap();
        let network = client
            .create_network(
                &CreateNetworkRequest {
                    request_id: id(),
                    name: "exit-daemon".into(),
                },
                user.token.expose(),
            )
            .await
            .unwrap()
            .network
            .id;
        let mut controls = Vec::new();
        for name in ["source", "relay", "exit"] {
            let mut control = Control::open(
                &directory.path().join(name),
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
                        network_id: network,
                    },
                    user.token.expose(),
                )
                .await
                .unwrap();
            control.poll_enrollment().await.unwrap();
            control.redeem_enrollment().await.unwrap();
            control.authenticate().await.unwrap();
            control
                .acquire_instance(super::super::capabilities())
                .await
                .unwrap();
            controls.push(control);
        }
        let external = std::env::var("MESHWAY_X4_EXTERNAL")
            .unwrap_or_else(|_| "198.18.71.11:18080".into())
            .parse()
            .unwrap();
        let mut fixture = Self {
            controls,
            directory,
            client,
            token: user.token,
            network,
            origin,
            external,
            app,
            stop,
            server,
        };
        fixture.configure(true).await;
        fixture.refresh().await;
        fixture
    }
    pub fn nodes(&self) -> Vec<NodeId> {
        self.controls
            .iter()
            .map(|control| control.status().binding.unwrap().node_id)
            .collect()
    }
    pub async fn configure(&self, enabled: bool) {
        use meshway_configuration::*;
        let source = self
            .client
            .get_configuration_source(
                &GetConfigurationSourcePath {
                    network_id: self.network,
                },
                &GetConfigurationSourceQuery::default(),
                self.token.expose(),
            )
            .await
            .unwrap();
        let mut input = NetworkConfigurationInput::from(&source.configuration);
        let nodes = self.nodes();
        input.nodes.get_mut(&nodes[2]).unwrap().exit_enabled = enabled;
        for node in input.nodes.values_mut() {
            node.listeners = serde_json::from_value(
                serde_json::json!({"udp":{"type":"native_udp","listenPort":23456}}),
            )
            .unwrap();
        }
        input.dns.magic_dns = false;
        input.dns.override_local = false;
        input.dns.servers.clear();
        input.dns.default.clear();
        input.dns.routes.clear();
        input.dns.search_domains.clear();
        input.access = vec![AccessRule {
            remark: String::new(),
            src: vec![format!("node:{}", nodes[0])],
            dst: vec![format!("{}/32", self.external.ip())],
            ip: vec![format!("udp:{}", self.external.port())],
            bidirectional: false,
        }];
        input.routing.egress.rules.clear();
        let rules = vec![EgressRule {
            remark: String::new(),
            matcher: RouteMatch {
                ip_cidr: vec![format!("{}/32", self.external.ip())],
                ..Default::default()
            },
            action: RuleAction::Exit {
                nodes: vec![format!("node:{}", nodes[2])],
                via: vec![format!("node:{}", nodes[1])],
            },
        }];
        input.routing.egress.profiles.insert(
            "exit-test".into(),
            RoutingProfile {
                rules,
                fallback: RouteAction::Unmanaged,
            },
        );
        input.routing.egress.assignments = vec![ProfileAssignment {
            target: vec![format!("node:{}", nodes[0])],
            allowed: vec!["exit-test".into(), "mesh".into()],
            default: "exit-test".into(),
            locked: false,
        }];
        self.client
            .apply_configuration(
                &ApplyConfigurationPath {
                    network_id: self.network,
                },
                &ConfigurationRequest::Document {
                    expected_revision: source.revision,
                    request_id: id(),
                    source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
                },
                self.token.expose(),
            )
            .await
            .unwrap_or_else(|error| match error {
                meshway_control_client::ClientError::Http { status, problem } => {
                    panic!("fixture configuration {status}: {problem:?}")
                }
                error => panic!("fixture configuration: {error:?}"),
            });
    }
    pub async fn refresh(&mut self) {
        for control in &mut self.controls {
            control.refresh_configuration().await.unwrap();
            control.acknowledge_saved().await.unwrap();
        }
    }
    pub fn host_directory(&self) -> Arc<meshway_private_storage::Directory> {
        Arc::new(
            meshway_private_storage::Directory::open(
                &self.directory.path().join("host"),
                "node.lock",
            )
            .unwrap(),
        )
    }
    pub fn options() -> meshway_underlay::Options {
        meshway_underlay::Options {
            bootstrap_nameservers: Some(vec!["192.0.2.53:53".parse().unwrap()]),
            ..Default::default()
        }
    }
    pub async fn capability(&mut self, capability: meshway_model::exit::ExitCapability) {
        let state = NodeState {
            system: SystemState {
                hostname: "exit-fixture".into(),
                os: "linux".into(),
                arch: std::env::consts::ARCH.into(),
                agent_version: "test".into(),
            },
            capabilities: super::super::capabilities(),
            configuration: ConfigurationState::default(),
            exit_capability: Some(capability),
            listeners: BTreeMap::new(),
            sockets: BTreeMap::new(),
            links: BTreeMap::new(),
            paths: BTreeMap::new(),
            selection: Default::default(),
            probes: Vec::new(),
            problems: Vec::new(),
        };
        self.controls[2]
            .report_runtime_state(state, chrono::Utc::now())
            .await
            .unwrap();
    }
    pub async fn demand(&mut self) -> RequestId {
        use meshway_configuration::execution::*;
        let request_id = id();
        let instance = self.controls[0].execution_snapshot().unwrap().instance();
        let result = self.controls[0]
            .create_demand(&DemandRequest {
                instance,
                request_id,
                target: BusinessTarget::Ip {
                    address: ExecutionAddress(self.external.ip()),
                },
                protocol: BusinessProtocol::Udp,
                port: Some(self.external.port().try_into().unwrap()),
                profile: "exit-test".into(),
                duration_seconds: 30,
            })
            .await
            .unwrap();
        assert!(
            matches!(result.decision, DemandDecision::Permitted { .. }),
            "{:?}",
            result.decision
        );
        self.refresh().await;
        request_id
    }
    pub async fn close(self) {
        for control in self.controls {
            control.shutdown().await.unwrap();
        }
        self.app.stream_shutdown()();
        let _ = self.stop.send(());
        self.server.await.unwrap();
        self.app.shutdown().await.unwrap();
    }
}
