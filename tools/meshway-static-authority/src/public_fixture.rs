//! Public, serializable compiler inputs shared by isolated benchmark processes.
//! There are no seeds here. Each process supplies only its own NodeMaterial.
use crate::{Error, NodeMaterial, StaticAuthority, random_uuid};
use meshway_authority::{Clock, Deadline, Started};
use meshway_configuration::{
    execution::*,
    transport::{Port, PreparedListener},
    *,
};
use meshway_model::*;
use meshway_policy::{
    SubjectContext,
    compile::{CompilationInput, compile},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicNode {
    pub id: NodeId,
    pub socket: SocketAddr,
    pub trusted: TrustedNode,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicFixture {
    pub network: NetworkId,
    pub revision: Revision,
    pub publication: PublicationId,
    pub configuration: NetworkConfiguration,
    pub inventory: BTreeMap<NodeId, TrustedNode>,
    pub demands: BTreeMap<RequestId, Demand>,
    pub path: Vec<NodeId>,
    pub sockets: BTreeMap<NodeId, SocketAddr>,
    pub carriers: Vec<Carrier>,
    /// Fixed compiler clock input, not a replacement for the live authority clock.
    pub compiled_at_ms: u64,
    pub expires_at_ms: u64,
    pub publication_digest: Digest,
}

impl PublicFixture {
    /// Exact A→B or A→R→B iperf3 fixture. Input nodes are in traversal order.
    pub fn iperf(nodes: Vec<PublicNode>, lifetime: Duration) -> Result<Self, Error> {
        if !(2..=3).contains(&nodes.len()) {
            return Err("iperf fixture requires 2 or 3 nodes".into());
        }
        let path: Vec<_> = nodes.iter().map(|node| node.id).collect();
        let mut configuration = new_network(
            AddressPools {
                ipv4: vec!["100.127.0.0/24".into()],
                ipv6: vec![],
            },
            "perf.internal".into(),
        )?;
        configuration.tags_owners.insert("tag:perf".into(), vec![]);
        for (i, node) in nodes.iter().enumerate() {
            let last = i + 1 == nodes.len();
            let address = Ipv4Addr::new(
                100,
                127,
                0,
                if i == 0 {
                    1
                } else if last {
                    2
                } else {
                    3
                },
            );
            let listener = serde_json::from_value(
                serde_json::json!({"type":"native_udp", "listen":node.socket.ip().to_string(), "listenPort":node.socket.port(),
                    "endpoints":[{"host":node.socket.ip().to_string(), "port":node.socket.port()}]}),
            )?;
            configuration.nodes.insert(
                node.id,
                NodeSettings {
                    hostname: format!("n{i}"),
                    remark: String::new(),
                    identity: NodeIdentity::Tag {
                        tags: vec!["tag:perf".into()],
                    },
                    enabled: true,
                    addresses: NodeAddresses {
                        ipv4: Some(address),
                        ipv6: None,
                    },
                    listeners: BTreeMap::from([(
                        crate::fixture::LISTENER_KEY.into(),
                        PreparedListener::prepare(listener)?,
                    )]),
                    dialers: Input::Omitted,
                    exit_enabled: false,
                },
            );
        }
        let last = *path.last().ok_or("missing terminal")?;
        configuration.access = vec![AccessRule {
            remark: String::new(),
            src: vec![format!("node:{}", path[0])],
            dst: vec![format!("node:{last}")],
            ip: vec!["tcp:5201".into(), "udp:5201".into()],
            bidirectional: false,
        }];
        configuration.routing.mesh = vec![MeshRoute {
            remark: String::new(),
            src: vec![format!("node:{}", path[0])],
            dst: vec![format!("node:{last}")],
            via: path[1..path.len() - 1]
                .iter()
                .map(|id| format!("node:{id}"))
                .collect(),
        }];
        let compiled_at_ms = meshway_authority::now()?;
        let expires_at_ms = compiled_at_ms
            .checked_add(u64::try_from(lifetime.as_millis())?)
            .ok_or("fixture expiry overflow")?;
        let mut demands = BTreeMap::new();
        for protocol in [BusinessProtocol::Tcp, BusinessProtocol::Udp] {
            let id = RequestId::new(random_uuid()?)?;
            demands.insert(
                id,
                Demand {
                    id,
                    source: path[0],
                    target: BusinessTarget::Mesh { node: last },
                    protocol,
                    port: Some(Port::new(5201)?),
                    profile: "mesh".into(),
                    expires_at_ms,
                },
            );
        }
        let carriers = path
            .windows(2)
            .map(|pair| Carrier {
                dialer: pair[0],
                listener: pair[1],
                listener_key: crate::fixture::LISTENER_KEY.into(),
                protocol: TransportProtocol::NativeUdp,
            })
            .collect();
        let mut fixture = Self {
            network: NetworkId::new(random_uuid()?)?,
            revision: Revision::MIN,
            publication: PublicationId::new(random_uuid()?)?,
            configuration,
            inventory: nodes
                .iter()
                .map(|node| (node.id, node.trusted.clone()))
                .collect(),
            sockets: nodes.iter().map(|node| (node.id, node.socket)).collect(),
            demands,
            path,
            carriers,
            compiled_at_ms,
            expires_at_ms,
            publication_digest: Digest::from_bytes([0; 32]),
        };
        fixture.publication_digest = Self::digest(&fixture.compile()?)?;
        Ok(fixture)
    }

    /// The tagged fixture has no user subjects. Actual compiler and policy decide
    /// every permit; no compiled node configuration is synthesized here.
    pub fn compile(&self) -> Result<CompiledPublication, Error> {
        let publication = compile(CompilationInput {
            network: self.network,
            revision: self.revision,
            publication: self.publication,
            configuration: &self.configuration,
            subjects: &SubjectContext::default(),
            inventory: &self.inventory,
            demands: &self.demands,
            dns: &[],
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            now_ms: self.compiled_at_ms,
        })?;
        Ok(publication)
    }

    pub fn digest(publication: &CompiledPublication) -> Result<Digest, Error> {
        Ok(Digest::from_bytes(
            Sha256::digest(serde_json::to_vec(publication)?).into(),
        ))
    }

    pub fn issue(&self, node: NodeId, material: NodeMaterial) -> Result<StaticAuthority, Error> {
        if self.inventory.get(&node) != Some(material.trusted()) {
            return Err("private material differs from public fixture inventory".into());
        }
        let publication = self.compile()?;
        if Self::digest(&publication)? != self.publication_digest {
            return Err("compiled publication digest differs from public fixture".into());
        }
        for demand in self.demands.keys() {
            for id in &self.path {
                let permit = publication
                    .nodes
                    .get(id)
                    .and_then(|node| node.configuration.permits.get(demand))
                    .ok_or("compiler did not permit fixture demand")?;
                if permit.paths.len() != 1 || permit.paths[0].nodes != self.path {
                    return Err("compiler did not produce exact fixture path".into());
                }
            }
        }
        let clock = Arc::new(Clock::new()?);
        let deadline = Deadline::from_response(Started::now(&clock)?, self.expires_at_ms)?;
        StaticAuthority::issue_publication(
            publication,
            BTreeMap::from([(node, material)]),
            clock,
            [deadline.clone(), deadline.clone(), deadline],
        )
    }

    pub fn carriers_for(&self, node: NodeId) -> Result<BTreeMap<NodeId, Carrier>, Error> {
        let index = self
            .path
            .iter()
            .position(|id| *id == node)
            .ok_or("unknown fixture node")?;
        if self.carriers.len() + 1 != self.path.len() {
            return Err("fixture carrier count differs from path".into());
        }
        let mut result = BTreeMap::new();
        if index > 0 {
            result.insert(self.path[index - 1], self.carriers[index - 1].clone());
        }
        if index + 1 < self.path.len() {
            result.insert(self.path[index + 1], self.carriers[index].clone());
        }
        Ok(result)
    }
}
