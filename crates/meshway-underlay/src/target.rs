//! One read view over the desired netmap. Every resource owner reads its
//! inputs through this view; nothing here grants anything by itself.
use meshway_configuration::{
    execution::{DialCandidate, TrustedNode},
    netmap::NodeNetmap,
};
use meshway_model::NodeId;
use meshway_transport::spec::PreparedListener;
use std::collections::BTreeMap;
#[cfg(target_os = "linux")]
use {
    meshway_configuration::{
        AddressPools,
        execution::{EgressCapturePlan, ExitNat},
    },
    meshway_model::{Digest, NetworkId, exit::ExitFamily},
};

#[derive(Clone, Copy)]
pub(crate) struct View<'a>(
    &'a NodeNetmap,
    #[cfg(target_os = "linux")] Option<&'a EgressCapturePlan>,
);

/// Exit responsibility identity. It has no instance: re-acquiring an
/// instance never changes it.
#[cfg(target_os = "linux")]
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RoleIdentity {
    pub generation: Digest,
    pub family: ExitFamily,
    pub nat: ExitNat,
}

#[cfg(target_os = "linux")]
impl From<&meshway_configuration::netmap::ExitRole> for RoleIdentity {
    fn from(role: &meshway_configuration::netmap::ExitRole) -> Self {
        Self {
            generation: role.generation,
            family: role.family,
            nat: role.nat,
        }
    }
}

/// The registered keys of a visible peer, which a kept descriptor toward it
/// depends on.
#[derive(PartialEq, Eq)]
pub(crate) struct PeerBinding<'a> {
    pub keys: &'a TrustedNode,
}

impl<'a> View<'a> {
    /// The content alone: listener and dialer comparisons, no capture.
    pub fn new(netmap: &'a NodeNetmap) -> Self {
        Self(
            netmap,
            #[cfg(target_os = "linux")]
            None,
        )
    }

    /// The desired content and its Node-computed capture boundary.
    #[cfg(target_os = "linux")]
    pub fn of(desired: &'a meshway_engine::Desired) -> Self {
        Self(desired.netmap(), desired.capture())
    }

    #[cfg(target_os = "linux")]
    pub fn network_id(self) -> NetworkId {
        self.0.network_id
    }
    pub fn node_id(self) -> NodeId {
        self.0.node_id
    }
    #[cfg(target_os = "linux")]
    pub fn addresses(self) -> &'a meshway_configuration::NodeAddresses {
        &self.0.self_.addresses
    }
    #[cfg(target_os = "linux")]
    pub fn mesh_address_pools(self) -> &'a AddressPools {
        &self.0.self_.mesh_address_pools
    }
    pub fn listeners(self) -> &'a BTreeMap<String, PreparedListener> {
        &self.0.self_.listeners
    }
    pub fn dialers(self) -> &'a BTreeMap<String, DialCandidate> {
        &self.0.dialers
    }
    #[cfg(target_os = "linux")]
    pub fn exit_enabled(self) -> bool {
        self.0.self_.exit_enabled
    }
    #[cfg(target_os = "linux")]
    pub fn exit_role(self) -> Option<RoleIdentity> {
        self.0.exit_role.as_ref().map(RoleIdentity::from)
    }
    #[cfg(target_os = "linux")]
    /// Source capture; `None` for a disabled Node, which owns none.
    pub fn egress_capture(self) -> Option<&'a EgressCapturePlan> {
        self.1
    }

    /// A known peer other than this Node.
    pub fn peer(self, peer: NodeId) -> Option<PeerBinding<'a>> {
        if peer == self.node_id() {
            return None;
        }
        self.0.projection.configuration.nodes.get(&peer)?;
        let keys = self.0.projection.inventory.get(&peer)?;
        Some(PeerBinding { keys })
    }
}
