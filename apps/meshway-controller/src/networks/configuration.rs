//! Complete typed initial configuration and its three protected persistence fields.

use super::NetworkError;
use crate::{
    accounts::random_uuid,
    secrets::{ProtectedBytes, Purpose, Sealer},
};
use meshway_configuration::{
    AddressPools, NetworkConfiguration, UserBinding, canonical_json, new_network,
};
use meshway_model::{ArtifactId, NetworkId, NodeId, Revision};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ConfigurationBindings {
    /// Policy references only; retained inactive memberships stay inert.
    pub(crate) users: BTreeMap<String, UserBinding>,
    /// Resolution of each User Node's editable Config.identity.
    pub(crate) node_users: BTreeMap<NodeId, UserBinding>,
    pub(crate) listeners: BTreeMap<NodeId, BTreeMap<String, Revision>>,
    pub(crate) rule_sets: BTreeMap<String, ArtifactId>,
}

pub(crate) struct PreparedNetwork {
    pub(crate) id: NetworkId,
    pub(crate) name: String,
    pub(crate) request_digest: [u8; 32],
    pub(crate) source: ProtectedBytes,
    pub(crate) configuration: ProtectedBytes,
    pub(crate) bindings: ProtectedBytes,
}

pub(crate) fn initial(id: NetworkId) -> Result<NetworkConfiguration, NetworkError> {
    let bytes = id.as_uuid().as_bytes();
    let ula = std::net::Ipv6Addr::new(
        0xfd00 | u16::from(bytes[0]),
        u16::from_be_bytes([bytes[1], bytes[2]]),
        u16::from_be_bytes([bytes[3], bytes[4]]),
        0,
        0,
        0,
        0,
        0,
    );
    let pools = AddressPools {
        ipv4: vec!["100.64.0.0/10".into()],
        ipv6: vec![format!("{ula}/48")],
    };
    new_network(pools, format!("{}.meshway.internal", id.as_uuid().simple()))
        .map_err(|_| NetworkError::Unavailable)
}

pub(crate) fn prepare(name: String, sealer: &Sealer) -> Result<PreparedNetwork, NetworkError> {
    let id = NetworkId::new(random_uuid()?).map_err(|_| NetworkError::Unavailable)?;
    let canonical = canonical_json(&initial(id)?).map_err(|_| NetworkError::Unavailable)?;
    let bindings = Zeroizing::new(
        serde_json::to_vec(&ConfigurationBindings::default())
            .map_err(|_| NetworkError::Unavailable)?,
    );
    let mut digest = Sha256::new();
    digest.update(b"meshway/network-create/v1\0");
    digest.update(name.as_bytes());
    Ok(PreparedNetwork {
        id,
        name,
        request_digest: digest.finalize().into(),
        source: sealer
            .seal(
                id,
                Revision::MIN,
                Purpose::Source,
                canonical.expose().as_bytes(),
            )
            .map_err(|_| NetworkError::Unavailable)?,
        configuration: sealer
            .seal(
                id,
                Revision::MIN,
                Purpose::Configuration,
                canonical.expose().as_bytes(),
            )
            .map_err(|_| NetworkError::Unavailable)?,
        bindings: sealer
            .seal(id, Revision::MIN, Purpose::Bindings, &bindings)
            .map_err(|_| NetworkError::Unavailable)?,
    })
}
