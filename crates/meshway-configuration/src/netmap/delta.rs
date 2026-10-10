//! Keyed netmap changes. A delta is applied to a private copy of the exact
//! base and accepted only when the result reproduces the target digest.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NetmapDeltaError {
    #[error("inconsistent netmap delta")]
    InvalidInput,
    #[error(transparent)]
    Serialization(#[from] SerializationError),
}

macro_rules! keyed_delta {
    ($name:ident, $key:ty, $value:ty, $what:literal) => {
        object! {
        #[doc = concat!("Removed and replaced ", $what, " between two netmaps.")]
        pub struct $name {
            /// Base keys to remove; never also upserted.
            pub remove: Vec<$key>,
            /// Complete values to insert or replace.
            pub upsert: BTreeMap<$key, $value>,
        } }
    };
}
keyed_delta!(
    NetmapNodeDelta,
    NodeId,
    crate::NodeSettingsView,
    "visible Node declarations"
);
keyed_delta!(
    NetmapInventoryDelta,
    NodeId,
    TrustedNode,
    "visible Node inventory"
);
keyed_delta!(NetmapDialerDelta, String, DialCandidate, "dial candidates");

object! {
/// Changes from an exact base netmap to an exact target netmap of the same Node.
pub struct NetmapDelta {
    /// Network shared by base and target.
    pub network_id: NetworkId,
    /// Node owning both netmaps.
    pub node_id: NodeId,
    /// Exact base the receiver must hold.
    pub base: NetmapRef,
    /// Exact target, including the digest to verify after reconstruction.
    pub target: NetmapRef,
    /// Target dependency digest.
    pub dependency_digest: Digest,
    /// Complete replacement of the target's own resources.
    #[serde(rename = "self")]
    pub self_: NetmapSelf,
    /// Complete replacement projected configuration except `nodes`, which is always empty here.
    pub configuration: crate::ConfigurationView,
    /// Visible Node declaration changes.
    pub nodes: NetmapNodeDelta,
    /// Complete replacement user bindings.
    pub subjects: crate::SubjectContext,
    /// Visible inventory changes.
    pub inventory: NetmapInventoryDelta,
    /// Complete replacement rule-set contents.
    pub rule_sets: BTreeMap<String, crate::execution::RuleSetContent>,
    /// Complete replacement profile selections.
    pub selected_profiles: BTreeMap<NodeId, String>,
    /// Complete replacement selection versions.
    pub selection_versions: BTreeMap<NodeId, meshway_model::Version>,
    /// Dial candidate changes.
    pub dialers: NetmapDialerDelta,
    /// Complete replacement egress plan.
    pub egress: Option<EgressPlan> [required = true],
    /// Complete replacement exit role.
    pub exit_role: Option<ExitRole> [required = true],
    /// Complete replacement DNS content.
    pub dns: NetmapDns,
} }

fn difference<K: Clone + Ord, V: Clone + PartialEq>(
    old: &BTreeMap<K, V>,
    new: &BTreeMap<K, V>,
) -> (Vec<K>, BTreeMap<K, V>) {
    (
        old.keys()
            .filter(|key| !new.contains_key(key))
            .cloned()
            .collect(),
        new.iter()
            .filter(|(key, value)| old.get(key) != Some(value))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

fn apply<K: Ord, V>(
    map: &mut BTreeMap<K, V>,
    remove: Vec<K>,
    upsert: BTreeMap<K, V>,
) -> Result<(), NetmapDeltaError> {
    for key in remove {
        if upsert.contains_key(&key) || map.remove(&key).is_none() {
            return Err(NetmapDeltaError::InvalidInput);
        }
    }
    map.extend(upsert);
    Ok(())
}

/// Compute a delta between two verified netmaps of the same Node.
pub fn netmap_delta(
    base: &CompiledNetmap,
    target: &CompiledNetmap,
) -> Result<NetmapDelta, NetmapDeltaError> {
    base.verify()?;
    target.verify()?;
    let (b, t) = (&base.netmap, &target.netmap);
    if b.node_id != t.node_id || b.network_id != t.network_id || t.serial <= b.serial {
        return Err(NetmapDeltaError::InvalidInput);
    }
    let (nodes_remove, nodes_upsert) = difference(
        &b.projection.configuration.nodes,
        &t.projection.configuration.nodes,
    );
    let (inventory_remove, inventory_upsert) =
        difference(&b.projection.inventory, &t.projection.inventory);
    let (dialers_remove, dialers_upsert) = difference(&b.dialers, &t.dialers);
    let mut configuration = t.projection.configuration.clone();
    configuration.nodes.clear();
    Ok(NetmapDelta {
        network_id: t.network_id,
        node_id: t.node_id,
        base: base.reference(),
        target: target.reference(),
        dependency_digest: t.dependency_digest,
        self_: t.self_.clone(),
        configuration,
        nodes: NetmapNodeDelta {
            remove: nodes_remove,
            upsert: nodes_upsert,
        },
        subjects: t.projection.subjects.clone(),
        inventory: NetmapInventoryDelta {
            remove: inventory_remove,
            upsert: inventory_upsert,
        },
        rule_sets: t.projection.rule_sets.clone(),
        selected_profiles: t.projection.selected_profiles.clone(),
        selection_versions: t.projection.selection_versions.clone(),
        dialers: NetmapDialerDelta {
            remove: dialers_remove,
            upsert: dialers_upsert,
        },
        egress: t.egress.clone(),
        exit_role: t.exit_role.clone(),
        dns: t.dns.clone(),
    })
}

/// Reconstruct in an owned copy; any failure leaves the caller's base intact.
pub fn apply_netmap_delta(
    base: &CompiledNetmap,
    delta: NetmapDelta,
) -> Result<CompiledNetmap, NetmapDeltaError> {
    base.verify()?;
    let b = &base.netmap;
    if delta.network_id != b.network_id
        || delta.node_id != b.node_id
        || delta.base != base.reference()
        || delta.target.serial <= delta.base.serial
    {
        return Err(NetmapDeltaError::InvalidInput);
    }
    if !delta.configuration.nodes.is_empty() {
        return Err(NetmapDeltaError::InvalidInput);
    }
    let mut t = b.as_ref().clone();
    t.revision = delta.target.revision;
    t.serial = delta.target.serial;
    t.dependency_digest = delta.dependency_digest;
    t.self_ = delta.self_;
    t.egress = delta.egress;
    t.exit_role = delta.exit_role;
    t.dns = delta.dns;
    let nodes = std::mem::take(&mut t.projection.configuration.nodes);
    t.projection.configuration = delta.configuration;
    t.projection.configuration.nodes = nodes;
    t.projection.subjects = delta.subjects;
    t.projection.rule_sets = delta.rule_sets;
    t.projection.selected_profiles = delta.selected_profiles;
    t.projection.selection_versions = delta.selection_versions;
    apply(
        &mut t.projection.configuration.nodes,
        delta.nodes.remove,
        delta.nodes.upsert,
    )?;
    apply(
        &mut t.projection.inventory,
        delta.inventory.remove,
        delta.inventory.upsert,
    )?;
    apply(&mut t.dialers, delta.dialers.remove, delta.dialers.upsert)?;
    let compiled = CompiledNetmap {
        digest: t.content_digest()?,
        netmap: std::sync::Arc::new(t),
    };
    if compiled.digest != delta.target.digest {
        return Err(NetmapDeltaError::InvalidInput);
    }
    compiled.verify()?;
    Ok(compiled)
}
