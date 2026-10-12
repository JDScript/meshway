//! Static, netmap-compiler-backed desired content for process-local
//! benchmarks and fixtures. Never assemble this crate into a production Node.
//! No storage, Server, socket, lease or Engine is provided: a fixture hands
//! components the content a real Engine would apply.
#![forbid(unsafe_code)]

pub mod fixture;
pub mod public_fixture;

use ed25519_dalek::SigningKey;
use meshway_configuration::{NetworkConfiguration, execution::TrustedNode, netmap::CompiledNetmap};
use meshway_engine::DataPlaneKeys;
use meshway_engine::Desired;
use meshway_model::{state::Capability, *};
use meshway_policy::{
    SubjectContext,
    compile::netmap::{NetmapInput, compile_netmap},
};
use std::{collections::BTreeMap, sync::Arc};
use zeroize::Zeroizing;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Process-private material, paired with its exact compiler inventory.
/// Debug and serialization are deliberately absent.
pub struct NodeMaterial {
    signing: SigningKey,
    noise: Zeroizing<[u8; 32]>,
    trusted: TrustedNode,
}

pub fn random_uuid() -> Result<uuid::Uuid, Error> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| "OS randomness unavailable")?;
    Ok(uuid::Builder::from_random_bytes(bytes).into_uuid())
}

impl NodeMaterial {
    /// Reconstruct only this process's private material against stable public IDs.
    pub fn from_seeds(
        signing_seed: [u8; 32],
        noise_seed: [u8; 32],
        signing_id: KeyId,
        noise_id: KeyId,
        capabilities: BTreeMap<String, Capability>,
    ) -> Self {
        let seed = Zeroizing::new(signing_seed);
        let signing = SigningKey::from_bytes(&seed);
        let noise = Zeroizing::new(noise_seed);
        let key = |id, algorithm, bytes| BoundNodeKey {
            id,
            generation: Version::MIN,
            public_key: NodePublicKey {
                algorithm,
                bytes: Digest::from_bytes(bytes),
            },
        };
        let trusted = TrustedNode {
            control_key: key(
                signing_id,
                KeyAlgorithm::Ed25519,
                signing.verifying_key().to_bytes(),
            ),
            dataplane_key: key(
                noise_id,
                KeyAlgorithm::X25519,
                curve25519_dalek::montgomery::MontgomeryPoint::mul_base_clamped(*noise).to_bytes(),
            ),
            capabilities,
        };
        Self {
            signing,
            noise,
            trusted,
        }
    }

    pub fn generate(capabilities: BTreeMap<String, Capability>) -> Result<Self, Error> {
        let mut seed = Zeroizing::new([0; 32]);
        getrandom::fill(&mut seed[..]).map_err(|_| "OS randomness unavailable")?;
        let mut noise = Zeroizing::new([0; 32]);
        getrandom::fill(&mut noise[..]).map_err(|_| "OS randomness unavailable")?;
        Ok(Self::from_seeds(
            *seed,
            *noise,
            KeyId::new(random_uuid()?)?,
            KeyId::new(random_uuid()?)?,
            capabilities,
        ))
    }

    pub fn trusted(&self) -> &TrustedNode {
        &self.trusted
    }
}

pub struct StaticAuthority {
    netmaps: BTreeMap<NodeId, Arc<CompiledNetmap>>,
    keys: BTreeMap<NodeId, DataPlaneKeys>,
}

/// Compile every Node's netmap from one immutable input with the production
/// netmap compiler.
pub fn compile_netmaps(
    network: NetworkId,
    configuration: &NetworkConfiguration,
    subjects: &SubjectContext,
    inventory: &BTreeMap<NodeId, TrustedNode>,
) -> Result<BTreeMap<NodeId, Arc<CompiledNetmap>>, Error> {
    let compiled = compile_netmap(NetmapInput {
        network,
        revision: Revision::MIN,
        configuration,
        subjects,
        inventory,
        rule_sets: &BTreeMap::new(),
        selected_profiles: &BTreeMap::new(),
        selection_versions: &BTreeMap::new(),
        previous: &BTreeMap::new(),
    })
    .map_err(|error| format!("netmap compilation failed: {error:?}"))?;
    Ok(compiled
        .nodes
        .into_iter()
        .map(|(id, netmap)| (id, Arc::new(netmap)))
        .collect())
}

impl StaticAuthority {
    /// Compile one immutable input for every Node.
    pub fn compile(
        network: NetworkId,
        configuration: &NetworkConfiguration,
        subjects: &SubjectContext,
        materials: BTreeMap<NodeId, NodeMaterial>,
    ) -> Result<Self, Error> {
        let inventory = materials
            .iter()
            .map(|(id, material)| (*id, material.trusted.clone()))
            .collect();
        let netmaps = compile_netmaps(network, configuration, subjects, &inventory)?;
        Self::issue(netmaps, materials)
    }

    /// Pair compiled `netmaps` with the private keys of `materials`.
    pub(crate) fn issue(
        netmaps: BTreeMap<NodeId, Arc<CompiledNetmap>>,
        materials: BTreeMap<NodeId, NodeMaterial>,
    ) -> Result<Self, Error> {
        let mut keys = BTreeMap::new();
        for (id, material) in materials {
            let netmap = netmaps.get(&id).ok_or("missing compiled node")?;
            let pair = DataPlaneKeys::new(material.signing, material.noise);
            if !pair.matches(&netmap.netmap.self_.self_keys) {
                return Err("private material differs from the compiled netmap".into());
            }
            keys.insert(id, pair);
        }
        Ok(Self { netmaps, keys })
    }

    pub fn netmap(&self, node: NodeId) -> Result<&CompiledNetmap, Error> {
        Ok(self.netmaps.get(&node).ok_or("unknown static node")?)
    }

    /// The content a real Engine would apply for `node`.
    pub fn desired(&self, node: NodeId) -> Result<Desired, Error> {
        let netmap = self.netmaps.get(&node).ok_or("unknown static node")?;
        Desired::new(netmap, &Default::default()).map_err(|_| "invalid compiled netmap".into())
    }

    pub fn keys(&self, node: NodeId) -> Result<DataPlaneKeys, Error> {
        Ok(self.keys.get(&node).ok_or("unknown static node")?.clone())
    }
}

/// The netmap decision of a TCP/UDP service from `source` toward `target`
/// must be exactly `path`.
pub fn check_path(
    netmap: &CompiledNetmap,
    source: NodeId,
    destination: std::net::IpAddr,
    service: (meshway_configuration::execution::BusinessProtocol, u16),
    path: &[NodeId],
) -> Result<(), Error> {
    let port = meshway_configuration::transport::Port::new(service.1)?;
    match meshway_policy::compile::netmap::compile_local(
        &netmap.netmap,
        &Default::default(),
        source,
        destination,
        (service.0, Some(port)),
    )
    .map_err(|_| "netmap decision failed")?
    {
        meshway_policy::compile::netmap::LocalDecision::Permitted(decision)
            if decision.paths.len() == 1 && decision.paths[0].nodes == path =>
        {
            Ok(())
        }
        _ => Err("netmap did not decide the exact fixture path".into()),
    }
}
