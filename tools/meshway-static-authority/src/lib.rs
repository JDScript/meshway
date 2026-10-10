//! Static, netmap-compiler-backed issuer for process-local benchmarks and
//! fixtures. Never assemble this crate into a production Node. No storage,
//! Server, socket, renewal or authority bypass is provided. Dropping the
//! issuer revokes all clones.
#![forbid(unsafe_code)]

pub mod fixture;
pub mod public_fixture;

use ed25519_dalek::SigningKey;
use meshway_authority::{
    Clock, DataPlaneIdentity, Deadline, ExecutionSnapshot, NetmapIssue, Started,
};
use meshway_configuration::{NetworkConfiguration, execution::TrustedNode, netmap::CompiledNetmap};
use meshway_model::{state::Capability, *};
use meshway_policy::{
    SubjectContext,
    compile::netmap::{NetmapInput, compile_netmap},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;
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

struct Grant {
    identity: DataPlaneIdentity,
    validity: Arc<AtomicBool>,
    invalidated: CancellationToken,
    _lineage: Arc<()>,
}

pub struct StaticAuthority {
    netmaps: BTreeMap<NodeId, Arc<CompiledNetmap>>,
    grants: BTreeMap<NodeId, Grant>,
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
    /// Compile one immutable input and issue every Node's netmap with one
    /// lease of `lifetime`, measured from the same start.
    pub fn compile(
        network: NetworkId,
        configuration: &NetworkConfiguration,
        subjects: &SubjectContext,
        materials: BTreeMap<NodeId, NodeMaterial>,
        lifetime: Duration,
    ) -> Result<Self, Error> {
        let inventory = materials
            .iter()
            .map(|(id, material)| (*id, material.trusted.clone()))
            .collect();
        let netmaps = compile_netmaps(network, configuration, subjects, &inventory)?;
        let issued = meshway_authority::now()?;
        let expires = issued
            .checked_add(u64::try_from(lifetime.as_millis())?)
            .ok_or("deadline overflow")?;
        Self::issue(netmaps, materials, issued, expires)
    }

    /// Issue `netmaps` to `materials` under one lease `[issued, expires]`
    /// (wall milliseconds), shared by every isolated process of a fixture.
    pub(crate) fn issue(
        netmaps: BTreeMap<NodeId, Arc<CompiledNetmap>>,
        materials: BTreeMap<NodeId, NodeMaterial>,
        issued: u64,
        expires: u64,
    ) -> Result<Self, Error> {
        let clock = Arc::new(Clock::new()?);
        let deadline = Deadline::from_response(Started::now(&clock)?, expires)?;
        let mut grants = BTreeMap::new();
        for (id, material) in materials {
            let netmap = netmaps.get(&id).ok_or("missing compiled node")?.clone();
            let validity = Arc::new(AtomicBool::new(true));
            let invalidated = CancellationToken::new();
            let lineage = Arc::new(());
            let lease = NetmapLease::new(netmap.reference(), issued, expires)
                .map_err(|_| "invalid fixture lease")?;
            let (snapshot, _) = ExecutionSnapshot::issue_netmap(NetmapIssue {
                netmap,
                lease,
                deadline: deadline.clone(),
                instance: None,
                validity: validity.clone(),
                invalidated: invalidated.clone(),
                clock: clock.clone(),
                lineage: Arc::downgrade(&lineage),
            })?;
            let identity = DataPlaneIdentity::new(
                snapshot,
                &meshway_authority::DataPlaneKeys::new(material.signing, material.noise),
            )?;
            grants.insert(
                id,
                Grant {
                    identity,
                    validity,
                    invalidated,
                    _lineage: lineage,
                },
            );
        }
        Ok(Self { netmaps, grants })
    }

    pub fn netmap(&self, node: NodeId) -> Result<&CompiledNetmap, Error> {
        Ok(self.netmaps.get(&node).ok_or("unknown static node")?)
    }

    pub fn identity(&self, node: NodeId) -> Result<DataPlaneIdentity, Error> {
        let grant = self.grants.get(&node).ok_or("unknown static node")?;
        grant.identity.snapshot().ensure_current()?;
        Ok(grant.identity.clone())
    }

    pub fn snapshot(&self, node: NodeId) -> Result<ExecutionSnapshot, Error> {
        Ok(self.identity(node)?.snapshot().clone())
    }

    /// Irreversible; every previously issued clone and queued output is affected.
    pub fn revoke(&self, node: NodeId) -> Result<(), Error> {
        let grant = self.grants.get(&node).ok_or("unknown static node")?;
        grant.validity.store(false, Ordering::Release);
        grant.invalidated.cancel();
        Ok(())
    }

    /// Block until this grant's lease ends. Does not move clocks, extend
    /// authority or replace expiration with synthetic configuration.
    pub fn wait_expired(&self, node: NodeId) -> Result<(), Error> {
        let grant = self.grants.get(&node).ok_or("unknown static node")?;
        std::thread::sleep(
            grant
                .identity
                .snapshot()
                .expires_at()
                .saturating_duration_since(std::time::Instant::now()),
        );
        grant
            .identity
            .snapshot()
            .ensure_current()
            .err()
            .ok_or("deadline remains live")?;
        Ok(())
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

impl Drop for StaticAuthority {
    fn drop(&mut self) {
        for grant in self.grants.values() {
            grant.validity.store(false, Ordering::Release);
            grant.invalidated.cancel();
        }
    }
}
