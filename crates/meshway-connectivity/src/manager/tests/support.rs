//! Real Server/Control identities and publication, with only raw I/O injected.
use meshway_configuration::{AccessRule, Input, NetworkConfigurationInput};
use meshway_control::{Control, DataPlaneIdentity};
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
    client: Client,
    user_token: WireSecret,
    network: meshway_model::NetworkId,
    access: Vec<AccessRule>,
}

impl Fixture {
    pub async fn new(stream: bool, ipv6: bool) -> Self {
        Self::build(stream, ipv6, false, false, None).await
    }

    pub async fn reverse() -> Self {
        Self::build(false, false, true, false, None).await
    }

    pub async fn multiple_listeners() -> Self {
        Self::build(false, false, false, true, None).await
    }

    pub async fn traversal(stun: SocketAddr) -> Self {
        Self::build(false, false, true, false, Some(stun)).await
    }

    async fn build(
        stream: bool,
        ipv6: bool,
        reverse: bool,
        multiple: bool,
        stun: Option<SocketAddr>,
    ) -> Self {
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
            reqwest::Client::builder()
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
        let listener = NativeUdpListener {
            listen: udp_address.ip(),
            listen_port: udp_address.port().try_into().unwrap(),
            endpoints: vec![Endpoint {
                host: udp_address.ip().to_string(),
                port: udp_address.port().try_into().unwrap(),
            }],
            enabled: true,
            remark: String::new(),
            bind_interface: String::new(),
            socket: Default::default(),
        };
        cfg.nodes
            .get_mut(&ids[1])
            .unwrap()
            .listeners
            .insert("udp".into(), Listener::NativeUdp(listener));
        let mut extra_udp = Vec::new();
        if multiple {
            let socket = UdpSocket::bind(local).await.unwrap();
            let address = socket.local_addr().unwrap();
            cfg.nodes.get_mut(&ids[1]).unwrap().listeners.insert(
                "udp2".into(),
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
                }),
            );
            extra_udp.push(socket);
        }
        if let Some(socket) = &reverse_udp {
            let address = socket.local_addr().unwrap();
            cfg.nodes.get_mut(&ids[0]).unwrap().listeners.insert(
                "udp".into(),
                Listener::NativeUdp(NativeUdpListener {
                    enabled: true,
                    remark: String::new(),
                    listen: address.ip(),
                    listen_port: address.port().try_into().unwrap(),
                    endpoints: vec![Endpoint {
                        host: address.ip().to_string(),
                        port: address.port().try_into().unwrap(),
                    }],
                    bind_interface: String::new(),
                    socket: Default::default(),
                }),
            );
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
        let demands = [request(), request()];
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

    pub async fn change_udp_buffer(&self, node: meshway_model::NodeId) {
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

    pub async fn set_access(&self, allowed: bool) {
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
        cfg.access = if allowed {
            self.access.clone()
        } else {
            Vec::new()
        };
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
