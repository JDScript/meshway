//! Private seed storage belongs solely to the node's disposable namespace.
use crate::Error;
use meshway_model::{
    KeyId, NodeId,
    state::{Capability, StreamKind, TransportRole},
};
use meshway_desired_fixture::{NodeMaterial, public_fixture::PublicNode, random_uuid};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::SocketAddr,
    path::Path,
};
use zeroize::Zeroizing;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Seeds {
    node: NodeId,
    signing_id: KeyId,
    noise_id: KeyId,
    signing: [u8; 32],
    noise: [u8; 32],
}
impl Drop for Seeds {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.signing.zeroize();
        self.noise.zeroize();
    }
}
impl Seeds {
    fn material(&self) -> NodeMaterial {
        NodeMaterial::from_seeds(
            self.signing,
            self.noise,
            self.signing_id,
            self.noise_id,
            BTreeMap::from([(
                "transport:native_udp".into(),
                Capability::Transport {
                    roles: vec![TransportRole::Dial, TransportRole::Listen],
                    streams: vec![StreamKind::Udp],
                },
            )]),
        )
    }
}

#[cfg(unix)]
pub fn generate(directory: &Path, socket: SocketAddr) -> Result<PublicNode, Error> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    std::fs::DirBuilder::new().mode(0o700).create(directory)?;
    let mut seeds = Seeds {
        node: NodeId::new(random_uuid()?)?,
        signing_id: KeyId::new(random_uuid()?)?,
        noise_id: KeyId::new(random_uuid()?)?,
        signing: [0; 32],
        noise: [0; 32],
    };
    getrandom::fill(&mut seeds.signing).map_err(|_| "OS randomness unavailable")?;
    getrandom::fill(&mut seeds.noise).map_err(|_| "OS randomness unavailable")?;
    let bytes = Zeroizing::new(serde_json::to_vec(&seeds)?);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join("seeds.json"))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(PublicNode {
        id: seeds.node,
        socket,
        trusted: seeds.material().trusted().clone(),
    })
}

#[cfg(unix)]
pub fn load(path: &Path) -> Result<(NodeId, NodeMaterial), Error> {
    use std::os::unix::fs::PermissionsExt;
    let file = std::fs::File::open(path)?;
    if file.metadata()?.permissions().mode() & 0o777 != 0o600 {
        return Err("private seed file must have mode 0600".into());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    std::io::Read::take(file, 4096).read_to_end(&mut bytes)?;
    let seeds: Seeds = serde_json::from_slice(&bytes).map_err(|_| "invalid private seed file")?;
    Ok((seeds.node, seeds.material()))
}

#[cfg(not(unix))]
pub fn generate(_: &Path, _: SocketAddr) -> Result<PublicNode, Error> {
    Err("Unix required".into())
}
#[cfg(not(unix))]
pub fn load(_: &Path) -> Result<(NodeId, NodeMaterial), Error> {
    Err("Unix required".into())
}
