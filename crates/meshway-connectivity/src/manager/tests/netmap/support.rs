//! Real Server enrollment and Control-verified netmaps. The netmaps are
//! compiled here by the shared policy compiler and delivered through Control's
//! sync path (scripted transport); each becomes the Manager's desired content.
use super::super::*;
use meshway_configuration::{NetworkConfiguration, netmap::CompiledNetmap};
use meshway_control::Control;
use meshway_protocol::EnrollmentResponse;
use meshway_test_server::{
    Facts, Script, TestServer,
    configuration::{compile, network, tag_node, trusted_keys},
};
use meshway_transport::spec::{
    DialerSet, Endpoint, NativeTcpDialer, NativeTcpListener, NativeUdpDialer, NativeUdpListener,
    PreparedListener,
};
use tokio::net::TcpListener;

const HOUR: u64 = 60 * 60 * 1000;

pub(in crate::manager) struct Netmaps {
    server: TestServer,
    controls: Vec<Control>,
    scripts: Vec<Arc<Script>>,
    bindings: Vec<EnrollmentResponse>,
    pub cfg: NetworkConfiguration,
    current: Vec<CompiledNetmap>,
    /// Each Node's desired content: Control's currently published netmap.
    pub desired: Vec<Desired>,
    pub keys: Vec<DataPlaneKeys>,
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

pub(in crate::manager) fn tcp_listener(address: SocketAddr) -> Listener {
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
    /// Nodes a, b, c. All three listen on UDP (b with or without a configured
    /// endpoint), so every edge is dialable both ways and on demand; `stream`
    /// makes b the only (Native TCP) listener and every Node dial TCP, so
    /// edges toward b are one-way and resident.
    pub async fn start(stream: bool, configured: bool) -> Self {
        let server = TestServer::start().await;
        let owner = server
            .owner(
                "netmap@example.test",
                "real netmap connectivity password",
                "Netmap adjacency",
            )
            .await;
        let (mut controls, mut scripts, mut bindings) = (Vec::new(), Vec::new(), Vec::new());
        for name in ["a", "b", "c"] {
            let mut control = server.open_control(
                name,
                meshway_test_server::http().timeout(Duration::from_secs(10)),
            );
            bindings.push(server.enroll(&owner, &mut control, name).await);
            control.authenticate().await.unwrap();
            let script = Script::new();
            control.set_sync_transport(script.clone());
            controls.push(control);
            scripts.push(script);
        }
        let udp = [
            UdpSocket::bind("127.0.0.1:0").await.unwrap(),
            UdpSocket::bind("127.0.0.1:0").await.unwrap(),
            UdpSocket::bind("127.0.0.1:0").await.unwrap(),
        ];
        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = network("netmap.internal");
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
            if index != 1 && !stream {
                // c on the second socket, a on the third.
                let socket = &udp[if index == 2 { 1 } else { 2 }];
                listeners.insert(
                    "udp".to_owned(),
                    PreparedListener::prepare(udp_listener(socket.local_addr().unwrap(), true))
                        .unwrap(),
                );
            }
            cfg.nodes.insert(
                binding.node_id,
                tag_node(["a", "b", "c"][index], index as u8 + 1, listeners, false),
            );
        }
        let mut fixture = Self {
            server,
            keys: controls
                .iter()
                .map(|control| control.dataplane_keys().unwrap())
                .collect(),
            controls,
            scripts,
            bindings,
            cfg,
            current: Vec::new(),
            desired: Vec::new(),
            sockets: Some((udp.into(), vec![tcp])),
        };
        fixture.publish(|_| {}).await;
        fixture
    }

    /// The reserved TCP listener address (b's listener when `stream`).
    pub fn tcp_address(&self) -> SocketAddr {
        let (_, tcp) = self.sockets.as_ref().unwrap();
        tcp[0].local_addr().unwrap()
    }

    /// The reserved UDP listener socket of `index` (b: 1, c: 2), for tests
    /// driving an owner directly instead of through a factory.
    pub fn take_udp(&mut self, index: usize) -> UdpSocket {
        let (udp, _) = self.sockets.as_mut().unwrap();
        udp.remove(index - 1)
    }

    /// A content update that keeps every Node's identity, addresses, keys
    /// and carriers (one more access rule); returns `index`'s new content.
    pub async fn reserial(&mut self, index: usize) -> Desired {
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
        self.desired[index].clone()
    }

    pub fn ids(&self) -> Vec<NodeId> {
        self.bindings
            .iter()
            .map(|binding| binding.node_id)
            .collect()
    }

    /// Control's currently published netmap as desired content.
    fn published(&self, index: usize) -> Desired {
        Desired::new(
            &self.controls[index].netmap().unwrap().content,
            &Default::default(),
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
            blocked_datagrams: Mutex::new(BTreeSet::new()),
            acquisitions: AtomicUsize::new(0),
            datagram_dials: AtomicUsize::new(0),
            traversal_binds: AtomicUsize::new(0),
            traversal_listeners: AtomicUsize::new(0),
            rebinds: Arc::default(),
            absent: Mutex::default(),
            remotes: Mutex::default(),
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
                    trusted_keys(&binding.keys, crate::capabilities()),
                )
            })
            .collect();
        let previous = self
            .current
            .iter()
            .map(|netmap| (netmap.netmap.node_id, netmap))
            .collect();
        let mut compiled = compile(
            self.bindings[0].network_id,
            &self.cfg,
            &inventory,
            &previous,
        );
        let mut changed = Vec::new();
        let mut current = Vec::new();
        for index in 0..self.bindings.len() {
            let netmap = compiled.remove(&self.bindings[index].node_id).unwrap();
            let same = self
                .current
                .get(index)
                .is_some_and(|old| old.reference() == netmap.reference());
            changed.push(!same);
            self.scripts[index].push(Ok(meshway_test_server::reply(&netmap, HOUR, !same)));
            self.controls[index]
                .sync(&mut Facts::default())
                .await
                .unwrap();
            let desired = self.published(index);
            if let Some(slot) = self.desired.get_mut(index) {
                *slot = desired;
            } else {
                self.desired.push(desired);
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
        self.server.close().await;
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
