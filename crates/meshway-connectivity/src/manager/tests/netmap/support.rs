//! Real Server enrollment and Control-issued netmap snapshots. The netmaps are
//! compiled here by the shared policy compiler and delivered through Control's
//! sync path (scripted transport) until the Server serves the route.
use super::super::*;
use meshway_authority::DataPlaneIdentity;
use meshway_configuration::{
    AddressPools, Input, NetworkConfiguration, NodeAddresses, NodeIdentity, NodeSettings,
    SubjectContext, execution::TrustedNode, netmap::CompiledNetmap,
};
use meshway_control::{Control, SyncFuture, SyncTransport};
use meshway_control_client::{ApproveDeviceAuthorizationPath, Client, ClientError};
use meshway_model::Revision;
use meshway_policy::compile::netmap::{NetmapInput, PreviousNetmap, compile_netmap};
use meshway_protocol::{
    ApproveEnrollmentRequest, CreateNetworkRequest, EnrollmentResponse, ErrorResponse,
    RegisterRequest, SyncRequest, SyncResponse, WireSecret,
};
use meshway_server::{App, Settings};
use meshway_transport_spec::{
    DialerSet, Endpoint, NativeTcpDialer, NativeTcpListener, NativeUdpDialer, NativeUdpListener,
    PreparedListener,
};
use std::{collections::VecDeque, sync::Mutex as StdMutex};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

const HOUR: u64 = 60 * 60 * 1000;

type Reply = Result<SyncResponse, ClientError<ErrorResponse>>;

#[derive(Default)]
struct Script(StdMutex<VecDeque<Reply>>);
impl SyncTransport for Script {
    fn sync<'a>(&'a self, _request: &'a SyncRequest, _token: &'a str) -> SyncFuture<'a> {
        let reply = self.0.lock().unwrap().pop_front();
        Box::pin(async move { reply.unwrap_or(Err(ClientError::Network { status: None })) })
    }
}

#[derive(Default)]
struct NoFacts;
impl meshway_control::PeerFactsSink for NoFacts {
    fn apply(
        &mut self,
        _: meshway_model::NetmapRef,
        _: meshway_protocol::PeerFactsDelta,
    ) -> Result<(), meshway_control::FactsRejected> {
        Ok(())
    }
}

pub(in crate::manager) struct Netmaps {
    app: App,
    proxy: meshway_netmap_fixture::PinningProxy,
    stop: oneshot::Sender<()>,
    server: JoinHandle<()>,
    _temp: tempfile::TempDir,
    controls: Vec<Control>,
    /// Staged migration: snapshots of each Control's verified netmap.
    issuers: Vec<meshway_authority::StagedIssuer>,
    scripts: Vec<Arc<Script>>,
    bindings: Vec<EnrollmentResponse>,
    pub cfg: NetworkConfiguration,
    current: Vec<CompiledNetmap>,
    pub identities: Vec<DataPlaneIdentity>,
    sockets: Option<(Vec<UdpSocket>, Vec<TcpListener>)>,
}

pub(in crate::manager) fn udp_listener(address: SocketAddr, configured: bool) -> Listener {
    Listener::NativeUdp(NativeUdpListener {
        listen: address.ip(),
        listen_port: address.port().try_into().unwrap(),
        endpoints: if configured {
            vec![Endpoint {
                host: address.ip().to_string(),
                port: address.port().try_into().unwrap(),
            }]
        } else {
            vec![]
        },
        enabled: true,
        remark: String::new(),
        bind_interface: String::new(),
        socket: Default::default(),
    })
}

fn tcp_listener(address: SocketAddr) -> Listener {
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
    })
}

impl Netmaps {
    /// Nodes a, b, c. b and c listen on UDP (b with or without a configured
    /// endpoint); `stream` makes b listen on Native TCP only and dial TCP.
    pub async fn start(stream: bool, configured: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let app = App::open(Settings::new(root.join("server"))).await.unwrap();
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
            let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
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
        let proxy = meshway_netmap_fixture::PinningProxy::start(&origin).await;
        // Password hashing is bounded process-wide; parallel fixtures retry
        // a busy answer instead of failing.
        let user = loop {
            match client
                .register(&RegisterRequest {
                    email: "netmap@example.test".into(),
                    password: WireSecret::new("real netmap connectivity password".into()),
                })
                .await
            {
                Err(ClientError::Http { status: 503, .. }) => {
                    tokio::time::sleep(Duration::from_millis(50)).await
                }
                result => break result.unwrap(),
            }
        };
        let network = client
            .create_network(
                &CreateNetworkRequest {
                    request_id: support::request(),
                    name: "Netmap adjacency".into(),
                },
                user.token.expose(),
            )
            .await
            .unwrap()
            .network
            .id;
        let (mut controls, mut scripts, mut bindings) = (Vec::new(), Vec::new(), Vec::new());
        for name in ["a", "b", "c"] {
            let mut control = Control::open(&root.join(name), &proxy.origin, builder()).unwrap();
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
            bindings.push(control.redeem_enrollment().await.unwrap());
            control.authenticate().await.unwrap();
            let script = Arc::new(Script::default());
            control.set_sync_transport(script.clone());
            controls.push(control);
            scripts.push(script);
        }
        let udp = [
            UdpSocket::bind("127.0.0.1:0").await.unwrap(),
            UdpSocket::bind("127.0.0.1:0").await.unwrap(),
        ];
        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = meshway_configuration::new_network(
            AddressPools {
                ipv4: vec!["100.64.0.0/24".into()],
                ipv6: vec![],
            },
            "netmap.internal".into(),
        )
        .unwrap();
        cfg.tags_owners.insert("tag:server".into(), vec![]);
        cfg.defaults.dialers = DialerSet {
            native_udp: (!stream).then(|| NativeUdpDialer {
                connect_timeout: "2s".parse().unwrap(),
                ..Default::default()
            }),
            native_tcp: stream.then(NativeTcpDialer::default),
            vless: None,
            trojan: None,
        };
        for (index, binding) in bindings.iter().enumerate() {
            let mut listeners = BTreeMap::new();
            if index == 1 {
                let listener = if stream {
                    tcp_listener(tcp.local_addr().unwrap())
                } else {
                    udp_listener(udp[0].local_addr().unwrap(), configured)
                };
                let key = if stream { "tcp" } else { "udp" };
                listeners.insert(key.to_owned(), PreparedListener::prepare(listener).unwrap());
            }
            if index == 2 && !stream {
                listeners.insert(
                    "udp".to_owned(),
                    PreparedListener::prepare(udp_listener(udp[1].local_addr().unwrap(), true))
                        .unwrap(),
                );
            }
            cfg.nodes.insert(
                binding.node_id,
                NodeSettings {
                    hostname: ["a", "b", "c"][index].into(),
                    remark: String::new(),
                    identity: NodeIdentity::Tag {
                        tags: vec!["tag:server".into()],
                    },
                    enabled: true,
                    addresses: NodeAddresses {
                        ipv4: Some(format!("100.64.0.{}", index + 1).parse().unwrap()),
                        ipv6: None,
                    },
                    listeners,
                    dialers: Input::Omitted,
                    exit_enabled: false,
                },
            );
        }
        let mut fixture = Self {
            app,
            proxy,
            stop,
            server,
            _temp: temp,
            issuers: controls
                .iter()
                .map(|_| meshway_authority::StagedIssuer::new().unwrap())
                .collect(),
            controls,
            scripts,
            bindings,
            cfg,
            current: Vec::new(),
            identities: Vec::new(),
            sockets: Some((udp.into(), vec![tcp])),
        };
        fixture.publish(|_| {}).await;
        fixture
    }

    /// The reserved UDP listener socket of `index` (b: 1, c: 2), for tests
    /// driving an owner directly instead of through a factory.
    pub fn take_udp(&mut self, index: usize) -> UdpSocket {
        let (udp, _) = self.sockets.as_mut().unwrap();
        udp.remove(index - 1)
    }

    /// A content update that keeps every Node's identity, addresses, keys
    /// and carriers (one more access rule); returns `index`'s new identity.
    pub async fn reserial(&mut self, index: usize) -> DataPlaneIdentity {
        let ids = self.ids();
        let port = 1000 + self.cfg.access.len();
        let changed = self
            .publish(|cfg| {
                cfg.access.push(meshway_configuration::AccessRule {
                    remark: String::new(),
                    src: vec![format!("node:{}", ids[0])],
                    dst: vec![format!("node:{}", ids[1])],
                    ip: vec![format!("tcp:{port}")],
                    bidirectional: false,
                })
            })
            .await;
        assert!(changed[index]);
        self.identities[index].clone()
    }

    pub fn ids(&self) -> Vec<NodeId> {
        self.bindings
            .iter()
            .map(|binding| binding.node_id)
            .collect()
    }

    pub async fn renew(&mut self, index: usize, validity_ms: u64) {
        self.scripts[index]
            .0
            .lock()
            .unwrap()
            .push_back(Ok(meshway_netmap_fixture::reply(
                &self.current[index],
                validity_ms,
                false,
            )));
        self.controls[index].sync(&mut NoFacts).await.unwrap();
        self.identities[index] = self.identity(index);
    }

    /// The packet-plane identity for Control's currently published netmap.
    fn identity(&mut self, index: usize) -> DataPlaneIdentity {
        let control = &self.controls[index];
        let netmap = control.netmap().unwrap();
        let instance =
            control
                .status()
                .instance
                .map(|instance| meshway_protocol::InstanceRequest {
                    instance_id: instance.instance_id,
                    generation: instance.generation,
                });
        self.issuers[index]
            .issue(
                &netmap.content,
                netmap.lease,
                instance,
                &control.dataplane_keys().unwrap(),
            )
            .unwrap()
    }

    /// The reserved listener sockets, for one shared test factory.
    pub(in crate::manager::tests) fn factory(&mut self) -> Arc<Factory> {
        let (udp, tcp) = self.sockets.take().unwrap();
        Arc::new(Factory {
            udp: Mutex::new(
                udp.into_iter()
                    .map(|socket| (socket.local_addr().unwrap(), socket))
                    .collect(),
            ),
            tcp: Mutex::new(
                tcp.into_iter()
                    .map(|socket| (socket.local_addr().unwrap(), socket))
                    .collect(),
            ),
            faults: Arc::default(),
            blocked_dialers: Mutex::new(BTreeSet::new()),
            acquisitions: AtomicUsize::new(0),
        })
    }

    /// Change the configuration and deliver every Node's netmap; returns
    /// which Nodes received new content (a larger serial).
    pub async fn publish(&mut self, change: impl FnOnce(&mut NetworkConfiguration)) -> Vec<bool> {
        change(&mut self.cfg);
        let inventory: BTreeMap<_, _> = self
            .bindings
            .iter()
            .map(|binding| {
                (
                    binding.node_id,
                    TrustedNode {
                        control_key: binding.keys.control.clone(),
                        dataplane_key: binding.keys.dataplane.clone(),
                        capabilities: crate::capabilities(),
                    },
                )
            })
            .collect();
        let previous = self
            .current
            .iter()
            .map(|netmap| (netmap.netmap.node_id, PreviousNetmap::from(netmap)))
            .collect();
        let mut compiled = compile_netmap(NetmapInput {
            network: self.bindings[0].network_id,
            revision: Revision::MIN,
            configuration: &self.cfg,
            subjects: &SubjectContext::default(),
            inventory: &inventory,
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            selection_versions: &BTreeMap::new(),
            previous: &previous,
        })
        .unwrap();
        let mut changed = Vec::new();
        let mut current = Vec::new();
        for index in 0..self.bindings.len() {
            let netmap = compiled
                .nodes
                .remove(&self.bindings[index].node_id)
                .unwrap();
            let same = self
                .current
                .get(index)
                .is_some_and(|old| old.reference() == netmap.reference());
            changed.push(!same);
            self.scripts[index]
                .0
                .lock()
                .unwrap()
                .push_back(Ok(meshway_netmap_fixture::reply(&netmap, HOUR, !same)));
            self.controls[index].sync(&mut NoFacts).await.unwrap();
            let identity = self.identity(index);
            if let Some(slot) = self.identities.get_mut(index) {
                *slot = identity;
            } else {
                self.identities.push(identity);
            }
            current.push(netmap);
        }
        self.current = current;
        changed
    }

    pub async fn close(self) {
        for control in self.controls {
            control.shutdown().await.unwrap();
        }
        self.proxy.close().await;
        self.app.stream_shutdown()();
        self.stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), self.server)
            .await
            .unwrap()
            .unwrap();
        self.app.shutdown().await.unwrap();
    }
}

pub(in crate::manager) async fn eventually(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(12), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what}"));
}

pub(in crate::manager) fn links(manager: &Manager) -> BTreeMap<NodeId, LinkFact> {
    manager
        .snapshot()
        .unwrap()
        .links
        .into_iter()
        .map(|link| (link.peer, link))
        .collect()
}
