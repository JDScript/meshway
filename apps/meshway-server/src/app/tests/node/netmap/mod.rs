//! Netmap chain on a real App: State reports, visibility, UDP coordination and
//! observations read stored netmaps and never compile or publish. The netmap
//! store is not part of this package; `StoreNetmaps` compiles the real stored
//! configuration and inventory with the production netmap compiler and keeps
//! serials across calls exactly as a store retaining previous content would.
use super::*;
use crate::state::facts::NetmapReader;
use meshway_configuration::netmap::CompiledNetmap;
use meshway_model::state::*;
use meshway_policy::compile::netmap::{NetmapInput, PreviousNetmap, compile_netmap};
use meshway_protocol::state::StateReport;
use std::sync::{Arc, Mutex};

mod facts;

pub(super) struct StoreNetmaps {
    sealer: Arc<crate::secrets::Sealer>,
    previous: Mutex<BTreeMap<NodeId, PreviousNetmap>>,
}

impl NetmapReader for StoreNetmaps {
    fn current(
        &self,
        tx: &rusqlite::Transaction<'_>,
        network: NetworkId,
        node: NodeId,
    ) -> Result<Option<Arc<CompiledNetmap>>, Error> {
        let (revision, cfg, bindings) =
            store::execution::current_configuration(tx, &self.sealer, network)?;
        let rule_sets =
            crate::resources::bindings::at_revision(tx, network, revision, &bindings.rule_sets)?;
        let rule_sets = crate::resources::bindings::compilation_contents(tx, network, &rule_sets)?;
        let context = store::execution::compilation_context(tx, network, &cfg, &bindings)?;
        let mut previous = self.previous.lock().unwrap();
        let compiled = compile_netmap(NetmapInput {
            network,
            revision,
            configuration: &cfg,
            subjects: &context.subjects,
            inventory: &context.inventory,
            rule_sets: &rule_sets,
            selected_profiles: &context.selected_profiles,
            selection_versions: &BTreeMap::new(),
            previous: &previous,
        })
        .map_err(|_| Error::Internal("test netmap compilation"))?;
        *previous = compiled
            .nodes
            .iter()
            .map(|(id, netmap)| (*id, PreviousNetmap::from(netmap)))
            .collect();
        Ok(compiled.nodes.get(&node).cloned().map(Arc::new))
    }
}

pub(super) struct Lab {
    pub _dir: tempfile::TempDir,
    pub app: App,
    pub network: NetworkId,
    pub owner: String,
    /// a and b may talk (a→b icmp and tcp:443) over an a–b topology edge;
    /// c is registered but sees and is seen by nobody.
    pub nodes: [NodeId; 3],
    pub tokens: [String; 3],
    pub instances: [InstanceRequest; 3],
    /// Each Node's current execution target, reported as applied.
    pub applied: [ConfigurationRef; 3],
    pub netmaps: Arc<StoreNetmaps>,
}

pub(super) async fn lab(exit: bool) -> Lab {
    let (dir, app, network, a, b, ak, bk, asigner, bsigner) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let csigner = SigningKey::from_bytes(&[64; 32]);
    let c = enroll_tag_node(&app, owner.token.expose(), network, &csigner, "c", [74; 32]).await;
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let mut config = current.document.configuration.clone();
    config.defaults.topology = TopologyDefault::Deny;
    config.topology = vec![TopologyRule {
        transports: Input::Omitted,
        remark: String::new(),
        action: TopologyAction::Allow,
        a: format!("node:{a}"),
        b: format!("node:{b}"),
    }];
    config.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{a}")],
        dst: vec![format!("node:{b}")],
        ip: vec!["icmp:*".into(), "tcp:443".into()],
        bidirectional: false,
    }];
    if exit {
        config.nodes.get_mut(&b).unwrap().exit_enabled = true;
        config.access.push(AccessRule {
            remark: String::new(),
            src: vec!["*".into()],
            dst: vec!["autogroup:internet".into()],
            ip: vec!["tcp:443".into()],
            bidirectional: false,
        });
        config.routing.egress.rules.push(EgressRule {
            remark: String::new(),
            matcher: RouteMatch {
                ip_cidr: vec!["8.8.8.8/32".into()],
                ..Default::default()
            },
            action: RuleAction::Exit {
                nodes: vec![format!("node:{b}")],
                via: vec![],
            },
        });
    }
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(canonical_json(&config).unwrap().expose().into()),
            },
            rid(),
        )
        .await
        .unwrap();
    let service = app.execution();
    let mut tokens = Vec::new();
    let mut instances = Vec::new();
    let mut applied = Vec::new();
    for (node, keys, signer) in [
        (a, &ak, &asigner),
        (b, &bk, &bsigner),
        (c.node_id, &c.keys, &csigner),
    ] {
        let session = session(&service, network, node, keys, signer).await;
        let lease = service
            .acquire(
                session.token.expose(),
                AcquireInstanceRequest {
                    instance_id: iid(),
                    capabilities: caps(),
                },
            )
            .await
            .unwrap();
        let instance = InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        };
        applied.push(
            service
                .renew(session.token.expose(), instance.clone())
                .await
                .unwrap()
                .target
                .unwrap(),
        );
        tokens.push(session.token.expose().to_owned());
        instances.push(instance);
    }
    let netmaps = Arc::new(StoreNetmaps {
        sealer: app.sealer(),
        previous: Mutex::default(),
    });
    app.install_netmaps(netmaps.clone());
    Lab {
        _dir: dir,
        app,
        network,
        owner: owner.token.expose().to_owned(),
        nodes: [a, b, c.node_id],
        tokens: tokens.try_into().unwrap(),
        instances: instances.try_into().unwrap(),
        applied: applied.try_into().unwrap(),
        netmaps,
    }
}

impl Lab {
    pub async fn netmap(&self, node: NodeId) -> Arc<CompiledNetmap> {
        let netmaps = self.netmaps.clone();
        let network = self.network;
        self.app
            .executor
            .handle()
            .read(move |tx| netmaps.current(tx, network, node))
            .await
            .unwrap()
            .unwrap()
    }

    /// Publications compiled on the SQLite owner since the previous call.
    pub async fn publications(&self) -> usize {
        self.app
            .executor
            .handle()
            .read(|_| Ok(crate::execution::diagnostics::take_publications()))
            .await
            .unwrap()
    }

    /// A complete report of the `udp` listener with the given local endpoints.
    pub async fn report_endpoints(&self, index: usize, sequence: u64, addresses: &[&str]) {
        let applied = self.applied[index].clone();
        let now = chrono::Utc::now();
        let report = StateReport {
            instance_id: self.instances[index].instance_id,
            sequence: Sequence::new(sequence).unwrap(),
            observed_at: now,
            state: NodeState {
                system: SystemState {
                    hostname: "netmap-fixture".into(),
                    os: "linux".into(),
                    arch: "aarch64".into(),
                    agent_version: "test".into(),
                },
                capabilities: caps(),
                configuration: ConfigurationState {
                    saved: Some(applied.clone()),
                    applied: Some(applied),
                    progress: None,
                },
                exit_capability: None,
                listeners: BTreeMap::from([(
                    "udp".into(),
                    ListenerState {
                        phase: ListenerPhase::Listening,
                        socket_ids: vec!["s1".into()],
                        loaded_certificate_sha256: None,
                        problem: None,
                        configuration: None,
                    },
                )]),
                sockets: BTreeMap::from([(
                    "s1".into(),
                    SocketState {
                        resource: ResourceRef::Listener { key: "udp".into() },
                        configuration: None,
                        generation: Sequence::new(1).unwrap(),
                        network: SocketNetwork::Udp,
                        bound_address: "0.0.0.0:23456".parse().unwrap(),
                        endpoints: vec![],
                        local_endpoints: Some(
                            addresses
                                .iter()
                                .map(|address| LocalEndpoint {
                                    address: address.parse().unwrap(),
                                    source: LocalEndpointSource::Local,
                                })
                                .collect(),
                        ),
                    },
                )]),
                links: BTreeMap::new(),
                paths: BTreeMap::new(),
                selection: SelectionState::default(),
                probes: vec![],
                problems: vec![],
            },
        };
        self.app
            .node_state_service()
            .unwrap()
            .accept_report(self.tokens[index].clone(), report)
            .await
            .unwrap();
    }

    /// A report carrying only the exit capability of the current target.
    pub async fn report_exit(
        &self,
        index: usize,
        sequence: u64,
        status: meshway_model::exit::ExitStatus,
    ) {
        use meshway_model::exit::{ExitCapability, ExitFamily};
        let now = chrono::Utc::now();
        let report = StateReport {
            instance_id: self.instances[index].instance_id,
            sequence: Sequence::new(sequence).unwrap(),
            observed_at: now,
            state: NodeState {
                system: SystemState {
                    hostname: "netmap-fixture".into(),
                    os: "linux".into(),
                    arch: "aarch64".into(),
                    agent_version: "test".into(),
                },
                capabilities: caps(),
                configuration: ConfigurationState::default(),
                exit_capability: Some(ExitCapability {
                    instance_id: self.instances[index].instance_id,
                    target: self.applied[index].clone(),
                    supported_families: vec![ExitFamily::Ipv4],
                    installed_families: vec![],
                    status,
                    observed_at: now,
                }),
                listeners: BTreeMap::new(),
                sockets: BTreeMap::new(),
                links: BTreeMap::new(),
                paths: BTreeMap::new(),
                selection: SelectionState::default(),
                probes: vec![],
                problems: vec![],
            },
        };
        self.app
            .node_state_service()
            .unwrap()
            .accept_report(self.tokens[index].clone(), report)
            .await
            .unwrap();
    }
}
