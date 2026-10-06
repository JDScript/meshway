use crate::error::Error;
pub(crate) mod admission;
mod authentication;
mod cache;
pub(crate) use cache::retire_obsolete;
pub(crate) mod diagnostics;
mod discovery;
mod instances;
mod inventory;
pub(crate) mod progress;
mod publications;
use crate::configuration::ConfigurationBindings;
use crate::{execution::NodePrincipal, secrets::Sealer};
pub(crate) use authentication::{authorize_node, challenge, insert_keys, rotate, session};
pub(crate) use discovery::query_dns;
pub(crate) use instances::{
    acquire, configuration, lease_for_node, receipt, refresh_configuration, release, renew,
};
pub(crate) use inventory::{inventory, inventory_in_transaction, progress, revoke_key, settings};
use meshway_configuration::{NetworkConfiguration, NodeIdentity};
use meshway_model::*;
pub(crate) use publications::{
    apply_configuration_effects, cancel_demand, check_diagnostic_compilation, check_publication,
    configuration_targets, demand, publish, publish_with_rule_sets, receipt_history_targets,
    recent_targets_for_node, renew_demand, retire_node_sessions, select_profile, target_for_node,
};
use rusqlite::{Transaction, params};

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
    let revision = super::configuration::versions::current_revision(tx, network).map_err(
        |error| match error {
            Error::NotFound => Error::InvalidCredentials,
            other => other,
        },
    )?;
    // Execution consumes the configuration owner's canonical version reader,
    // including the same sealed source, canonical value and bindings checks.
    let snapshot = super::configuration::versions::load(tx, network, revision)?.open(sealer)?;
    Ok((revision, snapshot.document.configuration, snapshot.bindings))
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
