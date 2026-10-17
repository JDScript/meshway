//! Netmap chain on a real App: State reports, visibility, UDP coordination and
//! observations read the stored netmaps and never compile anything.
use super::*;
use crate::state::facts::NetmapReader;
use meshway_configuration::netmap::CompiledNetmap;
use meshway_model::state::*;
use meshway_protocol::state::StateReport;
use std::sync::Arc;

mod associations;
mod diagnostics;
mod facts;
mod lifecycle;
mod receipts;
mod scope;

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
    /// Each Node's current netmap, reported as applied.
    pub applied: [NetmapRef; 3],
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
        keepalive: false,
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
        tokens.push(session.token.expose().to_owned());
        instances.push(instance);
    }
    for node in [a, b, c.node_id] {
        let netmaps = app.netmaps.clone();
        applied.push(
            app.executor
                .handle()
                .read(move |tx| netmaps.current(tx, network, node))
                .await
                .unwrap()
                .unwrap()
                .reference(),
        );
    }
    Lab {
        _dir: dir,
        app,
        network,
        owner: owner.token.expose().to_owned(),
        nodes: [a, b, c.node_id],
        tokens: tokens.try_into().unwrap(),
        instances: instances.try_into().unwrap(),
        applied: applied.try_into().unwrap(),
    }
}

impl Lab {
    pub async fn netmap(&self, node: NodeId) -> Arc<CompiledNetmap> {
        let netmaps = self.app.netmaps.clone();
        let network = self.network;
        self.app
            .executor
            .handle()
            .read(move |tx| netmaps.current(tx, network, node))
            .await
            .unwrap()
            .unwrap()
    }

    /// Every Node's current stored netmap reference.
    pub async fn references(&self) -> Vec<NetmapRef> {
        let mut references = Vec::new();
        for node in self.nodes {
            references.push(self.netmap(node).await.reference());
        }
        references
    }

    /// A complete report of the `udp` listener with the given local endpoints.
    pub async fn report_endpoints(&self, index: usize, sequence: u64, addresses: &[&str]) {
        self.report_channels(index, sequence, addresses, &[]).await;
    }

    /// The same report with one authenticated channel to each of `peers` on
    /// the `udp` listener socket.
    pub async fn report_channels(
        &self,
        index: usize,
        sequence: u64,
        addresses: &[&str],
        peers: &[NodeId],
    ) {
        self.report_listener(index, sequence, addresses, peers, vec![])
            .await;
    }

    /// The same report with the `udp` listener socket's STUN observations, in
    /// the order the Node lists them.
    pub async fn report_stun(
        &self,
        index: usize,
        sequence: u64,
        candidates: Vec<meshway_model::state::EndpointCandidate>,
    ) {
        self.report_listener(index, sequence, &[], &[], candidates)
            .await;
    }

    async fn report_listener(
        &self,
        index: usize,
        sequence: u64,
        addresses: &[&str],
        peers: &[NodeId],
        stun: Vec<meshway_model::state::EndpointCandidate>,
    ) {
        let applied = self.applied[index];
        let now = chrono::Utc::now();
        let links = peers
            .iter()
            .enumerate()
            .map(|(n, peer)| {
                (
                    *peer,
                    vec![LinkState {
                        link_id: format!("link-{n}"),
                        transport: "native_udp".into(),
                        initiator_node_id: *peer,
                        socket_id: "s1".into(),
                        phase: LinkPhase::Authenticated,
                        business_dependencies: 0,
                        observation_ids: vec![],
                        observed_at: now,
                        expires_at: now + chrono::Duration::seconds(60),
                        since: None,
                        acquisition: None,
                    }],
                )
            })
            .collect();
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
                    saved: Some(applied),
                    applied: Some(applied),
                    progress: None,
                },
                exit_capability: None,
                exit_dns: None,
                listeners: BTreeMap::from([(
                    "udp".into(),
                    ListenerState {
                        phase: ListenerPhase::Listening,
                        socket_ids: vec!["s1".into()],
                        loaded_certificate_sha256: None,
                        problem: None,
                        netmap: None,
                    },
                )]),
                sockets: BTreeMap::from([(
                    "s1".into(),
                    SocketState {
                        resource: ResourceRef::Listener { key: "udp".into() },
                        netmap: None,
                        generation: Sequence::new(1).unwrap(),
                        network: SocketNetwork::Udp,
                        bound_address: "0.0.0.0:23456".parse().unwrap(),
                        endpoints: stun,
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
                links,
                adjacencies: Default::default(),
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
        use meshway_model::exit::{ExitCapability, ExitFamily, ExitFamilyCapability};
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
                    netmap: self.applied[index],
                    role_generation: self
                        .netmap(self.nodes[index])
                        .await
                        .netmap
                        .exit_role
                        .as_ref()
                        .unwrap()
                        .generation,
                    families: vec![ExitFamilyCapability {
                        family: ExitFamily::Ipv4,
                        status,
                        installed: false,
                    }],
                    observed_at: now,
                }),
                exit_dns: None,
                listeners: BTreeMap::new(),
                sockets: BTreeMap::new(),
                links: BTreeMap::new(),
                adjacencies: Default::default(),
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

    /// A report carrying only an exit resolver fact.
    pub async fn report_exit_dns(
        &self,
        index: usize,
        sequence: u64,
        exit_dns: meshway_model::exit::ExitDns,
    ) -> Result<(), crate::error::Error> {
        let report = StateReport {
            instance_id: self.instances[index].instance_id,
            sequence: Sequence::new(sequence).unwrap(),
            observed_at: chrono::Utc::now(),
            state: NodeState {
                system: SystemState {
                    hostname: "netmap-fixture".into(),
                    os: "linux".into(),
                    arch: "aarch64".into(),
                    agent_version: "test".into(),
                },
                capabilities: caps(),
                configuration: ConfigurationState::default(),
                exit_capability: None,
                exit_dns: Some(exit_dns),
                listeners: BTreeMap::new(),
                sockets: BTreeMap::new(),
                links: BTreeMap::new(),
                adjacencies: Default::default(),
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
            .map(|_| ())
    }
}
