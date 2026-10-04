//! Download preparation contains canonical values, never invented artifact IDs.
//! Only the final configuration owner transaction persists content and bindings.
use super::Snapshot;
use crate::error::Error;
use crate::resources::{
    bindings::{self, Bindings, Contents},
    downloads::Downloads,
};
use meshway_configuration::{NetworkConfigurationInput, RuleSet};
use meshway_protocol::resources::PreparedRuleSet;
use rusqlite::Transaction;
use std::collections::BTreeMap;

pub(crate) struct Preparation {
    base: Bindings,
    definitions: BTreeMap<String, RuleSet>,
    pub contents: Contents,
}
impl Preparation {
    pub fn needs_download(&self) -> bool {
        !self.definitions.is_empty()
    }
    pub fn stage(
        tx: &Transaction<'_>,
        current: &Snapshot,
        input: &NetworkConfigurationInput,
    ) -> Result<Self, Error> {
        let base = bindings::at_revision(
            tx,
            current.network_id,
            current.revision,
            &current.bindings.rule_sets,
        )?;
        let definitions: BTreeMap<_, _> = input
            .rule_sets
            .iter()
            .filter(|(key, definition)| {
                matches!(definition, RuleSet::Remote { .. })
                    && (!base.contains_key(*key)
                        || !current
                            .document
                            .configuration
                            .rule_sets
                            .get(*key)
                            .is_some_and(|old| bindings::same_source(old, definition)))
            })
            .map(|(key, definition)| (key.clone(), definition.clone()))
            .collect();
        if definitions.len() > crate::resources::MAX_REMOTE_RULE_SETS {
            return Err(budget_exceeded(
                "maxSets",
                crate::resources::MAX_REMOTE_RULE_SETS,
            ));
        }
        Ok(Self {
            base,
            definitions,
            contents: BTreeMap::new(),
        })
    }
    pub async fn download(&mut self, downloads: &Downloads) -> Result<(), Error> {
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.download_all(downloads),
        )
        .await
        .map_err(|_| {
            Error::InvalidConfiguration(meshway_configuration::ConfigurationError::Invalid(vec![
                meshway_configuration::ConfigurationIssue::new(
                    "/ruleSets",
                    meshway_configuration::IssueCode::RuleSetTimeout,
                )
                .with("seconds", 30),
            ]))
        })?
    }
    async fn download_all(&mut self, downloads: &Downloads) -> Result<(), Error> {
        let mut bytes = 0usize;
        let mut sources: Vec<(&RuleSet, meshway_model::resources::RuleSetData)> = Vec::new();
        for (key, definition) in &self.definitions {
            let data = if let Some((_, data)) = sources
                .iter()
                .find(|(source, _)| bindings::same_source(source, definition))
            {
                data.clone()
            } else {
                let data = downloads.fetch(definition).await.map_err(|problem| {
                    let problem = match problem {
                        crate::resources::downloads::FetchError::Resource(problem) => problem,
                        crate::resources::downloads::FetchError::Cancelled => {
                            return Error::CLOSED;
                        }
                    };
                    let reason = serde_json::to_value(problem)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default();
                    Error::InvalidConfiguration(meshway_configuration::ConfigurationError::Invalid(
                        vec![
                            meshway_configuration::ConfigurationIssue::new(
                                format!("/ruleSets/{}", key.replace('~', "~0").replace('/', "~1")),
                                meshway_configuration::IssueCode::RuleSetUnavailable,
                            )
                            .with("reason", reason),
                        ],
                    ))
                })?;
                sources.push((definition, data.clone()));
                data
            };
            bytes = bytes.saturating_add(
                serde_json::to_vec(&data)
                    .map_err(|_| Error::encode("rule set data"))?
                    .len(),
            );
            if bytes > crate::resources::MAX_DATASET_BYTES {
                return Err(budget_exceeded(
                    "maxBytes",
                    crate::resources::MAX_DATASET_BYTES,
                ));
            }
            self.contents.insert(key.clone(), data);
        }
        Ok(())
    }
    pub fn revalidate(
        &self,
        tx: &Transaction<'_>,
        current: &Snapshot,
        input: &NetworkConfigurationInput,
    ) -> Result<(), Error> {
        if bindings::at_revision(
            tx,
            current.network_id,
            current.revision,
            &current.bindings.rule_sets,
        )? != self.base
            || self
                .definitions
                .iter()
                .any(|(key, definition)| input.rule_sets.get(key) != Some(definition))
            || self.contents.len() != self.definitions.len()
        {
            return Err(Error::RevisionConflict);
        }
        Ok(())
    }
    pub fn bind(
        &self,
        tx: &Transaction<'_>,
        current: &Snapshot,
        bindings: &mut Bindings,
    ) -> Result<(), Error> {
        for (key, data) in &self.contents {
            let artifact = crate::resources::store_artifact(
                tx,
                current.network_id,
                "rule-set",
                data,
                chrono::Utc::now(),
            )?;
            bindings.insert(key.clone(), artifact.id);
        }
        Ok(())
    }
    pub fn preview_bindings(
        &self,
        tx: &Transaction<'_>,
        current: &Snapshot,
        bindings: &mut Bindings,
    ) -> Result<bool, Error> {
        let mut new_content = false;
        for (key, data) in &self.contents {
            let digest = crate::resources::content_digest(
                &serde_json::to_vec(data).map_err(|_| Error::encode("rule set data"))?,
            );
            let id = crate::store::configuration::resources::artifact_by_digest(
                tx,
                current.network_id,
                digest,
            )?;
            if let Some(id) = id {
                if crate::resources::load_artifact(tx, current.network_id, id)? != *data {
                    return Err(Error::decode("dataset_artifacts.data"));
                }
                bindings.insert(key.clone(), id);
            } else {
                new_content = true;
            }
        }
        Ok(new_content)
    }
    pub fn preview(&self) -> Result<BTreeMap<String, PreparedRuleSet>, Error> {
        self.contents
            .iter()
            .map(|(key, data)| {
                Ok((
                    key.clone(),
                    PreparedRuleSet {
                        digest: crate::resources::content_digest(
                            &serde_json::to_vec(data)
                                .map_err(|_| Error::encode("rule set data"))?,
                        ),
                        entries: crate::resources::entries(data) as u32,
                    },
                ))
            })
            .collect()
    }
    pub fn record(
        &self,
        tx: &Transaction<'_>,
        network: meshway_model::NetworkId,
        revision: meshway_model::Revision,
    ) -> Result<(), Error> {
        for key in self.contents.keys() {
            crate::store::configuration::resources::record_check(
                tx,
                network,
                revision,
                key,
                chrono::Utc::now().timestamp_millis(),
            )?;
        }
        Ok(())
    }
}

/// Limits on what one edit may pull in are a property of the configuration,
/// reported like any other rule-set diagnostic, not a server outage.
fn budget_exceeded(limit: &'static str, value: usize) -> Error {
    Error::InvalidConfiguration(meshway_configuration::ConfigurationError::Invalid(vec![
        meshway_configuration::ConfigurationIssue::new(
            "/ruleSets",
            meshway_configuration::IssueCode::RuleSetBudget,
        )
        .with(limit, value),
    ]))
}
