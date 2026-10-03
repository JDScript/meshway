mod authentication;
mod instances;
mod inventory;
mod publications;
use crate::networks::configuration::ConfigurationBindings;
use crate::{
    execution::{NodeError, NodePrincipal},
    secrets::{Purpose, Sealer},
};
pub(crate) use authentication::*;
pub(crate) use instances::*;
pub(crate) use inventory::*;
use meshway_configuration::{NetworkConfiguration, NodeIdentity};
use meshway_model::*;
pub(crate) use publications::*;
use rusqlite::{OptionalExtension, Transaction, params};

pub(crate) fn now() -> Result<i64, NodeError> {
    let now = chrono::Utc::now().timestamp_millis();
    if now < 0 {
        Err(NodeError::Storage)
    } else {
        Ok(now)
    }
}
pub(crate) fn uuid() -> Result<uuid::Uuid, NodeError> {
    crate::accounts::random_uuid().map_err(|_| NodeError::Storage)
}
pub(crate) fn id<T: TryFrom<uuid::Uuid>>(v: Vec<u8>) -> Result<T, NodeError> {
    T::try_from(uuid::Uuid::from_slice(&v).map_err(|_| NodeError::Storage)?)
        .map_err(|_| NodeError::Storage)
}
pub(crate) fn json<T: serde::de::DeserializeOwned>(v: &[u8]) -> Result<T, NodeError> {
    serde_json::from_slice(v).map_err(|_| NodeError::Storage)
}
pub(crate) fn bytes<T: serde::Serialize>(v: &T) -> Result<Vec<u8>, NodeError> {
    serde_json::to_vec(v).map_err(|_| NodeError::Storage)
}

pub(crate) fn current_configuration(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
) -> Result<(Revision, NetworkConfiguration, ConfigurationBindings), NodeError> {
    let (revision,config,bindings):(i64,Vec<u8>,Vec<u8>)=tx.query_row("SELECT n.current_revision,c.configuration,c.bindings FROM networks n JOIN configuration_versions c ON c.network_id=n.id AND c.revision=n.current_revision WHERE n.id=?1 AND n.deleted_at IS NULL",[network.as_uuid().as_bytes().as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or(NodeError::Unauthorized)?;
    let revision = Revision::new(revision.try_into().map_err(|_| NodeError::Storage)?)
        .map_err(|_| NodeError::Storage)?;
    let config = sealer
        .open(network, revision, Purpose::Configuration, &config)
        .map_err(|_| NodeError::Storage)?;
    let bindings = sealer
        .open(network, revision, Purpose::Bindings, &bindings)
        .map_err(|_| NodeError::Storage)?;
    Ok((
        revision,
        json(config.as_bytes())?,
        json(bindings.as_bytes())?,
    ))
}
pub(crate) fn identity_eligible(
    tx: &Transaction<'_>,
    config: &NetworkConfiguration,
    bindings: &ConfigurationBindings,
    network: NetworkId,
    node: NodeId,
) -> Result<(), NodeError> {
    let settings = config.nodes.get(&node).ok_or(NodeError::Unauthorized)?;
    match &settings.identity {
        NodeIdentity::Tag { tags } => {
            if tags.is_empty() || tags.iter().any(|t| !config.tags_owners.contains_key(t)) {
                return Err(NodeError::Unauthorized);
            }
        }
        NodeIdentity::User { .. } => {
            let binding = bindings
                .node_users
                .get(&node)
                .ok_or(NodeError::Unauthorized)?;
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM memberships m JOIN users u ON u.id=m.user_id WHERE m.network_id=?1 AND m.id=?2 AND m.user_id=?3 AND m.revoked_at IS NULL AND u.disabled_at IS NULL)",params![network.as_uuid().as_bytes().as_slice(),binding.membership_id.as_uuid().as_bytes().as_slice(),binding.user_id.as_uuid().as_bytes().as_slice()],|r|r.get(0))?;
            if !valid {
                return Err(NodeError::Unauthorized);
            }
        }
    }
    Ok(())
}
pub(crate) fn user_binding<'a>(
    config: &NetworkConfiguration,
    bindings: &'a ConfigurationBindings,
    node: NodeId,
) -> Option<&'a meshway_configuration::UserBinding> {
    match &config.nodes.get(&node)?.identity {
        NodeIdentity::User { .. } => bindings.node_users.get(&node),
        NodeIdentity::Tag { .. } => None,
    }
}
pub(crate) fn subjects(
    tx: &Transaction<'_>,
    network: NetworkId,
    bindings: &ConfigurationBindings,
) -> Result<meshway_configuration::SubjectContext, NodeError> {
    Ok(meshway_configuration::SubjectContext {
        users: bindings.users.clone(),
        node_users: bindings.node_users.clone(),
        members: crate::execution::configuration::members(tx, network)
            .map_err(|_| NodeError::Storage)?,
    })
}
pub(crate) fn identity_digest(
    config: &NetworkConfiguration,
    bindings: &ConfigurationBindings,
    node: NodeId,
) -> Result<Digest, NodeError> {
    match &config
        .nodes
        .get(&node)
        .ok_or(NodeError::Unauthorized)?
        .identity
    {
        NodeIdentity::User { .. } => Ok(meshway_configuration::compile::digest(&(
            "user",
            bindings
                .node_users
                .get(&node)
                .ok_or(NodeError::Unauthorized)?,
        ))?),
        NodeIdentity::Tag { tags } => Ok(meshway_configuration::compile::digest(&("tag", tags))?),
    }
}
/// The state owner calls this within the same report transaction, not on a
/// cached principal alone. enabled=false may authenticate for control sync.
pub(crate) fn reauthorize(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    p: &NodePrincipal,
    now: i64,
) -> Result<NodePrincipal, NodeError> {
    authorize_node(tx, sealer, boot, p.token_hash, now)
}
