//! Production resource authority, exact version binding and publication adapter.
use super::{
    bindings,
    service::{ResourceProvider, RuleSetScope},
};
use crate::error::Error;
use crate::store;
use crate::{
    configuration::{Snapshot, SnapshotCache},
    secrets::Sealer,
    state::service::ReadIdentity,
};
use chrono::{DateTime, Utc};
use meshway_configuration::{NetworkConfigurationInput, RuleSet};
use meshway_model::{NetworkId, Revision};
use meshway_protocol::resources::{ArtifactSummary, ResourceProblem, RuleSetView};
use rusqlite::Transaction;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub struct Provider {
    pub(crate) sealer: Arc<Sealer>,
    pub(crate) cache: Arc<SnapshotCache>,
}
impl Provider {
    fn snapshot(&self, tx: &Transaction<'_>, network: NetworkId) -> Result<Arc<Snapshot>, Error> {
        let revision = store::configuration::versions::current_revision(tx, network)?;
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
        let mut scope = self.system_rule_sets(tx, *network, now)?;
        scope.user = Some(session.user.id);
        Ok(scope)
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
            let check = store::resources::checks::load(tx, *network, snapshot.revision, key)?;
            check_versions.insert(key.clone(), check.as_ref().map_or(0, |check| check.version));
            let problem = check.as_ref().and_then(|check| check.problem);
            let checked_at = check.as_ref().map(|check| check.checked_at);
            let last_success_at = check.and_then(|check| check.last_success_at);
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
            user: None,
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
    ) -> Result<Vec<crate::effect::Effect>, Error> {
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
        let context = crate::configuration::context::for_current(
            tx,
            &current,
            bindings::contents(tx, network, &selected)?,
        )?;
        meshway_policy::normalize_with_dependencies(input, &context)?;
        if unchanged {
            return Ok(Vec::new());
        }
        crate::store::resources::bindings::record(
            tx,
            network,
            revision,
            &selected,
            now.timestamp_millis(),
        )?;
        crate::store::execution::publish_with_rule_sets(
            tx,
            &self.sealer,
            network,
            "refresh",
            now.timestamp_millis(),
            &selected,
        )?;
        let netmap = crate::store::netmap::recompile_with_rule_sets(
            tx,
            &self.sealer,
            network,
            now.timestamp_millis(),
            &selected,
            crate::store::netmap::Trigger::Configuration,
        )?;
        Ok(netmap.effects())
    }
}

fn summary(
    tx: &Transaction<'_>,
    network: NetworkId,
    id: meshway_model::ArtifactId,
) -> Result<ArtifactSummary, Error> {
    let data = super::load_artifact(tx, network, id)?;
    let (digest, created_at) = store::resources::artifacts::metadata(tx, network, id)?;
    Ok(ArtifactSummary {
        id,
        digest,
        created_at,
        entries: super::entries(&data) as u32,
    })
}
