pub mod diagnostic;
pub mod dns;
pub mod packet;
pub mod secure;

use meshway_configuration::{AccessRule, MeshRoute, NetworkConfigurationInput};
use meshway_control::Control;
use meshway_control_client::{
    ApplyConfigurationPath, ApproveDeviceAuthorizationPath, Client, GetConfigurationSourcePath,
    GetConfigurationSourceQuery,
};
use meshway_model::{NetworkId, NodeId, RequestId};
use meshway_protocol::{
    ApproveEnrollmentRequest, ConfigurationRequest, CreateNetworkRequest, DemandRequest,
    InstanceRequest, RegisterRequest, WireSecret,
};
use meshway_server::{App, Settings};
use std::{collections::BTreeMap, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle};

pub const WAIT: Duration = Duration::from_secs(1);

pub fn request() -> RequestId {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).unwrap();
    RequestId::new(uuid::Builder::from_random_bytes(bytes).into_uuid()).unwrap()
}

pub struct Server {
    origin: String,
    client: Client,
    app: App,
    temp: tempfile::TempDir,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

pub struct Owner {
    token: WireSecret,
    network: NetworkId,
}

pub struct Node {
    pub id: NodeId,
    pub control: Control,
}

impl Node {
    /// Save the current target, then explicitly request its execution lease.
    /// A query can return an existing grant, but must not invent a missing one.
    pub async fn sync_current(&mut self) -> meshway_authority::ExecutionSnapshot {
        let target = self.control.sync_configuration().await.unwrap().unwrap();
        assert_eq!(self.control.status().cached, Some(target.clone()));
        if self
            .control
            .status()
            .instance
            .unwrap()
            .execution_expires_at_ms
            .is_none()
        {
            assert!(matches!(
                self.control.execution_snapshot(),
                Err(meshway_control::Error::Expired)
            ));
        }
        let lease = self.control.renew_instance().await.unwrap();
        assert_eq!(lease.target, Some(target.clone()));
        let snapshot = self.control.execution_snapshot().unwrap();
        assert_eq!(snapshot.reference(), target);
        snapshot
    }

    pub fn instance(&self) -> InstanceRequest {
        let lease = self.control.status().instance.unwrap();
        InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        }
    }
}

impl Server {
    pub async fn renew_observation_if_due(
        &self,
        owner: &Owner,
        task: &mut meshway_protocol::ObservationTask,
    ) -> bool {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        if now_ms < u128::try_from(task.renew_after.timestamp_millis()).unwrap() {
            return false;
        }
        let subscription_id = task.subscription_id.unwrap();
        let renewed = self
            .client
            .renew_observation(
                &meshway_control_client::RenewObservationPath {
                    network_id: owner.network,
                    subscription_id,
                },
                owner.token.expose(),
            )
            .await
            .unwrap();
        *task = renewed
            .items
            .into_iter()
            .find(|item| item.subscription_id == Some(subscription_id))
            .expect("renewed observer subscription");
        true
    }

    pub async fn observe(
        &self,
        owner: &Owner,
        source: NodeId,
        target: NodeId,
    ) -> meshway_protocol::ObservationTask {
        self.client
            .create_observation(
                &meshway_control_client::CreateObservationPath {
                    network_id: owner.network,
                },
                &meshway_protocol::CreateObservation {
                    subscription_id: meshway_model::ObservationSubscriptionId::new(
                        request().into_uuid(),
                    )
                    .unwrap(),
                    source_node_id: source,
                    target_node_id: target,
                    selection: meshway_protocol::observations::ObservationPath::Auto,
                    service: meshway_configuration::execution::ObservationServiceScope::IcmpEcho,
                    metrics: vec![meshway_model::observations::MetricKind::Rtt],
                    interval_ms: 1000,
                },
                owner.token.expose(),
            )
            .await
            .unwrap()
    }

    pub async fn cancel_observation(
        &self,
        owner: &Owner,
        subscription_id: meshway_model::ObservationSubscriptionId,
    ) {
        self.client
            .cancel_observation(
                &meshway_control_client::CancelObservationPath {
                    network_id: owner.network,
                    subscription_id,
                },
                owner.token.expose(),
            )
            .await
            .unwrap();
    }

    pub async fn start() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let app = App::open(Settings::new(
            temp.path().canonicalize().unwrap().join("server"),
        ))
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = meshway_server::http::router(&app);
        let (stop, done) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = done.await;
                })
                .await
                .unwrap();
        });
        let client = Client::new(
            &origin,
            meshway_control_client::http_client_builder()
                .unwrap()
                .no_proxy()
                .timeout(Duration::from_secs(10)),
            16 * 1024 * 1024,
        )
        .unwrap();
        Self {
            origin,
            client,
            app,
            temp,
            stop,
            task,
        }
    }

    pub async fn owner(&self) -> Owner {
        let session = self
            .client
            .register(&RegisterRequest {
                email: "runtime-owner@example.test".into(),
                password: WireSecret::new("real runtime fixture password".into()),
            })
            .await
            .unwrap();
        let response = self
            .client
            .create_network(
                &CreateNetworkRequest {
                    request_id: request(),
                    name: "Runtime HTTP fixture".into(),
                },
                session.token.expose(),
            )
            .await
            .unwrap();
        Owner {
            token: session.token,
            network: response.network.id,
        }
    }

    pub async fn enroll(&self, owner: &Owner, hostname: &str) -> Node {
        use meshway_model::state::{Capability, StreamKind, TransportRole};
        self.enroll_with_capabilities(
            owner,
            hostname,
            BTreeMap::from([(
                "transport:native_udp".into(),
                Capability::Transport {
                    roles: vec![TransportRole::Dial, TransportRole::Listen],
                    streams: vec![StreamKind::Udp],
                },
            )]),
        )
        .await
    }

    pub async fn enroll_with_capabilities(
        &self,
        owner: &Owner,
        hostname: &str,
        capabilities: BTreeMap<String, meshway_model::state::Capability>,
    ) -> Node {
        let path = self.temp.path().canonicalize().unwrap().join(hostname);
        let mut control = Control::open(
            &path,
            &self.origin,
            meshway_control_client::http_client_builder()
                .unwrap()
                .no_proxy()
                .timeout(Duration::from_secs(10)),
        )
        .unwrap();
        assert!(control.execution_snapshot().is_err());
        let pending = control.begin_enrollment(hostname).await.unwrap();
        self.client
            .approve_device_authorization(
                &ApproveDeviceAuthorizationPath {
                    enrollment_request_id: pending.id,
                },
                &ApproveEnrollmentRequest {
                    network_id: owner.network,
                },
                owner.token.expose(),
            )
            .await
            .unwrap();
        control.poll_enrollment().await.unwrap();
        let enrolled = control.redeem_enrollment().await.unwrap();
        control.authenticate().await.unwrap();
        assert!(control.execution_snapshot().is_err());
        control.acquire_instance(capabilities).await.unwrap();
        assert!(control.execution_snapshot().is_err());
        Node {
            id: enrolled.node_id,
            control,
        }
    }

    pub async fn configure(
        &self,
        owner: &Owner,
        edit: impl FnOnce(&mut NetworkConfigurationInput),
    ) {
        let source = self
            .client
            .get_configuration_source(
                &GetConfigurationSourcePath {
                    network_id: owner.network,
                },
                &GetConfigurationSourceQuery::default(),
                owner.token.expose(),
            )
            .await
            .unwrap();
        let mut input = NetworkConfigurationInput::from(&source.configuration);
        edit(&mut input);
        self.client
            .apply_configuration(
                &ApplyConfigurationPath {
                    network_id: owner.network,
                },
                &ConfigurationRequest::Document {
                    expected_revision: source.revision,
                    request_id: request(),
                    source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
                },
                owner.token.expose(),
            )
            .await
            .unwrap_or_else(|error| {
                if let meshway_control_client::ClientError::Http { problem, .. } = &error {
                    panic!("fixture configuration: {problem:?}");
                }
                panic!("fixture configuration: {error:?}");
            });
    }

    pub async fn close(self) {
        self.app.stream_shutdown()();
        self.stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(10), self.task)
            .await
            .expect("bounded HTTP server shutdown")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), self.app.shutdown())
            .await
            .expect("bounded database owner shutdown")
            .unwrap();
    }
}

pub struct Mesh {
    pub server: Server,
    pub owner: Owner,
    pub a: Node,
    pub r: Node,
    pub b: Node,
    pub demand: RequestId,
    pub path: Vec<NodeId>,
}

impl Mesh {
    pub async fn start() -> Self {
        use meshway_configuration::execution::{BusinessProtocol, BusinessTarget, DemandDecision};
        let server = Server::start().await;
        let owner = server.owner().await;
        let mut a = server.enroll(&owner, "a").await;
        let mut r = server.enroll(&owner, "r").await;
        let mut b = server.enroll(&owner, "b").await;
        server
            .configure(&owner, |input| {
                for node in input.nodes.values_mut() {
                    node.listeners =
                        serde_json::from_str(r#"{"udp":{"type":"native_udp","listenPort":23456}}"#)
                            .unwrap();
                }
                input.access = vec![AccessRule {
                    remark: String::new(),
                    src: vec![format!("node:{}", a.id)],
                    dst: vec![format!("node:{}", b.id)],
                    ip: vec!["udp:42424".into()],
                    bidirectional: false,
                }];
                input.routing.mesh = vec![MeshRoute {
                    remark: String::new(),
                    src: vec![format!("node:{}", a.id)],
                    dst: vec![format!("node:{}", b.id)],
                    via: vec![format!("node:{}", r.id)],
                }];
            })
            .await;
        let demand = request();
        let instance = a.instance();
        let response = a
            .control
            .create_demand(&DemandRequest {
                instance,
                request_id: demand,
                target: BusinessTarget::Mesh { node: b.id },
                protocol: BusinessProtocol::Udp,
                port: Some(42424.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 120,
            })
            .await
            .unwrap();
        assert!(matches!(
            response.decision,
            DemandDecision::Permitted { .. }
        ));
        // A demand acknowledgement is not an execution handoff.
        assert!(a.control.execution_snapshot().is_err());
        let path = vec![a.id, r.id, b.id];
        let source_id = a.id;
        for node in [&mut a, &mut r, &mut b] {
            let guard = node.sync_current().await;
            let configuration = guard.configuration().unwrap();
            let permit = &configuration.permits[&demand];
            assert_eq!(permit.source, source_id);
            assert_eq!(permit.paths.len(), 1);
            assert_eq!(permit.paths[0].nodes, path);
        }
        Self {
            server,
            owner,
            a,
            r,
            b,
            demand,
            path,
        }
    }

    pub async fn close(self) {
        for node in [self.a, self.r, self.b] {
            node.control.shutdown().await.unwrap();
        }
        self.server.close().await;
    }
}
