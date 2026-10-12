//! Lazy local decisions on one installed netmap, cached per content.
//!
//! Every value here is derived from the netmap with Engine's shared
//! `meshway-policy` compiler and is discarded when the content changes. A
//! cached decision never outlives its content and is never an alternative to
//! admission: callers still pass it through `meshway_policy::admission::netmap`.
//! Nothing is precomputed for absent pairs.
use meshway_configuration::{
    NodeIdentity,
    execution::{BusinessProtocol, Carrier, LogicalPath, UnavailableReason},
    netmap::CompiledNetmap,
};
use meshway_model::{BoundNodeKey, Digest, NetmapRef, NetworkId, NodeId};
use meshway_policy::compile::netmap::{
    Associations, Decision, LocalCompiler, LocalDecision, Target,
};
use meshway_transport::spec::Port;
use sha2::{Digest as _, Sha256};
use std::{collections::BTreeMap, net::IpAddr, sync::Arc};

/// Upper bound of each cache; reaching it starts the cache over. Every miss
/// is one bounded compilation, so a flood of distinct pairs costs CPU, never memory.
const MAX_ENTRIES: usize = 4096;

/// Circuit purpose: business packets or path diagnostics (no IP payload).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Purpose {
    Business,
    Diagnostic,
    Dns,
}

/// Outcome of a local decision for one (source, destination, service).
#[derive(Clone, Debug)]
pub(crate) enum Outcome {
    Permitted(Arc<Decision>),
    Direct,
    Unmanaged,
    Denied(UnavailableReason),
}

#[derive(Default)]
pub(crate) struct Decisions {
    content: Option<NetmapRef>,
    compiler: Option<Arc<LocalCompiler>>,
    /// Trusted DNS associations of the desired state; empty for other content.
    associations: Arc<Associations>,
    dns: BTreeMap<NodeId, Arc<meshway_configuration::netmap::DnsService>>,
    /// Permitted decisions carry the pair's complete service set.
    permitted: BTreeMap<(NodeId, IpAddr), Arc<Decision>>,
    /// Non-permitted outcomes are exact to the service.
    other: BTreeMap<(NodeId, IpAddr, u8, Option<u16>), Outcome>,
    transit: BTreeMap<(NodeId, Target), Arc<Vec<LogicalPath>>>,
    egress: BTreeMap<NodeId, Digest>,
    adjacency: Vec<(NodeId, Carrier, bool)>,
}

fn protocol(value: BusinessProtocol) -> u8 {
    match value {
        BusinessProtocol::Tcp => 6,
        BusinessProtocol::Udp => 17,
        BusinessProtocol::IcmpEcho => 1,
    }
}

impl Decisions {
    pub fn dns_service(
        &mut self,
        netmap: &CompiledNetmap,
        source: NodeId,
    ) -> Result<Arc<meshway_configuration::netmap::DnsService>, crate::Error> {
        self.bind(netmap);
        if let Some(service) = self.dns.get(&source) {
            return Ok(service.clone());
        }
        let service = self
            .compiler
            .as_ref()
            .expect("bound compiler")
            .dns_service(source)
            .map_err(|_| {
                crate::Error::Admission(meshway_policy::admission::AdmissionError::Configuration)
            })?
            .ok_or(crate::Error::Admission(
                meshway_policy::admission::AdmissionError::Permit,
            ))?;
        if self.dns.len() >= MAX_ENTRIES {
            self.dns.clear();
        }
        let service = Arc::new(service);
        self.dns.insert(source, service.clone());
        Ok(service)
    }
    /// Empty caches over Engine's one shared compiler of `desired`.
    pub fn for_desired(desired: &meshway_engine::Desired) -> Self {
        Self {
            content: Some(desired.reference()),
            compiler: Some(desired.compiler().clone()),
            associations: desired.associations().clone(),
            ..Default::default()
        }
    }

    /// The compiler judging `netmap` (binding to it first).
    #[cfg(test)]
    pub(crate) fn compiler_for(&mut self, netmap: &CompiledNetmap) -> Arc<LocalCompiler> {
        self.bind(netmap);
        self.compiler.clone().expect("bound compiler")
    }

    /// Callers pass the content these caches belong to; another content
    /// starts over on its own compiler.
    pub fn bind(&mut self, netmap: &CompiledNetmap) {
        let reference = netmap.reference();
        if self.content != Some(reference) {
            *self = Self {
                content: Some(reference),
                // Content other than the desired one is judged without
                // artifacts: rules naming a rule set are unavailable.
                compiler: Some(Arc::new(LocalCompiler::from_shared(
                    netmap.netmap.clone(),
                    &Default::default(),
                ))),
                ..Default::default()
            };
        }
    }

    pub fn decide(
        &mut self,
        netmap: &CompiledNetmap,
        source: NodeId,
        destination: IpAddr,
        service: (BusinessProtocol, Option<Port>),
    ) -> Result<Outcome, crate::Error> {
        self.bind(netmap);
        let destination = destination.to_canonical();
        if let Some(decision) = self.permitted.get(&(source, destination)) {
            return Ok(if decision.services.contains(service.0, service.1) {
                Outcome::Permitted(decision.clone())
            } else {
                Outcome::Denied(UnavailableReason::Unauthorized)
            });
        }
        let key = (
            source,
            destination,
            protocol(service.0),
            service.1.map(Port::get),
        );
        if let Some(outcome) = self.other.get(&key) {
            return Ok(outcome.clone());
        }
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("policy.netmap_decide");
        let _profile = PROFILE.enter();
        let decided = self
            .compiler
            .as_ref()
            .expect("bound compiler")
            .decide_with(
                source,
                destination,
                service.0,
                service.1,
                &self.associations,
            )
            .map_err(|_| {
                crate::Error::Admission(meshway_policy::admission::AdmissionError::Configuration)
            })?;
        let outcome = match decided {
            LocalDecision::Permitted(decision) => {
                let decision = Arc::new(decision);
                if self.permitted.len() >= MAX_ENTRIES {
                    self.permitted.clear();
                }
                self.permitted
                    .insert((source, destination), decision.clone());
                return Ok(Outcome::Permitted(decision));
            }
            LocalDecision::Direct => Outcome::Direct,
            LocalDecision::Unmanaged => Outcome::Unmanaged,
            LocalDecision::Denied(reason) => Outcome::Denied(reason),
        };
        if self.other.len() >= MAX_ENTRIES {
            self.other.clear();
        }
        self.other.insert(key, outcome.clone());
        Ok(outcome)
    }

    /// Every path of `origin` toward `target` this content admits for some
    /// service, for circuit-level checks at endpoints and relays.
    pub fn transit(
        &mut self,
        netmap: &CompiledNetmap,
        origin: NodeId,
        target: &Target,
    ) -> Result<Arc<Vec<LogicalPath>>, crate::Error> {
        self.bind(netmap);
        if let Some(paths) = self.transit.get(&(origin, target.clone())) {
            return Ok(paths.clone());
        }
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("policy.netmap_transit");
        let _profile = PROFILE.enter();
        let paths = Arc::new(
            self.compiler
                .as_ref()
                .expect("bound compiler")
                .transit(origin, target)
                .map_err(|_| {
                    crate::Error::Admission(
                        meshway_policy::admission::AdmissionError::Configuration,
                    )
                })?,
        );
        if self.transit.len() >= MAX_ENTRIES {
            self.transit.clear();
        }
        self.transit.insert((origin, target.clone()), paths.clone());
        Ok(paths)
    }

    /// The egress selection identity of a visible source, as its own netmap has it.
    pub fn selection(
        &mut self,
        netmap: &CompiledNetmap,
        source: NodeId,
    ) -> Result<Digest, crate::Error> {
        self.bind(netmap);
        if let Some(selection) = self.egress.get(&source) {
            return Ok(*selection);
        }
        let selection = if source == netmap.netmap.node_id {
            netmap.netmap.egress.as_ref().map(|plan| plan.generation)
        } else {
            self.compiler
                .as_ref()
                .expect("bound compiler")
                .egress(source)
                .ok()
                .map(|plan| plan.generation)
        }
        .ok_or(crate::Error::Admission(
            meshway_policy::admission::AdmissionError::Permit,
        ))?;
        if self.egress.len() >= MAX_ENTRIES {
            self.egress.clear();
        }
        self.egress.insert(source, selection);
        Ok(selection)
    }

    /// `admit_adjacency` on this content for one authenticated carrier.
    pub fn adjacency(&mut self, netmap: &CompiledNetmap, peer: NodeId, carrier: &Carrier) -> bool {
        self.bind(netmap);
        if let Some((_, _, admitted)) = self
            .adjacency
            .iter()
            .find(|(node, known, _)| *node == peer && known == carrier)
        {
            return *admitted;
        }
        let admitted = meshway_policy::admission::netmap::admit_adjacency(
            self.compiler.as_ref().expect("bound compiler"),
            &netmap.netmap,
            peer,
            carrier,
        )
        .is_ok();
        if self.adjacency.len() >= 1024 {
            self.adjacency.clear();
        }
        self.adjacency.push((peer, carrier.clone(), admitted));
        admitted
    }
}

fn bytes(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u32).to_be_bytes());
    hash.update(value);
}

fn node(hash: &mut Sha256, id: &NodeId) {
    hash.update(id.as_uuid().as_bytes());
}

fn key(hash: &mut Sha256, key: &BoundNodeKey) {
    hash.update(key.id.as_uuid().as_bytes());
    hash.update(key.generation.get().to_be_bytes());
    bytes(hash, format!("{:?}", key.public_key.algorithm).as_bytes());
    hash.update(key.public_key.bytes.as_bytes());
}

pub(crate) fn encode_target(hash: &mut Sha256, target: &Target) {
    match target {
        Target::Mesh { node: id } => {
            hash.update([1]);
            node(hash, id);
        }
        Target::Exit { nodes, via } => {
            hash.update([2, nodes.len() as u8]);
            nodes.iter().for_each(|id| node(hash, id));
            hash.update([via.len() as u8]);
            via.iter().for_each(|id| node(hash, id));
        }
    }
}

/// Digest binding one circuit to the exact authorized path of its pair:
/// grant (origin, target, purpose), the complete ordered path with every dial
/// candidate, each path Node's identity, addresses and keys, and the source
/// egress selection of exit circuits. Service sets are deliberately absent:
/// Access port changes are enforced per packet and never force a handshake.
/// Every endpoint and relay computes it from its own projection; unequal
/// results reject the handshake, and a change across content retires the circuit.
pub(crate) fn path_authorization(
    netmap: &CompiledNetmap,
    network: NetworkId,
    purpose: Purpose,
    target: &Target,
    path: &LogicalPath,
    selection: Option<Digest>,
) -> Result<Digest, crate::secure::Error> {
    use crate::secure::Error;
    let netmap = &netmap.netmap;
    let mut hash = Sha256::new();
    hash.update(b"meshway/circuit/path-authorization/v6\0");
    hash.update(network.as_uuid().as_bytes());
    hash.update([match purpose {
        Purpose::Business => 1,
        Purpose::Diagnostic => 2,
        Purpose::Dns => 3,
    }]);
    encode_target(&mut hash, target);
    hash.update([path.nodes.len() as u8]);
    for id in &path.nodes {
        node(&mut hash, id);
        let (identity, addresses, trusted) = if *id == netmap.node_id {
            (
                &netmap.self_.identity,
                &netmap.self_.addresses,
                &netmap.self_.self_keys,
            )
        } else {
            let settings = netmap
                .projection
                .configuration
                .nodes
                .get(id)
                .ok_or(Error::Context)?;
            (
                &settings.identity,
                &settings.addresses,
                netmap.projection.inventory.get(id).ok_or(Error::Context)?,
            )
        };
        match identity {
            NodeIdentity::User { user } => {
                hash.update([1]);
                bytes(&mut hash, user.as_bytes());
            }
            NodeIdentity::Tag { tags } => {
                hash.update([2, tags.len() as u8]);
                tags.iter().for_each(|tag| bytes(&mut hash, tag.as_bytes()));
            }
        }
        bytes(
            &mut hash,
            addresses
                .ipv4
                .map(|a| a.octets().to_vec())
                .unwrap_or_default()
                .as_slice(),
        );
        bytes(
            &mut hash,
            addresses
                .ipv6
                .map(|a| a.octets().to_vec())
                .unwrap_or_default()
                .as_slice(),
        );
        key(&mut hash, &trusted.control_key);
        key(&mut hash, &trusted.dataplane_key);
    }
    hash.update((path.dial_candidates.len() as u32).to_be_bytes());
    for (dialer, keys) in &path.dial_candidates {
        node(&mut hash, dialer);
        hash.update((keys.len() as u32).to_be_bytes());
        keys.iter().for_each(|key| bytes(&mut hash, key.as_bytes()));
    }
    match selection {
        Some(selection) => {
            hash.update([1]);
            hash.update(selection.as_bytes());
        }
        None => hash.update([0]),
    }
    Ok(Digest::from_bytes(hash.finalize().into()))
}
