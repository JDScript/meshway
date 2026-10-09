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
keyed_delta!(NetmapPeerDelta, NodeId, PeerConfiguration, "peers");
keyed_delta!(NetmapDialerDelta, String, DialCandidate, "dial candidates");
keyed_delta!(NetmapEdgeDelta, NodeId, Edges, "adjacent edges");

object! {
/// Removed and replaced grants between two netmaps.
pub struct NetmapGrantDelta {
    /// Base grant keys to remove; never also upserted.
    pub remove: Vec<GrantKey>,
    /// Complete grants to insert or replace, in increasing key order.
    #[serde(with = "grants")]
    pub upsert: BTreeMap<GrantKey, Grant> [value_type = Vec<Grant>],
} }

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
    /// Peer changes.
    pub peers: NetmapPeerDelta,
    /// Dial candidate changes.
    pub dialers: NetmapDialerDelta,
    /// Edge changes.
    pub edges: NetmapEdgeDelta,
    /// Grant changes.
    pub grants: NetmapGrantDelta,
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
    let (peers_remove, peers_upsert) = difference(&b.peers, &t.peers);
    let (dialers_remove, dialers_upsert) = difference(&b.dialers, &t.dialers);
    let (edges_remove, edges_upsert) = difference(&b.edges, &t.edges);
    let (grants_remove, grants_upsert) = difference(&b.grants, &t.grants);
    Ok(NetmapDelta {
        network_id: t.network_id,
        node_id: t.node_id,
        base: base.reference(),
        target: target.reference(),
        dependency_digest: t.dependency_digest,
        self_: t.self_.clone(),
        peers: NetmapPeerDelta {
            remove: peers_remove,
            upsert: peers_upsert,
        },
        dialers: NetmapDialerDelta {
            remove: dialers_remove,
            upsert: dialers_upsert,
        },
        edges: NetmapEdgeDelta {
            remove: edges_remove,
            upsert: edges_upsert,
        },
        grants: NetmapGrantDelta {
            remove: grants_remove,
            upsert: grants_upsert,
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
    let mut t = b.clone();
    t.revision = delta.target.revision;
    t.serial = delta.target.serial;
    t.dependency_digest = delta.dependency_digest;
    t.self_ = delta.self_;
    t.egress = delta.egress;
    t.exit_role = delta.exit_role;
    t.dns = delta.dns;
    apply(&mut t.peers, delta.peers.remove, delta.peers.upsert)?;
    apply(&mut t.dialers, delta.dialers.remove, delta.dialers.upsert)?;
    apply(&mut t.edges, delta.edges.remove, delta.edges.upsert)?;
    apply(&mut t.grants, delta.grants.remove, delta.grants.upsert)?;
    let compiled = CompiledNetmap {
        digest: t.content_digest()?,
        netmap: t,
    };
    if compiled.digest != delta.target.digest {
        return Err(NetmapDeltaError::InvalidInput);
    }
    compiled.verify()?;
    Ok(compiled)
}
