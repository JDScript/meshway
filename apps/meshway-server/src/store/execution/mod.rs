use crate::error::Error;
mod authentication;
mod discovery;
mod instances;
mod inventory;
mod publications;
use crate::configuration::ConfigurationBindings;
use crate::{
    execution::NodePrincipal,
    secrets::{Purpose, Sealer},
};
pub(crate) use authentication::{authorize_node, challenge, insert_keys, rotate, session};
pub(crate) use discovery::query_dns;
pub(crate) use instances::{
    acquire, configuration, current_instance, instance_lease, lease_for_node, receipt, release,
    renew,
};
pub(crate) use inventory::{inventory, inventory_in_transaction, progress, revoke_key, settings};
use meshway_configuration::{NetworkConfiguration, NodeIdentity};
use meshway_model::*;
pub(crate) use publications::{
    apply_configuration_effects, cancel_demand, check_publication, configuration_targets,
    current_target, demand, publish, publish_with_rule_sets, receipt_history_targets,
    recent_targets_for_node, retire_node_sessions, select_profile, target_for_node,
};
use rusqlite::{OptionalExtension, Transaction, params};

pub(crate) fn now() -> Result<i64, Error> {
    let now = chrono::Utc::now().timestamp_millis();
    if now < 0 {
        Err(Error::Internal("system clock before epoch"))
    } else {
        Ok(now)
    }
}
pub(crate) fn uuid() -> Result<uuid::Uuid, Error> {
    crate::accounts::random_uuid()
}
pub(crate) fn id<T: TryFrom<uuid::Uuid>>(v: Vec<u8>) -> Result<T, Error> {
    T::try_from(uuid::Uuid::from_slice(&v).map_err(|_| Error::decode("execution ID"))?)
        .map_err(|_| Error::decode("execution ID"))
}
pub(crate) fn json<T: serde::de::DeserializeOwned>(v: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(v).map_err(|_| Error::decode("execution payload"))
}
pub(crate) fn bytes<T: serde::Serialize>(v: &T) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(v).map_err(|_| Error::encode("execution payload"))
}

pub(crate) fn current_configuration(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
) -> Result<(Revision, NetworkConfiguration, ConfigurationBindings), Error> {
    let (revision,config,bindings):(i64,Vec<u8>,Vec<u8>)=tx.query_row("SELECT n.current_revision,c.configuration,c.bindings FROM networks n JOIN configuration_versions c ON c.network_id=n.id AND c.revision=n.current_revision WHERE n.id=?1 AND n.deleted_at IS NULL",[network.as_uuid().as_bytes().as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or(Error::InvalidCredentials)?;
    let revision = Revision::new(
        revision
            .try_into()
            .map_err(|_| Error::decode("networks.current_revision"))?,
    )
    .map_err(|_| Error::decode("networks.current_revision"))?;
    let config = sealer.open(network, revision, Purpose::Configuration, &config)?;
    let bindings = sealer.open(network, revision, Purpose::Bindings, &bindings)?;
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
) -> Result<(), Error> {
    let settings = config.nodes.get(&node).ok_or(Error::InvalidCredentials)?;
    match &settings.identity {
        NodeIdentity::Tag { tags } => {
            if tags.is_empty() || tags.iter().any(|t| !config.tags_owners.contains_key(t)) {
                return Err(Error::InvalidCredentials);
            }
        }
        NodeIdentity::User { .. } => {
            let binding = bindings
                .node_users
                .get(&node)
                .ok_or(Error::InvalidCredentials)?;
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM memberships m JOIN users u ON u.id=m.user_id WHERE m.network_id=?1 AND m.id=?2 AND m.user_id=?3 AND m.revoked_at IS NULL AND u.disabled_at IS NULL)",params![network.as_uuid().as_bytes().as_slice(),binding.membership_id.as_uuid().as_bytes().as_slice(),binding.user_id.as_uuid().as_bytes().as_slice()],|r|r.get(0))?;
            if !valid {
                return Err(Error::InvalidCredentials);
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
) -> Result<meshway_policy::SubjectContext, Error> {
    Ok(meshway_policy::SubjectContext {
        users: bindings.users.clone(),
        node_users: bindings.node_users.clone(),
        members: crate::configuration::context::members(tx, network)?,
    })
}
pub(crate) fn identity_digest(
    config: &NetworkConfiguration,
    bindings: &ConfigurationBindings,
    node: NodeId,
) -> Result<Digest, Error> {
    match &config
        .nodes
        .get(&node)
        .ok_or(Error::InvalidCredentials)?
        .identity
    {
        NodeIdentity::User { .. } => Ok(meshway_configuration::digest(&(
            "user",
            bindings
                .node_users
                .get(&node)
                .ok_or(Error::InvalidCredentials)?,
        ))?),
        NodeIdentity::Tag { tags } => Ok(meshway_configuration::digest(&("tag", tags))?),
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
) -> Result<NodePrincipal, Error> {
    authorize_node(tx, sealer, boot, p.token_hash, now)
}
