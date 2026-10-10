//! A real in-process Server enrolls one Node; its production Control issues
//! every snapshot from sync replies carrying netmaps compiled by the shared
//! Policy compiler for that identity. No second issuer exists here.
use meshway_configuration::{
    AccessRule, AddressPools, Input, NodeAddresses, NodeIdentity, NodeSettings, SubjectContext,
    execution::TrustedNode,
    netmap::CompiledNetmap,
    transport::{Listener, PreparedListener},
};
use meshway_control::{Control, FactsRejected, PeerFactsSink, SyncFuture, SyncTransport};
use meshway_control_client::{ApproveDeviceAuthorizationPath, Client, ClientError};
use meshway_model::{
    BoundNodeKey, KeyAlgorithm, NetmapRef, NetworkId, NodeId, NodePublicKey, RequestId, Revision,
    Version,
    state::{Capability, StreamKind, TransportRole},
};
use meshway_policy::compile::netmap::{NetmapInput, PreviousNetmap, compile_netmap};
use meshway_protocol::{
    ApproveEnrollmentRequest, CreateNetworkRequest, PeerFactsDelta, RegisterRequest, SyncRequest,
    SyncResponse, WireSecret,
};
use meshway_server::{App, Settings};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub const HOUR: u64 = 60 * 60 * 1000;

pub fn request() -> RequestId {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).unwrap();
    bytes[6] = (bytes[6] & 15) | 0x40;
    bytes[8] = (bytes[8] & 63) | 0x80;
    let hex = format!("{:032x}", u128::from_be_bytes(bytes));
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
    .parse()
    .unwrap()
}

#[derive(Default)]
pub struct Script(Mutex<VecDeque<SyncResponse>>);
impl SyncTransport for Script {
    fn sync<'a>(&'a self, _: &'a SyncRequest, _: &'a str) -> SyncFuture<'a> {
        let reply = self.0.lock().unwrap().pop_front();
        Box::pin(async move { reply.ok_or(ClientError::Network { status: None }) })
    }
}
pub struct Facts;
impl PeerFactsSink for Facts {
    fn apply(&mut self, _: NetmapRef, _: PeerFactsDelta) -> Result<(), FactsRejected> {
        Ok(())
    }
}

pub struct Server {
    app: Option<App>,
    task: Option<tokio::task::JoinHandle<()>>,
    stop: CancellationToken,
    client: Client,
    /// Node traffic goes through it so enrollment pins the fixture netmap key.
    proxy: meshway_netmap_fixture::PinningProxy,
}

impl Server {
    pub async fn start(directory: &Path) -> Self {
        let app = App::open(Settings::new(directory.join("server")))
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = meshway_server::http::router(&app);
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(stopped.cancelled_owned())
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
            app: Some(app),
            task: Some(task),
            stop,
            client,
            proxy: meshway_netmap_fixture::PinningProxy::start(&origin).await,
        }
    }

    /// One enrolled Node whose sync replies come from `Script`.
    pub async fn enroll(&self, directory: &Path) -> (Control, Arc<Script>) {
        let session = self
            .client
            .register(&RegisterRequest {
                email: "underlay-netmap@example.test".into(),
                password: WireSecret::new("isolated underlay netmap fixture".into()),
            })
            .await
            .unwrap();
        let network = self
            .client
            .create_network(
                &CreateNetworkRequest {
                    request_id: request(),
                    name: "Underlay netmap".into(),
                },
                session.token.expose(),
            )
            .await
            .unwrap()
            .network
            .id;
        let mut control = Control::open(
            &directory.join("node"),
            &self.proxy.origin,
            meshway_control_client::http_client_builder()
                .unwrap()
                .no_proxy(),
        )
        .unwrap();
        let pending = control.begin_enrollment("underlay-node").await.unwrap();
        self.client
            .approve_device_authorization(
                &ApproveDeviceAuthorizationPath {
                    enrollment_request_id: pending.id,
                },
                &ApproveEnrollmentRequest {
                    network_id: network,
                },
                session.token.expose(),
            )
            .await
            .unwrap();
        control.poll_enrollment().await.unwrap();
        control.redeem_enrollment().await.unwrap();
        control.authenticate().await.unwrap();
        let script = Arc::new(Script::default());
        control.set_sync_transport(script.clone());
        (control, script)
    }

    pub async fn close(mut self) {
        self.app.as_ref().unwrap().stream_shutdown()();
        self.stop.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(10), self.task.take().unwrap()).await;
        let _ = tokio::time::timeout(Duration::from_secs(10), self.app.take().unwrap().shutdown())
            .await;
    }
}

pub async fn sync(
    control: &mut Control,
    script: &Script,
    netmap: &CompiledNetmap,
    full: bool,
    validity_ms: u64,
) -> meshway_engine::Desired {
    script
        .0
        .lock()
        .unwrap()
        .push_back(meshway_netmap_fixture::reply(netmap, validity_ms, full));
    let netmap = control.sync(&mut Facts).await.unwrap().netmap;
    meshway_engine::Desired::new(&netmap.content).unwrap()
}

pub fn node(i: u8) -> NodeId {
    format!("00000000-0000-4000-8000-{i:012}").parse().unwrap()
}
pub fn sel(id: NodeId) -> String {
    format!("node:{id}")
}

fn trusted(i: u8, exit: bool) -> TrustedNode {
    let key = |offset: u8, algorithm| BoundNodeKey {
        id: format!(
            "00000000-0000-4002-8000-{:012}",
            u16::from(i) + u16::from(offset)
        )
        .parse()
        .unwrap(),
        generation: Version::MIN,
        public_key: NodePublicKey {
            algorithm,
            bytes: [i.wrapping_add(offset); 32].into(),
        },
    };
    let mut capabilities = BTreeMap::from([(
        "transport:native_udp".into(),
        Capability::Transport {
            roles: vec![TransportRole::Dial, TransportRole::Listen],
            streams: vec![StreamKind::Udp],
        },
    )]);
    if exit {
        capabilities.insert(
            "feature:exit-ipv4".into(),
            Capability::Feature { version: 1 },
        );
    }
    TrustedNode {
        control_key: key(0, KeyAlgorithm::Ed25519),
        dataplane_key: key(100, KeyAlgorithm::X25519),
        capabilities,
    }
}

pub fn listener(port: u16) -> PreparedListener {
    let value: Listener =
        serde_json::from_str(&format!(r#"{{"type":"native_udp","listenPort":{port}}}"#)).unwrap();
    PreparedListener::prepare(value).unwrap()
}

/// The enrolled Node is `self_id` (100.64.0.1). Peer 2 (100.64.0.2) may be
/// reached by the source when `access`. With `exit`, the enrolled Node is an
/// exit-enabled exit for source 2, which may reach the Internet through it.
pub struct Network {
    pub network: NetworkId,
    pub self_id: NodeId,
    pub keys: TrustedNode,
    pub listeners: BTreeMap<String, PreparedListener>,
    pub exit: bool,
    pub access: bool,
}

impl Network {
    pub fn new(control: &Control, exit: bool) -> Self {
        let binding = control.status().binding.unwrap();
        let mut keys = trusted(9, exit);
        keys.control_key = binding.keys.control.clone();
        keys.dataplane_key = binding.keys.dataplane.clone();
        Self {
            network: binding.network_id,
            self_id: binding.node_id,
            keys,
            listeners: BTreeMap::from([
                ("udp".into(), listener(41001)),
                ("keep".into(), listener(41002)),
            ]),
            exit,
            access: true,
        }
    }

    pub fn compile(&self, previous: Option<&CompiledNetmap>) -> CompiledNetmap {
        use meshway_configuration::{ProfileAssignment, RouteAction, RoutingProfile};
        let mut cfg = meshway_configuration::new_network(
            AddressPools {
                ipv4: vec!["100.64.0.0/24".into()],
                ipv6: vec![],
            },
            "test.internal".into(),
        )
        .unwrap();
        let peer = node(2);
        cfg.access.clear();
        if self.access {
            cfg.access.push(AccessRule {
                remark: String::new(),
                src: vec![sel(peer)],
                dst: if self.exit {
                    vec!["autogroup:internet".into()]
                } else {
                    vec![sel(self.self_id)]
                },
                ip: vec!["udp:*".into()],
                bidirectional: false,
            });
        }
        cfg.tags_owners.insert("tag:server".into(), vec![]);
        for (id, i, listeners) in [
            (self.self_id, 1, self.listeners.clone()),
            (peer, 2, BTreeMap::from([("udp".into(), listener(41001))])),
        ] {
            cfg.nodes.insert(
                id,
                NodeSettings {
                    hostname: format!("n{i}"),
                    remark: String::new(),
                    identity: NodeIdentity::Tag {
                        tags: vec!["tag:server".into()],
                    },
                    enabled: true,
                    addresses: NodeAddresses {
                        ipv4: Some(format!("100.64.0.{i}").parse().unwrap()),
                        ipv6: None,
                    },
                    listeners,
                    dialers: Input::Omitted,
                    exit_enabled: self.exit && id == self.self_id,
                },
            );
        }
        if self.exit {
            cfg.routing.egress.profiles.insert(
                "exit".into(),
                RoutingProfile {
                    rules: vec![],
                    fallback: RouteAction::Exit {
                        nodes: vec![sel(self.self_id)],
                        via: vec![],
                    },
                },
            );
            cfg.routing.egress.assignments.push(ProfileAssignment {
                target: vec![sel(peer)],
                allowed: vec!["exit".into()],
                default: "exit".into(),
                locked: false,
            });
        }
        let inventory =
            BTreeMap::from([(self.self_id, self.keys.clone()), (peer, trusted(2, false))]);
        let previous = previous
            .map(|old| BTreeMap::from([(self.self_id, PreviousNetmap::from(old))]))
            .unwrap_or_default();
        compile_netmap(NetmapInput {
            network: self.network,
            revision: Revision::MIN,
            configuration: &cfg,
            subjects: &SubjectContext::default(),
            inventory: &inventory,
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            selection_versions: &BTreeMap::new(),
            previous: &previous,
        })
        .unwrap()
        .nodes
        .remove(&self.self_id)
        .unwrap()
    }
}
