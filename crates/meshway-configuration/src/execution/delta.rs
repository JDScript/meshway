use super::*;
use crate::{SerializationError, digest};
use meshway_model::ConfigurationRef;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DeltaError {
    #[error("inconsistent configuration delta")]
    InvalidInput,
    #[error(transparent)]
    Serialization(#[from] SerializationError),
}
macro_rules! map_delta {
    ($name:ident,$key:ty,$value:ty) => {
        object! {
        /// Server-computed keyed changes between two complete execution targets.
        pub struct $name {
            /// Keys present in the base that must be removed from the reconstructed target.
            pub remove:Vec<$key>,
            /// Complete values to insert or replace; keys cannot also occur in remove.
            pub upsert:BTreeMap<$key,$value>
        } }
    };
}
map_delta!(ListenerDelta, String, PreparedListener);
map_delta!(DialerDelta, String, DialCandidate);
map_delta!(PeerDelta, NodeId, PeerConfiguration);
map_delta!(PermitDelta, RequestId, PathPermit);
map_delta!(LocalEgressDelta, RequestId, LocalEgressPermit);
map_delta!(
    DiagnosticDelta,
    meshway_model::ObservationId,
    DiagnosticPermit
);
map_delta!(DecisionDelta, RequestId, DemandDecision);
object! {
/// Server-computed execution delta reconstructed in a temporary complete target and verified against its digest before installation.
pub struct NodeConfigurationDelta {
    /// Network shared by the base and target configurations.
    pub network_id:NetworkId,
    /// Only Node to which this delta applies.
    pub node_id:NodeId,
    /// Exact saved configuration required as the reconstruction base.
    pub base:ConfigurationRef,
    /// Exact reconstructed target reference, including the digest to verify.
    pub target:ConfigurationRef,
    /// Replacement digest of the target's trusted compilation dependencies.
    pub dependency_digest:Digest,
    /// Target's configured business execution enablement.
    pub enabled:bool,
    /// Target's configured canonical hostname.
    pub hostname:String,
    /// Target's complete User or Tag authorization identity.
    pub identity:NodeIdentity,
    /// Target's complete configured Mesh address assignment.
    pub addresses:NodeAddresses,
    /// Target's trusted local public key bindings and authenticated capabilities.
    pub self_keys:TrustedNode,
    /// Target's configured exit eligibility, gated by Node enablement.
    pub exit_enabled:bool,
    /// Complete replacement DNS execution content.
    pub dns:DnsExecution,
    /// Complete replacement Mesh capture pools, without granting business access.
    pub mesh_address_pools:AddressPools,
    /// Complete external packet ownership replacement.
    pub egress_capture:EgressCapturePlan,
    /// Complete exit role replacement; absent withdraws the role.
    pub exit_role:Option<ExitRolePlan>,
    /// Local listener removals and complete replacements.
    pub listeners:ListenerDelta,
    /// Outbound candidate removals and complete replacements.
    pub dialers:DialerDelta,
    /// Trusted peer removals and complete replacements.
    pub peers:PeerDelta,
    /// Business path permit removals and complete replacements.
    pub permits:PermitDelta,
    /// Source demand decision removals and complete replacements.
    pub decisions:DecisionDelta,
    /// Source-only local egress permit removals and complete replacements.
    pub local_egress:LocalEgressDelta,
    /// Independent diagnostic permit removals and complete replacements.
    pub diagnostics:DiagnosticDelta,
} }
fn difference<K: Clone + Ord, V: Clone + PartialEq>(
    old: &BTreeMap<K, V>,
    new: &BTreeMap<K, V>,
) -> (Vec<K>, BTreeMap<K, V>) {
    (
        old.keys()
            .filter(|k| !new.contains_key(k))
            .cloned()
            .collect(),
        new.iter()
            .filter(|(k, v)| old.get(k) != Some(v))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    )
}
pub fn configuration_delta(
    base: &CompiledNode,
    target: &CompiledNode,
) -> Result<NodeConfigurationDelta, DeltaError> {
    let b = &base.configuration;
    let t = &target.configuration;
    if b.node_id != t.node_id
        || b.network_id != t.network_id
        || digest(b)? != base.digest
        || digest(t)? != target.digest
    {
        return Err(DeltaError::InvalidInput);
    }
    let (lr, lu) = difference(&b.listeners, &t.listeners);
    let (dr, du) = difference(&b.dialers, &t.dialers);
    let (pr, pu) = difference(&b.peers, &t.peers);
    let (ar, au) = difference(&b.permits, &t.permits);
    let (er, eu) = difference(&b.local_egress, &t.local_egress);
    let (rr, ru) = difference(&b.decisions, &t.decisions);
    let (xr, xu) = difference(&b.diagnostics, &t.diagnostics);
    Ok(NodeConfigurationDelta {
        network_id: t.network_id,
        node_id: t.node_id,
        base: ConfigurationRef {
            revision: b.revision,
            execution_publication: b.publication,
            digest: base.digest,
        },
        target: ConfigurationRef {
            revision: t.revision,
            execution_publication: t.publication,
            digest: target.digest,
        },
        dependency_digest: t.dependency_digest,
        enabled: t.enabled,
        hostname: t.hostname.clone(),
        identity: t.identity.clone(),
        addresses: t.addresses.clone(),
        mesh_address_pools: t.mesh_address_pools.clone(),
        egress_capture: t.egress_capture.clone(),
        exit_role: t.exit_role.clone(),
        self_keys: t.self_keys.clone(),
        exit_enabled: t.exit_enabled,
        dns: t.dns.clone(),
        listeners: ListenerDelta {
            remove: lr,
            upsert: lu,
        },
        dialers: DialerDelta {
            remove: dr,
            upsert: du,
        },
        peers: PeerDelta {
            remove: pr,
            upsert: pu,
        },
        permits: PermitDelta {
            remove: ar,
            upsert: au,
        },
        local_egress: LocalEgressDelta {
            remove: er,
            upsert: eu,
        },
        decisions: DecisionDelta {
            remove: rr,
            upsert: ru,
        },
        diagnostics: DiagnosticDelta {
            remove: xr,
            upsert: xu,
        },
    })
}
fn apply<K: Ord, V>(
    map: &mut BTreeMap<K, V>,
    remove: Vec<K>,
    upsert: BTreeMap<K, V>,
) -> Result<(), DeltaError> {
    for key in remove {
        if map.remove(&key).is_none() || upsert.contains_key(&key) {
            return Err(DeltaError::InvalidInput);
        }
    }
    map.extend(upsert);
    Ok(())
}
/// Reconstruct only in a temporary owned target; failures leave the caller's
/// saved/running base intact. Digest verification is required before installation.
pub fn apply_configuration_delta(
    base: &CompiledNode,
    delta: NodeConfigurationDelta,
) -> Result<CompiledNode, DeltaError> {
    let b = &base.configuration;
    if delta.node_id != b.node_id
        || delta.network_id != b.network_id
        || delta.base.digest != base.digest
        || delta.base.revision != b.revision
        || delta.base.execution_publication != b.publication
        || digest(b)? != base.digest
    {
        return Err(DeltaError::InvalidInput);
    }
    let mut t = b.clone();
    t.revision = delta.target.revision;
    t.publication = delta.target.execution_publication;
    t.dependency_digest = delta.dependency_digest;
    t.enabled = delta.enabled;
    t.hostname = delta.hostname;
    t.identity = delta.identity;
    t.addresses = delta.addresses;
    t.mesh_address_pools = delta.mesh_address_pools;
    t.egress_capture = delta.egress_capture;
    t.exit_role = delta.exit_role;
    t.self_keys = delta.self_keys;
    t.exit_enabled = delta.exit_enabled;
    t.dns = delta.dns;
    apply(
        &mut t.listeners,
        delta.listeners.remove,
        delta.listeners.upsert,
    )?;
    apply(&mut t.dialers, delta.dialers.remove, delta.dialers.upsert)?;
    apply(&mut t.peers, delta.peers.remove, delta.peers.upsert)?;
    apply(&mut t.permits, delta.permits.remove, delta.permits.upsert)?;
    apply(
        &mut t.local_egress,
        delta.local_egress.remove,
        delta.local_egress.upsert,
    )?;
    apply(
        &mut t.diagnostics,
        delta.diagnostics.remove,
        delta.diagnostics.upsert,
    )?;
    apply(
        &mut t.decisions,
        delta.decisions.remove,
        delta.decisions.upsert,
    )?;
    let digest = digest(&t)?;
    if digest != delta.target.digest {
        return Err(DeltaError::InvalidInput);
    }
    Ok(CompiledNode {
        digest,
        configuration: t,
    })
}
