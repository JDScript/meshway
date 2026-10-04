//! Production resource authority, exact version binding and publication adapter.
use super::{
    bindings,
    service::{ResourceProvider, RuleSetScope},
};
use crate::error::Error;
use crate::{
    configuration::{Snapshot, SnapshotCache},
    secrets::Sealer,
    state::service::ReadIdentity,
};
use chrono::{DateTime, Utc};
use meshway_configuration::{NetworkConfigurationInput, RuleSet};
use meshway_model::{NetworkId, Revision};
use meshway_protocol::resources::{ArtifactSummary, ResourceProblem, RuleSetView};
use rusqlite::{OptionalExtension, Transaction, params};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub struct Provider {
    pub(crate) sealer: Arc<Sealer>,
    pub(crate) cache: Arc<SnapshotCache>,
}
impl Provider {
    fn snapshot(&self, tx: &Transaction<'_>, network: NetworkId) -> Result<Arc<Snapshot>, Error> {
        let revision: i64 = tx
            .query_row(
                "SELECT current_revision FROM networks WHERE id=?1 AND deleted_at IS NULL",
                [network.as_uuid().as_bytes().as_slice()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        let revision = Revision::new(
            revision
                .try_into()
                .map_err(|_| Error::decode("networks.current_revision"))?,
        )
        .map_err(|_| Error::decode("networks.current_revision"))?;
        self.cache.load(tx, network, revision, &self.sealer)
    }
}
impl ResourceProvider for Provider {
    fn rule_sets(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<RuleSetScope, Error> {
        let ReadIdentity::UserSession { digest, network } = identity else {
            return Err(Error::PermissionDenied);
        };
        let session =
            crate::store::authorization::current_session(tx, digest, now.timestamp_millis())?;
        // Resource administration is a full-network view. Member and Node views
        // receive compiled execution only, never source URLs or global catalogs.
        crate::store::authorization::authorize_manager(tx, *network, session.user.id)?;
        self.system_rule_sets(tx, *network, now)
    }
    fn system_rule_sets(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        _now: DateTime<Utc>,
    ) -> Result<RuleSetScope, Error> {
        let network = &network;
        let snapshot = self.snapshot(tx, *network)?;
        let selected = bindings::at_revision(
            tx,
            *network,
            snapshot.revision,
            &snapshot.bindings.rule_sets,
        )?;
        let publication = bindings::latest_publication(tx, *network, snapshot.revision)?;
        let mut views = BTreeMap::new();
        let mut check_versions = BTreeMap::new();
        for (key, definition) in &snapshot.document.configuration.rule_sets {
            let artifact = selected
                .get(key)
                .map(|id| summary(tx, *network, *id))
                .transpose()?;
            let check: Option<(Option<String>, i64, Option<i64>, i64)> = tx.query_row(
                "SELECT problem,checked_at,last_success_at,check_version FROM rule_set_checks WHERE network_id=?1 AND revision=?2 AND rule_set_key=?3",
                params![network.as_uuid().as_bytes().as_slice(), snapshot.revision.get() as i64, key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ).optional()?;
            check_versions.insert(
                key.clone(),
                check.as_ref().map_or(0, |(_, _, _, version)| *version),
            );
            let problem = check
                .as_ref()
                .and_then(|(problem, _, _, _)| problem.as_ref())
                .map(|value| {
                    serde_json::from_str(value)
                        .map_err(|_| Error::decode("rule_set_checks.problem"))
                })
                .transpose()?;
            let checked_at = check
                .as_ref()
                .map(|(_, at, _, _)| {
                    DateTime::from_timestamp_millis(*at)
                        .ok_or(Error::decode("rule_set_checks.checked_at"))
                })
                .transpose()?;
            let last_success_at = check
                .and_then(|(_, _, at, _)| at)
                .map(|at| {
                    DateTime::from_timestamp_millis(at)
                        .ok_or(Error::decode("rule_set_checks.last_success_at"))
                })
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
            check_versions,
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
    ) -> Result<(), Error> {
        let current = self.snapshot(tx, network)?;
        if current.revision != revision {
            return Err(Error::StateConflict);
        }
        current
            .document
            .configuration
            .rule_sets
            .get(key)
            .ok_or(Error::NotFound)?;
        if summary(tx, network, artifact.id)? != *artifact {
            return Err(Error::InvalidRequest);
        }
        let input = NetworkConfigurationInput::from(&current.document.configuration);
        let mut selected =
            bindings::at_revision(tx, network, revision, &current.bindings.rule_sets)?;
        let unchanged = selected.get(key) == Some(&artifact.id);
        selected.insert(key.into(), artifact.id);
        let mut context =
            crate::execution::configuration::validation_context(tx, &current, &input, None, true)?;
        context.rule_sets = bindings::contents(tx, network, &selected)?;
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
        meshway_configuration::normalize_with_dependencies(input, &context)?;
        if unchanged {
            return Ok(());
        }
        crate::store::execution::publish_with_rule_sets(
            tx,
            &self.sealer,
            network,
            "refresh",
            now.timestamp_millis(),
            &selected,
        )?;
        Ok(())
    }
}

fn summary(
    tx: &Transaction<'_>,
    network: NetworkId,
    id: meshway_model::ArtifactId,
) -> Result<ArtifactSummary, Error> {
    let data = super::load_artifact(tx, network, id)?;
    let (digest, created): (String, i64) = tx.query_row(
        "SELECT digest,created_at FROM dataset_artifacts WHERE network_id=?1 AND id=?2 AND kind='ruleSet'",
        params![network.as_uuid().as_bytes().as_slice(), id.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(ArtifactSummary {
        id,
        digest: digest
            .parse()
            .map_err(|_| Error::decode("dataset_artifacts.digest"))?,
        created_at: DateTime::from_timestamp_millis(created)
            .ok_or(Error::decode("dataset_artifacts.created_at"))?,
        entries: (data.ip_cidr.len() + data.domain_suffix.len()) as u32,
    })
}
