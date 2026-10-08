//! Real Server/Control identities and publication, with only raw I/O injected.
use meshway_authority::DataPlaneIdentity;
use meshway_configuration::{AccessRule, Input, NetworkConfigurationInput};
use meshway_control::Control;
use meshway_control_client::{
    ApplyConfigurationPath, ApproveDeviceAuthorizationPath, Client, GetConfigurationSourcePath,
    GetConfigurationSourceQuery,
};
use meshway_model::RequestId;
use meshway_protocol::{
    ApproveEnrollmentRequest, ConfigurationRequest, CreateNetworkRequest, DemandRequest,
    InstanceRequest, RegisterRequest, WireSecret,
};
use meshway_server::{App, Settings};
use meshway_transport_spec::{
    DialerSet, Endpoint, Listener, NativeTcpDialer, NativeTcpListener, NativeUdpDialer,
    NativeUdpListener,
};
use std::{collections::BTreeMap, net::SocketAddr, time::Duration};
use tokio::{
    net::{TcpListener, UdpSocket},
    sync::oneshot,
    task::JoinHandle,
};

pub(super) fn request() -> RequestId {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).unwrap();
    RequestId::new(uuid::Builder::from_random_bytes(bytes).into_uuid()).unwrap()
}

pub(in crate::manager) struct Fixture {
    app: App,
    stop: oneshot::Sender<()>,
    server: JoinHandle<()>,
    _temp: tempfile::TempDir,
    pub controls: Vec<Control>,
    pub identities: Vec<DataPlaneIdentity>,
    pub demands: [RequestId; 2],
    pub udp: Option<UdpSocket>,
    pub reverse_udp: Option<UdpSocket>,
    pub extra_udp: Vec<UdpSocket>,
    pub tcp: Option<TcpListener>,
    pub extra_tcp: Vec<TcpListener>,
    client: Client,
    user_token: WireSecret,
    network: meshway_model::NetworkId,
    access: Vec<AccessRule>,
}

impl Fixture {
    pub async fn new(stream: bool, ipv6: bool) -> Self {
        Self::builder().stream(stream).ipv6(ipv6).build().await
    }

    pub fn builder() -> FixtureBuilder {
        FixtureBuilder::default()
    }

    pub fn manager(
        &self,
        index: usize,
        factory: std::sync::Arc<dyn super::SocketFactory>,
    ) -> ManagerBuilder {
        ManagerBuilder {
            identity: self.identities[index].clone(),
            factory,
            limits: super::Limits::default(),
            traversal: None,
        }
    }

    async fn build(options: FixtureBuilder) -> Self {
        let FixtureBuilder {
            stream,
            ipv6,
            reverse,
            multiple,
            stun,
            indirect_first,
        } = options;
        use meshway_configuration::execution::{BusinessProtocol, BusinessTarget, DemandDecision};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut settings = Settings::new(root.join("server"));
        if let Some(stun) = stun {
            settings.stun_servers.insert(
                "test-observer".into(),
                Endpoint {
                    host: stun.ip().to_string(),
                    port: stun.port().try_into().unwrap(),
                },
            );
        }
        let app = App::open(settings).await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = meshway_server::http::router(&app);
        let (stop, done) = oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = done.await;
                })
                .await
                .unwrap();
        });
        let builder = || {
            // Loopback HTTP does not use TLS or require the container's CA bundle.
            let tls = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
            reqwest::Client::builder()
                .tls_backend_preconfigured(tls)
                .no_proxy()
                .timeout(Duration::from_secs(10))
        };
        let client = Client::new(&origin, builder(), 16 * 1024 * 1024).unwrap();
        let user = client
            .register(&RegisterRequest {
                email: "connectivity@example.test".into(),
                password: WireSecret::new("real connectivity test password".into()),
            })
            .await
            .unwrap();
        let network = client
            .create_network(
                &CreateNetworkRequest {
                    request_id: request(),
                    name: "Datagram owner".into(),
                },
                user.token.expose(),
            )
            .await
            .unwrap()
            .network
            .id;
        let mut controls = Vec::new();
        let mut ids = Vec::new();
        for name in ["a", "b", "c"] {
            let mut control = Control::open(&root.join(name), &origin, builder()).unwrap();
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
            ids.push(control.redeem_enrollment().await.unwrap().node_id);
            control.authenticate().await.unwrap();
            control
                .acquire_instance(crate::capabilities())
                .await
                .unwrap();
            controls.push(control);
        }
        let local: SocketAddr = if ipv6 { "[::1]:0" } else { "127.0.0.1:0" }
            .parse()
            .unwrap();
        let udp = UdpSocket::bind(local).await.unwrap();
        let reverse_udp = if reverse {
            Some(UdpSocket::bind(local).await.unwrap())
        } else {
            None
        };
        let tcp = if stream {
            Some(TcpListener::bind(local).await.unwrap())
        } else {
            None
        };
        let udp_address = udp.local_addr().unwrap();
        let source = client
            .get_configuration_source(
                &GetConfigurationSourcePath {
                    network_id: network,
                },
                &GetConfigurationSourceQuery::default(),
                user.token.expose(),
            )
            .await
            .unwrap();
        let mut cfg = NetworkConfigurationInput::from(&source.configuration);
        for node in cfg.nodes.values_mut() {
            node.listeners.clear();
        }
        cfg.nodes
            .get_mut(&ids[1])
            .unwrap()
            .listeners
            .insert("udp".into(), udp_listener(udp_address));
        let mut extra_udp = Vec::new();
        if multiple {
            let socket = UdpSocket::bind(local).await.unwrap();
            let address = socket.local_addr().unwrap();
            cfg.nodes
                .get_mut(&ids[1])
                .unwrap()
                .listeners
                .insert("udp2".into(), udp_listener(address));
            extra_udp.push(socket);
        }
        if let Some(socket) = &reverse_udp {
            let address = socket.local_addr().unwrap();
            cfg.nodes
                .get_mut(&ids[0])
                .unwrap()
                .listeners
                .insert("udp".into(), udp_listener(address));
        }
        if let Some(tcp) = &tcp {
            let address = tcp.local_addr().unwrap();
            cfg.nodes.get_mut(&ids[1]).unwrap().listeners.insert(
                "tcp".into(),
                Listener::NativeTcp(NativeTcpListener {
                    listen: address.ip(),
                    listen_port: address.port().try_into().unwrap(),
                    endpoints: vec![Endpoint {
                        host: address.ip().to_string(),
                        port: address.port().try_into().unwrap(),
                    }],
                    enabled: true,
                    remark: String::new(),
                    bind_interface: String::new(),
                    socket: Default::default(),
                    tcp: Default::default(),
                }),
            );
        }
        let udp_dialer = NativeUdpDialer {
            connect_timeout: "2s".parse().unwrap(),
            ..Default::default()
        };
        cfg.defaults.dialers = DialerSet {
            native_udp: Some(udp_dialer),
            native_tcp: stream.then(NativeTcpDialer::default),
            vless: None,
            trojan: None,
        };
        for node in cfg.nodes.values_mut() {
            node.dialers = Input::Omitted;
        }
        cfg.access = vec![AccessRule {
            remark: String::new(),
            src: vec![format!("node:{}", ids[0]), format!("node:{}", ids[2])],
            dst: vec![format!("node:{}", ids[1])],
            ip: vec!["udp:42424".into()],
            bidirectional: false,
        }];
        if stun.is_some() {
            for node in cfg.nodes.values_mut() {
                for listener in node.listeners.values_mut() {
                    if let Listener::NativeUdp(listener) = listener {
                        listener.endpoints.clear();
                    }
                }
            }
        }
        client
            .apply_configuration(
                &ApplyConfigurationPath {
                    network_id: network,
                },
                &ConfigurationRequest::Document {
                    expected_revision: source.revision,
                    request_id: request(),
                    source_jsonc: WireSecret::new(serde_json::to_string(&cfg).unwrap()),
                },
                user.token.expose(),
            )
            .await
            .unwrap();
        let mut demands = [request(), request()];
        if let Some(indirect_first) = indirect_first {
            demands.sort();
            if indirect_first {
                demands.swap(0, 1);
            }
        }
        for (index, demand) in [(0, demands[0]), (2, demands[1])] {
            let lease = controls[index].status().instance.unwrap();
            let result = controls[index]
                .create_demand(&DemandRequest {
                    instance: InstanceRequest {
                        instance_id: lease.instance_id,
                        generation: lease.generation,
                    },
                    request_id: demand,
                    target: BusinessTarget::Mesh { node: ids[1] },
                    protocol: BusinessProtocol::Udp,
                    port: Some(42424.try_into().unwrap()),
                    profile: "mesh".into(),
                    duration_seconds: 120,
                })
                .await
                .unwrap();
            assert!(matches!(result.decision, DemandDecision::Permitted { .. }));
        }
        let mut identities = Vec::new();
        for control in &mut controls {
            control.sync_configuration().await.unwrap().unwrap();
            control.renew_instance().await.unwrap();
            identities.push(
                control
                    .dataplane_identity(control.execution_snapshot().unwrap())
                    .unwrap(),
            );
        }
        Self {
            app,
            stop,
            server,
            _temp: temp,
            controls,
            identities,
            demands,
            udp: Some(udp),
            reverse_udp,
            extra_udp,
            tcp,
            extra_tcp: Vec::new(),
            client,
            user_token: user.token,
            network,
            access: cfg.access,
        }
    }

    pub fn ids(&self) -> Vec<meshway_model::NodeId> {
        self.identities
            .iter()
            .map(|identity| identity.snapshot().configuration().unwrap().node_id)
            .collect()
    }

    pub async fn configure(&self, change: impl FnOnce(&mut NetworkConfigurationInput)) {
        let source = self
            .client
            .get_configuration_source(
                &GetConfigurationSourcePath {
                    network_id: self.network,
                },
                &GetConfigurationSourceQuery::default(),
                self.user_token.expose(),
            )
            .await
            .unwrap();
        let mut cfg = NetworkConfigurationInput::from(&source.configuration);
        change(&mut cfg);
        self.client
            .apply_configuration(
                &ApplyConfigurationPath {
                    network_id: self.network,
                },
                &ConfigurationRequest::Document {
                    expected_revision: source.revision,
                    request_id: request(),
                    source_jsonc: WireSecret::new(serde_json::to_string(&cfg).unwrap()),
                },
                self.user_token.expose(),
            )
            .await
            .unwrap();
    }

    pub async fn refresh(&mut self, index: usize) -> DataPlaneIdentity {
        let control = &mut self.controls[index];
        control.refresh_configuration().await.unwrap();
        let identity = control
            .dataplane_identity(control.execution_snapshot().unwrap())
            .unwrap();
        self.identities[index] = identity.clone();
        identity
    }

    pub async fn create_demand(&mut self, index: usize, duration_seconds: u16) -> RequestId {
        use meshway_configuration::execution::{BusinessProtocol, BusinessTarget, DemandDecision};
        let target = self.controls[1].status().binding.unwrap().node_id;
        let control = &mut self.controls[index];
        let lease = control.status().instance.unwrap();
        let id = request();
        let response = control
            .create_demand(&DemandRequest {
                instance: InstanceRequest {
                    instance_id: lease.instance_id,
                    generation: lease.generation,
                },
                request_id: id,
                target: BusinessTarget::Mesh { node: target },
                protocol: BusinessProtocol::Udp,
                port: Some(42424.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds,
            })
            .await
            .unwrap();
        assert!(matches!(
            response.decision,
            DemandDecision::Permitted { .. }
        ));
        id
    }

    pub async fn change_udp_buffer(&self, node: meshway_model::NodeId) {
        self.configure(|cfg| {
            let Listener::NativeUdp(listener) = cfg
                .nodes
                .get_mut(&node)
                .unwrap()
                .listeners
                .get_mut("udp")
                .unwrap()
            else {
                panic!("UDP listener");
            };
            listener.socket.receive_buffer = 65_536;
        })
        .await;
    }

    pub async fn set_access(&self, allowed: bool) {
        self.configure(|cfg| {
            cfg.access = if allowed {
                self.access.clone()
            } else {
                Vec::new()
            };
        })
        .await;
    }

    pub async fn close(self) {
        for control in self.controls {
            control.shutdown().await.unwrap();
        }
        self.app.stream_shutdown()();
        self.stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), self.server)
            .await
            .unwrap()
            .unwrap();
        self.app.shutdown().await.unwrap();
    }
}

pub(in crate::manager) fn carriers(
    identity: &DataPlaneIdentity,
) -> BTreeMap<meshway_model::NodeId, crate::Carrier> {
    let cfg = identity.snapshot().configuration().unwrap();
    cfg.dialers
        .values()
        .filter(|candidate| {
            candidate.parameters.transport() == meshway_transport_spec::TransportProtocol::NativeUdp
        })
        .map(|candidate| {
            (
                candidate.peer,
                crate::Carrier {
                    dialer: cfg.node_id,
                    listener: candidate.peer,
                    listener_key: candidate.listener_key.clone(),
                    protocol: candidate.parameters.transport(),
                },
            )
        })
        .collect()
}

fn udp_listener(address: SocketAddr) -> Listener {
    Listener::NativeUdp(NativeUdpListener {
        listen: address.ip(),
        listen_port: address.port().try_into().unwrap(),
        endpoints: vec![Endpoint {
            host: address.ip().to_string(),
            port: address.port().try_into().unwrap(),
        }],
        enabled: true,
        remark: String::new(),
        bind_interface: String::new(),
        socket: Default::default(),
    })
}

/// One topology/Server owner for plain, reverse, dual-stack and STUN scenarios.
#[derive(Default)]
pub(in crate::manager) struct FixtureBuilder {
    stream: bool,
    ipv6: bool,
    reverse: bool,
    multiple: bool,
    stun: Option<SocketAddr>,
    indirect_first: Option<bool>,
}

impl FixtureBuilder {
    pub fn stream(mut self, enabled: bool) -> Self {
        self.stream = enabled;
        self
    }

    pub fn ipv6(mut self, enabled: bool) -> Self {
        self.ipv6 = enabled;
        self
    }

    pub fn reverse(mut self) -> Self {
        self.reverse = true;
        self
    }

    pub fn multiple_listeners(mut self) -> Self {
        self.multiple = true;
        self
    }

    pub fn stun(mut self, address: SocketAddr) -> Self {
        self.stun = Some(address);
        self.reverse = true;
        self
    }

    pub fn indirect_first(mut self, order: Option<bool>) -> Self {
        self.indirect_first = order;
        self
    }

    pub async fn build(self) -> Fixture {
        Fixture::build(self).await
    }
}

/// Startup is separate from topology publication and never waits for a link.
/// Tests retain control of start order, limits, fault injection and shutdown.
pub(in crate::manager) struct ManagerBuilder {
    identity: DataPlaneIdentity,
    factory: std::sync::Arc<dyn super::SocketFactory>,
    limits: super::Limits,
    traversal: Option<(
        std::sync::Arc<dyn crate::traversal::TraversalGateway>,
        meshway_model::Sequence,
    )>,
}

impl ManagerBuilder {
    pub fn limits(mut self, limits: super::Limits) -> Self {
        self.limits = limits;
        self
    }

    pub fn traversal(
        mut self,
        gateway: std::sync::Arc<dyn crate::traversal::TraversalGateway>,
        generation: meshway_model::Sequence,
    ) -> Self {
        self.traversal = Some((gateway, generation));
        self
    }

    pub async fn try_start(self) -> Result<super::Manager, crate::Error> {
        match self.traversal {
            Some((gateway, generation)) => {
                super::Manager::start_with_traversal(
                    self.identity,
                    self.factory,
                    self.limits,
                    gateway,
                    generation,
                )
                .await
            }
            None => super::Manager::start(self.identity, self.factory, self.limits).await,
        }
    }

    pub async fn start(self) -> super::Manager {
        self.try_start().await.unwrap()
    }
}

mod sockets;
pub(super) use sockets::{Factory, Faults};

pub(super) async fn linked(
    manager: &super::Manager,
    count: usize,
    protocol: super::TransportProtocol,
) {
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            let facts = manager.snapshot().unwrap();
            if facts.links.len() == count
                && facts
                    .links
                    .iter()
                    .all(|link| link.carrier.protocol == protocol)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("bounded actual peer handshake/failover");
}

pub(super) async fn frame(manager: &mut super::Manager) -> (super::LinkFact, super::Frame) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match manager.recv().await {
                super::Event::Frame { link, frame, .. } => return (*link, frame),
                super::Event::Changed => {}
                super::Event::Closed => panic!("manager closed"),
            }
        }
    })
    .await
    .unwrap()
}
