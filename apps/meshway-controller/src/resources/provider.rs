//! Production resource authority, exact version binding and publication adapter.
use super::{
    bindings,
    service::{ResourceProvider, RuleSetScope},
};
use crate::{
    configuration::{Snapshot, SnapshotCache},
    secrets::Sealer,
    state::{StateError, service::ReadIdentity},
};
use chrono::{DateTime, Utc};
use meshway_configuration::{NetworkConfigurationInput, RuleSet};
use meshway_model::{NetworkId, Revision};
use meshway_protocol::resources::{ArtifactSummary, ResourceProblem, RuleSetView};
use rusqlite::{OptionalExtension, Transaction, params};
use std::{collections::BTreeMap, sync::Arc};

pub struct Provider {
    pub(crate) sealer: Arc<Sealer>,
    pub(crate) cache: Arc<SnapshotCache>,
}
impl Provider {
    fn snapshot(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
    ) -> Result<Arc<Snapshot>, StateError> {
        let revision: i64 = tx
            .query_row(
                "SELECT current_revision FROM networks WHERE id=?1 AND deleted_at IS NULL",
                [network.as_uuid().as_bytes().as_slice()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StateError::NotFound)?;
        let revision = Revision::new(revision.try_into().map_err(|_| StateError::Storage)?)
            .map_err(|_| StateError::Storage)?;
        self.cache
            .load(tx, network, revision, &self.sealer)
            .map_err(|_| StateError::Storage)
    }
}
impl ResourceProvider for Provider {
    fn rule_sets(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<RuleSetScope, StateError> {
        let ReadIdentity::UserSession { digest, network } = identity else {
            return Err(StateError::Forbidden);
        };
        let session =
            crate::store::authorization::current_session(tx, digest, now.timestamp_millis())
                .map_err(|_| StateError::Unauthorized)?;
        // Resource administration is a full-network view. Member and Node views
        // receive compiled execution only, never source URLs or global catalogs.
        crate::store::authorization::authorize_manager(tx, *network, session.user.id)
            .map_err(|_| StateError::Forbidden)?;
        let snapshot = self.snapshot(tx, *network)?;
        let selected = bindings::at_revision(
            tx,
            *network,
            snapshot.revision,
            &snapshot.bindings.rule_sets,
        )
        .map_err(|_| StateError::Storage)?;
        let publication = bindings::latest_publication(tx, *network, snapshot.revision)
            .map_err(|_| StateError::Storage)?;
        let mut views = BTreeMap::new();
        for (key, definition) in &snapshot.document.configuration.rule_sets {
            let artifact = selected
                .get(key)
                .map(|id| summary(tx, *network, *id))
                .transpose()?;
            let check: Option<(Option<String>, i64, Option<i64>)> = tx.query_row(
                "SELECT problem,checked_at,last_success_at FROM rule_set_checks WHERE network_id=?1 AND revision=?2 AND rule_set_key=?3",
                params![network.as_uuid().as_bytes().as_slice(), snapshot.revision.get() as i64, key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).optional()?;
            let problem = check
                .as_ref()
                .and_then(|(problem, _, _)| problem.as_ref())
                .map(|value| serde_json::from_str(value).map_err(|_| StateError::Storage))
                .transpose()?;
            let checked_at = check
                .as_ref()
                .map(|(_, at, _)| DateTime::from_timestamp_millis(*at).ok_or(StateError::Storage))
                .transpose()?;
            let last_success_at = check
                .and_then(|(_, _, at)| at)
                .map(|at| DateTime::from_timestamp_millis(at).ok_or(StateError::Storage))
                .transpose()?;
            views.insert(
                key.clone(),
                RuleSetView {
                    key: key.clone(),
                    configuration_revision: snapshot.revision,
                    definition: Some(definition.clone()),
                    artifact,
                    publication: publication.filter(|_| selected.contains_key(key)),
                    problem: problem.or_else(|| {
                        matches!(definition, RuleSet::Remote { .. })
                            .then_some(ResourceProblem::NotReady)
                            .filter(|_| !selected.contains_key(key))
                    }),
                    checked_at,
                    last_success_at,
                },
            );
        }
        Ok(RuleSetScope {
            network: *network,
            revision: snapshot.revision,
            can_refresh: true,
            definitions: snapshot.document.configuration.rule_sets.clone(),
            views,
        })
    }

    fn bind_rule_set(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        revision: Revision,
        key: &str,
        artifact: &ArtifactSummary,
        now: DateTime<Utc>,
    ) -> Result<(), StateError> {
        let current = self.snapshot(tx, network)?;
        if current.revision != revision {
            return Err(StateError::Conflict);
        }
        current
            .document
            .configuration
            .rule_sets
            .get(key)
            .ok_or(StateError::NotFound)?;
        if summary(tx, network, artifact.id)? != *artifact {
            return Err(StateError::Invalid);
        }
        let input = NetworkConfigurationInput::from(&current.document.configuration);
        let mut selected =
            bindings::at_revision(tx, network, revision, &current.bindings.rule_sets)
                .map_err(|_| StateError::Storage)?;
        selected.insert(key.into(), artifact.id);
        let mut context =
            crate::execution::configuration::validation_context(tx, &current, &input, None, true)
                .map_err(|_| StateError::Invalid)?;
        context.rule_sets =
            bindings::contents(tx, network, &selected).map_err(|_| StateError::Invalid)?;
        context.verified_listeners = current
            .document
            .configuration
            .nodes
            .iter()
            .flat_map(|(node, config)| {
                config.listeners.iter().map(move |(key, listener)| {
                    ((*node, key.clone()), listener.declaration().clone())
                })
            })
            .collect();
        meshway_configuration::normalize_with_dependencies(input, &context)
            .map_err(|_| StateError::Invalid)?;
        crate::store::execution::publish_with_rule_sets(
            tx,
            &self.sealer,
            network,
            "refresh",
            now.timestamp_millis(),
            &selected,
        )
        .map_err(|error| match error {
            crate::execution::NodeError::Storage => StateError::Storage,
            _ => StateError::Invalid,
        })?;
        Ok(())
    }
}

fn summary(
    tx: &Transaction<'_>,
    network: NetworkId,
    id: meshway_model::ArtifactId,
) -> Result<ArtifactSummary, StateError> {
    let data = super::load_artifact(tx, network, id)?;
    let (digest, created): (String, i64) = tx.query_row(
        "SELECT digest,created_at FROM dataset_artifacts WHERE network_id=?1 AND id=?2 AND kind='ruleSet'",
        params![network.as_uuid().as_bytes().as_slice(), id.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(ArtifactSummary {
        id,
        digest: digest.parse().map_err(|_| StateError::Storage)?,
        created_at: DateTime::from_timestamp_millis(created).ok_or(StateError::Storage)?,
        entries: (data.ip_cidr.len() + data.domain_suffix.len()) as u32,
    })
}
