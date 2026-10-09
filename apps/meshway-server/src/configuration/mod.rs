//! Complete network configuration: versions, bindings, materials, editing and
//! the one commit boundary every lifecycle change goes through (`commit`).
pub(crate) mod catalog;
pub(crate) mod commit;
pub(crate) mod context;
pub(crate) mod materials;
mod preview;
mod resources;
mod service;
mod snapshot;
pub(crate) use commit::{Committer, Intent};
pub(crate) use service::Configurations;
pub(crate) use service::source_response;
pub(crate) use snapshot::{Snapshot, SnapshotCache};

use crate::error::Error;
use meshway_configuration::{
    ConfigurationChange, ConfigurationDocument, NetworkConfigurationInput, UserBinding,
    ValidationContext, configuration_changes,
};
use meshway_model::{ArtifactId, NetworkId, NodeId, PublicationId, Revision};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The stable subjects behind a configuration version's readable references.
/// Persisted encrypted next to the version; never edited independently.
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

pub(crate) struct PreparedConfiguration {
    pub(crate) network_id: NetworkId,
    pub(crate) base_revision: Revision,
    pub(crate) document: ConfigurationDocument,
    pub(crate) bindings: ConfigurationBindings,
    pub(crate) changes: Vec<ConfigurationChange>,
    pub(crate) deleted_node_ids: BTreeSet<NodeId>,
    pub(crate) source_changed: bool,
    pub(crate) rollback_from: Option<Revision>,
    pub(crate) cause: ChangeCause,
}

pub(crate) use meshway_protocol::history::ConfigurationChangeCause as ChangeCause;

pub(crate) struct CommitEffects {
    pub(crate) notice: Option<meshway_protocol::events::ConfigurationNotice>,
    pub(crate) network_id: NetworkId,
    pub(crate) revision: Revision,
    pub(crate) publication_ids: Vec<PublicationId>,
    pub(crate) target_node_ids: BTreeSet<NodeId>,
    pub(crate) deleted_node_ids: BTreeSet<NodeId>,
    /// Nodes whose netmap this version changed.
    pub(crate) netmap: crate::store::netmap::Recompiled,
}

impl CommitEffects {
    /// Consumed by the owner of the outer transaction.
    pub(crate) fn changes(&self) -> Vec<crate::effect::Effect> {
        use crate::effect::Effect;
        let mut changes = Vec::new();
        if let Some(notice) = &self.notice {
            changes.push(Effect::Configuration {
                notice: notice.clone(),
            });
            changes.push(Effect::Authorization {
                network_id: self.network_id,
            });
            changes.push(Effect::Progress {
                network_id: self.network_id,
            });
        }
        changes.extend(
            self.deleted_node_ids
                .iter()
                .map(|node_id| Effect::NodeRetired {
                    network_id: self.network_id,
                    node_id: *node_id,
                }),
        );
        changes.extend(self.netmap.effects());
        changes
    }
}

/// Normalizes a draft against its trusted context into the next version's
/// document, bindings and change list. In-memory only.
pub(crate) fn prepare(
    input: NetworkConfigurationInput,
    source: String,
    current: &Snapshot,
    context: &ValidationContext<'_>,
    rule_sets: BTreeMap<String, ArtifactId>,
) -> Result<PreparedConfiguration, Error> {
    if context.previous != Some(&current.document.configuration) {
        return Err(Error::DependencyUnavailable);
    }
    let normalized = meshway_policy::normalize_with_dependencies(input, context)?;
    let configuration = normalized.configuration;
    let user_bindings = normalized.user_bindings;
    let mut changes = configuration_changes(&current.document.configuration, &configuration)?;
    let old = &current.document.configuration;
    let deleted_node_ids = old
        .nodes
        .keys()
        .filter(|id| !configuration.nodes.contains_key(id))
        .copied()
        .collect();
    let listeners = listener_bindings(
        current,
        configuration
            .nodes
            .iter()
            .map(|(id, node)| (*id, node.listeners.keys().cloned().collect())),
        &context.recreated_listeners,
    )?;
    let source_changed = current.document.source_jsonc.expose() != source;
    let mut document = current.document.clone();
    document.set_source(source);
    document.configuration = configuration;
    let bindings = ConfigurationBindings {
        users: user_bindings,
        node_users: normalized.node_user_bindings,
        listeners,
        rule_sets,
    };
    changes.extend(binding_changes(&current.bindings, &bindings));
    Ok(PreparedConfiguration {
        network_id: current.network_id,
        base_revision: current.revision,
        document,
        bindings,
        changes,
        deleted_node_ids,
        source_changed,
        rollback_from: None,
        cause: ChangeCause::Edit,
    })
}

fn listener_bindings(
    current: &Snapshot,
    nodes: impl IntoIterator<Item = (NodeId, Vec<String>)>,
    recreated: &BTreeSet<(NodeId, String)>,
) -> Result<BTreeMap<NodeId, BTreeMap<String, Revision>>, Error> {
    let next = Revision::new(
        current
            .revision
            .get()
            .checked_add(1)
            .ok_or(Error::Internal("configuration revision overflow"))?,
    )
    .map_err(|_| Error::Internal("configuration revision overflow"))?;
    Ok(nodes
        .into_iter()
        .map(|(id, keys)| {
            (
                id,
                keys.into_iter()
                    .map(|key| {
                        let revision = if recreated.contains(&(id, key.clone())) {
                            next
                        } else {
                            current
                                .bindings
                                .listeners
                                .get(&id)
                                .and_then(|m| m.get(&key))
                                .copied()
                                .unwrap_or(next)
                        };
                        (key, revision)
                    })
                    .collect(),
            )
        })
        .collect())
}

fn binding_changes(
    before: &ConfigurationBindings,
    after: &ConfigurationBindings,
) -> Vec<ConfigurationChange> {
    [
        ("users", before.users != after.users),
        ("nodeUsers", before.node_users != after.node_users),
        ("listeners", before.listeners != after.listeners),
        ("ruleSets", before.rule_sets != after.rule_sets),
    ]
    .into_iter()
    .filter(|(_, changed)| *changed)
    .map(|(field, _)| ConfigurationChange {
        path: format!("/bindings/{field}"),
        kind: meshway_configuration::ChangeKind::Changed,
    })
    .collect()
}
