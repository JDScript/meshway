//! Static, compiler-backed issuer for process-local benchmarks and fixtures.
//! Never assemble this crate into a production Node. No storage, Server, socket,
//! renewal or authority bypass is provided. Dropping the issuer revokes all clones.
#![forbid(unsafe_code)]

pub mod fixture;

use ed25519_dalek::SigningKey;
use meshway_authority::{Clock, DataPlaneIdentity, Deadline, ExecutionSnapshot, Issue, Started};
use meshway_configuration::{NetworkConfiguration, execution::*};
use meshway_model::{state::Capability, *};
use meshway_policy::{
    SubjectContext,
    compile::{CompilationInput, compile},
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

/// Random process-private material, paired with its exact compiler inventory.
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
    pub fn generate(capabilities: BTreeMap<String, Capability>) -> Result<Self, Error> {
        let mut seed = Zeroizing::new([0; 32]);
        getrandom::fill(&mut seed[..]).map_err(|_| "OS randomness unavailable")?;
        let signing = SigningKey::from_bytes(&seed);
        let mut noise = Zeroizing::new([0; 32]);
        getrandom::fill(&mut noise[..]).map_err(|_| "OS randomness unavailable")?;
        let key = |algorithm, bytes| -> Result<BoundNodeKey, Error> {
            Ok(BoundNodeKey {
                id: KeyId::new(random_uuid()?)?,
                generation: Version::MIN,
                public_key: NodePublicKey {
                    algorithm,
                    bytes: Digest::from_bytes(bytes),
                },
            })
        };
        let trusted = TrustedNode {
            control_key: key(KeyAlgorithm::Ed25519, signing.verifying_key().to_bytes())?,
            dataplane_key: key(
                KeyAlgorithm::X25519,
                curve25519_dalek::montgomery::MontgomeryPoint::mul_base_clamped(*noise).to_bytes(),
            )?,
            capabilities,
        };
        Ok(Self {
            signing,
            noise,
            trusted,
        })
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
    publication: CompiledPublication,
    grants: BTreeMap<NodeId, Grant>,
}

impl StaticAuthority {
    /// Compile one immutable input and issue session/instance/execution deadlines
    /// measured from the same start. The earliest duration bounds every consumer.
    pub fn compile(
        network: NetworkId,
        configuration: &NetworkConfiguration,
        subjects: &SubjectContext,
        materials: BTreeMap<NodeId, NodeMaterial>,
        demands: &BTreeMap<RequestId, Demand>,
        lifetimes: [Duration; 3],
    ) -> Result<Self, Error> {
        let clock = Arc::new(Clock::new()?);
        let started = Started::now(&clock)?;
        let mut deadlines = Vec::new();
        for duration in lifetimes {
            let expiry = started
                .wall()
                .checked_add(u64::try_from(duration.as_millis())?)
                .ok_or("deadline overflow")?;
            deadlines.push(Deadline::from_response(started.clone(), expiry)?);
        }
        let deadlines: [Deadline; 3] = deadlines
            .try_into()
            .map_err(|_| "three deadlines required")?;
        let inventory = materials
            .iter()
            .map(|(id, material)| (*id, material.trusted.clone()))
            .collect();
        let publication = compile(CompilationInput {
            network,
            revision: Revision::MIN,
            publication: PublicationId::new(random_uuid()?)?,
            configuration,
            subjects,
            inventory: &inventory,
            demands,
            dns: &[],
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            now_ms: started.wall(),
        })?;
        let mut grants = BTreeMap::new();
        for (id, material) in materials {
            let target = publication.nodes.get(&id).ok_or("missing compiled node")?;
            let validity = Arc::new(AtomicBool::new(true));
            let invalidated = CancellationToken::new();
            let lineage = Arc::new(());
            let snapshot = ExecutionSnapshot::issue(Issue {
                target: Arc::new(target.clone()),
                instance: InstanceId::new(random_uuid()?)?,
                generation: Version::MIN,
                validity: validity.clone(),
                invalidated: invalidated.clone(),
                clock: clock.clone(),
                deadlines: deadlines.clone(),
                lineage: Arc::downgrade(&lineage),
                publication: Arc::new(()),
                predecessor: None,
            })?;
            let identity = DataPlaneIdentity::new(snapshot, material.signing, material.noise)?;
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
        Ok(Self {
            publication,
            grants,
        })
    }

    pub fn publication(&self) -> &CompiledPublication {
        &self.publication
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

    /// Block until this grant's original earliest deadline. Does not move clocks,
    /// extend authority or replace expiration with synthetic configuration.
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

impl Drop for StaticAuthority {
    fn drop(&mut self) {
        for grant in self.grants.values() {
            grant.validity.store(false, Ordering::Release);
            grant.invalidated.cancel();
        }
    }
}
