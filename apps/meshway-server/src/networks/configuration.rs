//! The complete typed initial configuration of a new network, sealed for persistence.

use crate::configuration::ConfigurationBindings;
use crate::error::Error;
use crate::{
    accounts::random_uuid,
    secrets::{ProtectedBytes, Purpose, Sealer},
};
use meshway_configuration::{AddressPools, NetworkConfiguration, canonical_json, new_network};
use meshway_model::{NetworkId, Revision};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

pub(crate) struct PreparedNetwork {
    pub(crate) id: NetworkId,
    pub(crate) name: String,
    pub(crate) request_digest: [u8; 32],
    pub(crate) source: ProtectedBytes,
    pub(crate) configuration: ProtectedBytes,
    pub(crate) bindings: ProtectedBytes,
}

pub(crate) fn initial(id: NetworkId) -> Result<NetworkConfiguration, Error> {
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
        .map_err(|_| Error::Internal("initial network configuration"))
}

pub(crate) fn prepare(name: String, sealer: &Sealer) -> Result<PreparedNetwork, Error> {
    let id = NetworkId::new(random_uuid()?).map_err(|_| Error::Internal("generated network ID"))?;
    let canonical = canonical_json(&initial(id)?)
        .map_err(|_| Error::encode("initial network configuration"))?;
    let bindings = Zeroizing::new(
        serde_json::to_vec(&ConfigurationBindings::default())
            .map_err(|_| Error::encode("configuration bindings"))?,
    );
    let mut digest = Sha256::new();
    digest.update(b"meshway/network-create/v1\0");
    digest.update(name.as_bytes());
    Ok(PreparedNetwork {
        id,
        name,
        request_digest: digest.finalize().into(),
        source: sealer.seal(
            id,
            Revision::MIN,
            Purpose::Source,
            canonical.expose().as_bytes(),
        )?,
        configuration: sealer.seal(
            id,
            Revision::MIN,
            Purpose::Configuration,
            canonical.expose().as_bytes(),
        )?,
        bindings: sealer.seal(id, Revision::MIN, Purpose::Bindings, &bindings)?,
    })
}
