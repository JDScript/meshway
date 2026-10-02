use std::path::{Path, PathBuf};

use meshway_protocol::{
    CONFIGURATION_SCHEMA_VERSION, ConfigurationSnapshot, MAX_CONFIGURATION_BYTES,
    MAX_INVENTORY_NODES, configuration_digest,
};
use serde::{Deserialize, Serialize};

use super::{Agent, AgentError, AgentStatus, Identity, local};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cache {
    format_version: u32,
    controller: String,
    pub(crate) node_id: String,
    pub(crate) snapshot: ConfigurationSnapshot,
}

#[cfg(feature = "managed-connections")]
impl Cache {
    pub(crate) fn public_inventory(&self) -> &meshway_protocol::NodeConfiguration {
        &self.snapshot.payload
    }
}

/// Receipt for saved inventory, never policy authorization or application.
#[derive(Debug, Serialize)]
pub struct SyncReceipt {
    pub controller: String,
    pub node_id: String,
    pub stored_revision: u32,
    pub digest: String,
    pub mesh_ipv4: String,
    pub mesh_ipv6: String,
}

pub(crate) fn cache_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(".config.json");
    PathBuf::from(value)
}

pub(super) fn load(path: &Path, identity: &Identity) -> Result<Option<Cache>, AgentError> {
    let path = cache_path(path);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let cache: Cache = serde_json::from_str(&local::read_private_bounded(
        &path,
        MAX_CONFIGURATION_BYTES + 64 * 1024,
    )?)
    .map_err(|_| AgentError::Invalid("invalid configuration cache; refusing to overwrite"))?;
    if cache.format_version != 1
        || cache.controller != identity.controller
        || Some(&cache.node_id) != identity.node_id.as_ref()
    {
        return Err(AgentError::Invalid(
            "configuration cache belongs to a different controller or node",
        ));
    }
    validate(&cache.snapshot, identity, None)?;
    Ok(Some(cache))
}

pub(crate) fn validate(
    snapshot: &ConfigurationSnapshot,
    identity: &Identity,
    previous: Option<&Cache>,
) -> Result<(), AgentError> {
    let payload = &snapshot.payload;
    if serde_json::to_vec(snapshot)
        .map_err(|_| AgentError::Invalid("cannot encode configuration"))?
        .len()
        > MAX_CONFIGURATION_BYTES
        || payload.schema_version != CONFIGURATION_SCHEMA_VERSION
        || payload.revision == 0
        || Some(&payload.node.id) != identity.node_id.as_ref()
        || payload.node.name != identity.name
        || payload.peers.len() >= MAX_INVENTORY_NODES
        || payload
            .peers
            .windows(2)
            .any(|pair| pair[0].id >= pair[1].id)
        || configuration_digest(payload)
            .map_err(|_| AgentError::Invalid("cannot hash configuration"))?
            != snapshot.digest
    {
        return Err(AgentError::Invalid(
            "invalid configuration schema, binding, inventory, or digest",
        ));
    }
    if let Some(previous) = previous {
        let old = &previous.snapshot;
        if payload.revision < old.payload.revision
            || (payload.revision == old.payload.revision && snapshot.digest != old.digest)
            || payload.network != old.payload.network
            || payload.node.mesh_ipv4 != old.payload.node.mesh_ipv4
            || payload.node.mesh_ipv6 != old.payload.node.mesh_ipv6
        {
            return Err(AgentError::Invalid(
                "configuration rollback, same-revision mutation, or identity renumbering",
            ));
        }
    }
    let mut nodes = payload.peers.clone();
    nodes.push(payload.node.clone());
    let ipv4: ipnet::Ipv4Net = payload
        .network
        .ipv4_prefix
        .parse()
        .map_err(|_| AgentError::Invalid("invalid IPv4 pool"))?;
    let ipv6: ipnet::Ipv6Net = payload
        .network
        .ipv6_prefix
        .parse()
        .map_err(|_| AgentError::Invalid("invalid IPv6 pool"))?;
    if ipv4.prefix_len() > 30
        || ipv4.addr() != ipv4.network()
        || ipv4.to_string() != payload.network.ipv4_prefix
        || ipv6.prefix_len() != 64
        || ipv6.addr() != ipv6.network()
        || ipv6.to_string() != payload.network.ipv6_prefix
        || u128::from(ipv6.addr()) >> 120 != 0xfd
    {
        return Err(AgentError::Invalid(
            "configuration network is not a canonical dual-stack mesh pool",
        ));
    }
    for node in &nodes {
        let v4: std::net::Ipv4Addr = node
            .mesh_ipv4
            .parse()
            .map_err(|_| AgentError::Invalid("invalid node IPv4"))?;
        let v6: std::net::Ipv6Addr = node
            .mesh_ipv6
            .parse()
            .map_err(|_| AgentError::Invalid("invalid node IPv6"))?;
        if !local::valid_credential(&node.id)
            || !super::valid_name(&node.name)
            || node.country.as_ref().is_some_and(|value| {
                value.len() != 2 || !value.bytes().all(|byte| byte.is_ascii_uppercase())
            })
            || !ipv4.contains(&v4)
            || v4 == ipv4.network()
            || v4 == ipv4.broadcast()
            || v4.to_string() != node.mesh_ipv4
            || v6.to_string() != node.mesh_ipv6
            || u128::from(v6)
                != u128::from(ipv6.network())
                    + u128::from(u32::from(v4).saturating_sub(u32::from(ipv4.network())))
        {
            return Err(AgentError::Invalid(
                "invalid public mesh node identity or assigned address",
            ));
        }
    }
    meshway_policy::validate_inventory(&payload.network, &nodes).map_err(AgentError::Invalid)?;
    Ok(())
}

pub(super) fn status(identity: &Identity, cache: Option<&Cache>) -> AgentStatus {
    let mut status = identity.status();
    if let Some(cache) = cache {
        status.stored_revision = cache.snapshot.payload.revision;
        status.stored_digest = Some(cache.snapshot.digest.clone());
        status.mesh_ipv4 = Some(cache.snapshot.payload.node.mesh_ipv4.clone());
        status.mesh_ipv6 = Some(cache.snapshot.payload.node.mesh_ipv6.clone());
    }
    status
}

impl Agent {
    pub(crate) fn persist_configuration(
        &mut self,
        snapshot: ConfigurationSnapshot,
    ) -> Result<(), AgentError> {
        if Identity::load(&self.path)? != self.identity {
            return Err(AgentError::Invalid(
                "identity changed during configuration sync",
            ));
        }
        let disk = load(&self.path, &self.identity)?;
        if disk.as_ref().map(|cache| &cache.snapshot.digest)
            != self.cache.as_ref().map(|cache| &cache.snapshot.digest)
        {
            return Err(AgentError::Invalid(
                "configuration cache changed during sync",
            ));
        }
        if self
            .cache
            .as_ref()
            .is_some_and(|cache| cache.snapshot.digest == snapshot.digest)
        {
            return Ok(());
        }
        let cache = Cache {
            format_version: 1,
            controller: self.identity.controller.clone(),
            node_id: snapshot.payload.node.id.clone(),
            snapshot,
        };
        let bytes = serde_json::to_vec(&cache)
            .map_err(|_| AgentError::Invalid("cannot encode configuration cache"))?;
        let result = local::atomic_write(&cache_path(&self.path), &bytes, self.cache.is_some());
        if result.is_ok() || result.as_ref().is_err_and(|error| error.committed()) {
            // Publication changed the visible file, even if its durability is
            // uncertain. Preserve that value as the retry/replay floor in memory;
            // propagate the error and do not ACK this attempt.
            self.cache = Some(cache);
        }
        result?;
        Ok(())
    }
}
