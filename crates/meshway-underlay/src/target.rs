//! One read view over the two execution target kinds while the Node migrates:
//! the legacy per-publication `NodeConfiguration` and a verified netmap. Every
//! resource owner reads its inputs through this view; neither kind grants
//! anything without the snapshot that carries it.
use meshway_configuration::{
    NodeAddresses, NodeIdentity,
    execution::{DialCandidate, NodeConfiguration, TrustedNode},
    netmap::NodeNetmap,
};
use meshway_model::NodeId;
use meshway_transport_spec::PreparedListener;
use std::collections::BTreeMap;
#[cfg(target_os = "linux")]
use {
    meshway_authority::ExecutionSnapshot,
    meshway_configuration::{
        AddressPools,
        execution::{EgressCapturePlan, ExitNat, ExitRolePlan},
    },
    meshway_model::{Digest, InstanceId, NetworkId, exit::ExitFamily},
};

#[derive(Clone, Copy)]
pub(crate) enum View<'a> {
    Legacy(&'a NodeConfiguration),
    Netmap(&'a NodeNetmap),
}

/// Exit responsibility identity. A netmap role has no instance: re-acquiring
/// an instance never changes it. A legacy role stays bound to its instance.
#[cfg(target_os = "linux")]
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RoleIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<InstanceId>,
    pub generation: Digest,
    pub family: ExitFamily,
    pub nat: ExitNat,
}

#[cfg(target_os = "linux")]
impl From<&ExitRolePlan> for RoleIdentity {
    fn from(plan: &ExitRolePlan) -> Self {
        Self {
            instance_id: Some(plan.instance_id),
            generation: plan.generation,
            family: plan.family,
            nat: plan.nat,
        }
    }
}

#[cfg(target_os = "linux")]
impl From<&meshway_configuration::netmap::ExitRole> for RoleIdentity {
    fn from(role: &meshway_configuration::netmap::ExitRole) -> Self {
        Self {
            instance_id: None,
            generation: role.generation,
            family: role.family,
            nat: role.nat,
        }
    }
}

/// Everything about a peer that a retained descriptor toward it depends on.
#[derive(PartialEq, Eq)]
pub(crate) struct PeerBinding<'a> {
    identity: &'a NodeIdentity,
    addresses: &'a NodeAddresses,
    keys: Option<&'a TrustedNode>,
    legacy: Option<&'a meshway_configuration::execution::PeerConfiguration>,
}

impl<'a> View<'a> {
    #[cfg(target_os = "linux")]
    /// The current target of a snapshot; `None` once it is not current.
    pub fn of(snapshot: &'a ExecutionSnapshot) -> Option<Self> {
        if snapshot.is_netmap() {
            snapshot
                .netmap()
                .ok()
                .map(|netmap| Self::Netmap(&netmap.netmap))
        } else {
            snapshot.configuration().ok().map(Self::Legacy)
        }
    }

    #[cfg(target_os = "linux")]
    pub fn network_id(self) -> NetworkId {
        match self {
            Self::Legacy(cfg) => cfg.network_id,
            Self::Netmap(netmap) => netmap.network_id,
        }
    }
    pub fn node_id(self) -> NodeId {
        match self {
            Self::Legacy(cfg) => cfg.node_id,
            Self::Netmap(netmap) => netmap.node_id,
        }
    }
    #[cfg(target_os = "linux")]
    pub fn addresses(self) -> &'a NodeAddresses {
        match self {
            Self::Legacy(cfg) => &cfg.addresses,
            Self::Netmap(netmap) => &netmap.self_.addresses,
        }
    }
    #[cfg(target_os = "linux")]
    pub fn mesh_address_pools(self) -> &'a AddressPools {
        match self {
            Self::Legacy(cfg) => &cfg.mesh_address_pools,
            Self::Netmap(netmap) => &netmap.self_.mesh_address_pools,
        }
    }
    pub fn listeners(self) -> &'a BTreeMap<String, PreparedListener> {
        match self {
            Self::Legacy(cfg) => &cfg.listeners,
            Self::Netmap(netmap) => &netmap.self_.listeners,
        }
    }
    pub fn dialers(self) -> &'a BTreeMap<String, DialCandidate> {
        match self {
            Self::Legacy(cfg) => &cfg.dialers,
            Self::Netmap(netmap) => &netmap.dialers,
        }
    }
    #[cfg(target_os = "linux")]
    pub fn exit_enabled(self) -> bool {
        match self {
            Self::Legacy(cfg) => cfg.exit_enabled,
            Self::Netmap(netmap) => netmap.self_.exit_enabled,
        }
    }
    #[cfg(target_os = "linux")]
    pub fn exit_role(self) -> Option<RoleIdentity> {
        match self {
            Self::Legacy(cfg) => cfg.exit_role.as_ref().map(RoleIdentity::from),
            Self::Netmap(netmap) => netmap.exit_role.as_ref().map(RoleIdentity::from),
        }
    }
    #[cfg(target_os = "linux")]
    /// Source capture; `None` for a disabled netmap Node, which owns none.
    pub fn egress_capture(self) -> Option<&'a EgressCapturePlan> {
        match self {
            Self::Legacy(cfg) => Some(&cfg.egress_capture),
            Self::Netmap(netmap) => netmap.egress.as_ref().map(|plan| &plan.capture),
        }
    }

    /// A known peer other than this Node.
    pub fn peer(self, peer: NodeId) -> Option<PeerBinding<'a>> {
        if peer == self.node_id() {
            return None;
        }
        match self {
            Self::Legacy(cfg) => cfg.peers.get(&peer).map(|value| PeerBinding {
                identity: &value.identity,
                addresses: &value.addresses,
                keys: None,
                legacy: Some(value),
            }),
            Self::Netmap(netmap) => {
                let settings = netmap.projection.configuration.nodes.get(&peer)?;
                let keys = netmap.projection.inventory.get(&peer)?;
                Some(PeerBinding {
                    identity: &settings.identity,
                    addresses: &settings.addresses,
                    keys: Some(keys),
                    legacy: None,
                })
            }
        }
    }
}
