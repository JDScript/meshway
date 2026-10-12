//! Production resource authority and exact version binding.
use super::rulesets::Source;
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
use meshway_configuration::NetworkConfigurationInput;
use meshway_model::{NetworkId, Revision};
use meshway_protocol::resources::{ResourceProblem, RuleSetProviderView, RuleSetView};
use meshway_rules::Digest;
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
        let definitions = super::bindings::configuration_sources(&snapshot.document.configuration);
        let mut views = BTreeMap::new();
        let mut check_versions = BTreeMap::new();
        for (key, source) in &definitions {
            let digest = selected.get(key);
            let artifact = digest
                .map(|digest| super::artifacts::summary(tx, digest))
                .transpose()?;
            let record = digest
                .map(|digest| super::artifacts::record(tx, source, digest))
                .transpose()?
                .flatten();
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
                    implicit: matches!(source, Source::Provider { .. }),
                    definition: source.definition().cloned(),
                    provider: match source {
                        Source::Provider {
                            provider,
                            definition,
                            ..
                        } => Some(RuleSetProviderView {
                            name: provider.clone(),
                            catalog: definition.catalog.clone(),
                            collection: definition.collection.clone(),
                            sequence: record
                                .as_ref()
                                .and_then(|r| r.sequence)
                                .map(|s| u32::try_from(s).unwrap_or(u32::MAX)),
                        }),
                        _ => None,
                    },
                    artifact,
                    source: record.as_ref().map(super::rulesets::Record::source),
                    conversion: record.as_ref().map(super::rulesets::Record::conversion),
                    held: record.as_ref().and_then(super::rulesets::Record::held),
                    problem: problem.or_else(|| {
                        source
                            .needs_download()
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
            definitions,
            providers: meshway_configuration::providers::effective_rule_set_providers(
                &snapshot.document.configuration.rule_set_providers,
            ),
            sealer: Arc::clone(&self.sealer),
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
        artifact: &Digest,
        now: DateTime<Utc>,
    ) -> Result<Vec<crate::effect::Effect>, Error> {
        let current = self.snapshot(tx, network)?;
        if current.revision != revision {
            return Err(Error::StateConflict);
        }
        if !bindings::configuration_sources(&current.document.configuration).contains_key(key) {
            return Err(Error::NotFound);
        }
        if !crate::store::resources::artifacts::exists(tx, artifact)? {
            return Err(Error::InvalidRequest);
        }
        let input = NetworkConfigurationInput::from(&current.document.configuration);
        let mut selected =
            bindings::at_revision(tx, network, revision, &current.bindings.rule_sets)?;
        let unchanged = selected.get(key) == Some(artifact);
        selected.insert(key.into(), *artifact);
        let context = crate::configuration::context::for_current(
            tx,
            &current,
            bindings::contents(tx, &selected)?,
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
