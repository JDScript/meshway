//! A real in-process Server enrolls one Node; its production Control issues
//! every snapshot from sync replies carrying netmaps compiled by the shared
//! Policy compiler for that identity. No second issuer exists here.
use meshway_configuration::{
    AccessRule, execution::TrustedNode, netmap::CompiledNetmap, transport::PreparedListener,
};
use meshway_control::Control;
use meshway_model::{NetworkId, NodeId};
use meshway_test_server::{
    Facts, Script, TestServer,
    configuration::{compile_one, network, node, sel, tag_node, trusted},
};
use std::{collections::BTreeMap, path::Path, sync::Arc};

pub const HOUR: u64 = 60 * 60 * 1000;

pub struct Server(TestServer);

impl Server {
    pub async fn start() -> Self {
        Self(TestServer::start().await)
    }

    /// One enrolled Node whose sync replies come from `Script`.
    pub async fn enroll(&self, directory: &Path) -> (Control, Arc<Script>) {
        let owner = self
            .0
            .owner(
                "underlay-netmap@example.test",
                "isolated underlay netmap fixture",
                "Underlay netmap",
            )
            .await;
        let mut control = Control::open(
            &directory.join("node"),
            &self.0.node_origin,
            meshway_test_server::http(),
        )
        .unwrap();
        self.0.enroll(&owner, &mut control, "underlay-node").await;
        control.authenticate().await.unwrap();
        let script = Script::new();
        control.set_sync_transport(script.clone());
        (control, script)
    }

    pub async fn close(self) {
        self.0.close().await;
    }
}

pub async fn sync(
    control: &mut Control,
    script: &Script,
    netmap: &CompiledNetmap,
    full: bool,
    validity_ms: u64,
) -> meshway_engine::Desired {
    script.push(Ok(meshway_test_server::reply(netmap, validity_ms, full)));
    let netmap = control.sync(&mut Facts::default()).await.unwrap().netmap;
    meshway_engine::Desired::new(&netmap.content).unwrap()
}

pub fn listener(port: u16) -> PreparedListener {
    meshway_test_server::configuration::listener(
        serde_json::json!({"type": "native_udp", "listenPort": port}),
    )
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
        let mut cfg = network("test.internal");
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
        for (id, i, listeners) in [
            (self.self_id, 1, self.listeners.clone()),
            (peer, 2, BTreeMap::from([("udp".into(), listener(41001))])),
        ] {
            cfg.nodes.insert(
                id,
                tag_node(
                    &format!("n{i}"),
                    i,
                    listeners,
                    self.exit && id == self.self_id,
                ),
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
        compile_one(self.network, &cfg, &inventory, self.self_id, previous)
    }
}
