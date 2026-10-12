//! Rule-set preparation of one edit: inline sets are converted at once,
//! remote and catalog sets are downloaded outside the transaction. Only the
//! final configuration owner transaction stores artifacts and binds them.
use super::Snapshot;
use crate::error::Error;
use crate::resources::{
    bindings::{self, Bindings, Contents},
    downloads::Downloads,
    rulesets::{Prepared, Source, pointer},
};
use meshway_configuration::{
    ConfigurationError, ConfigurationIssue, IssueCode, NetworkConfigurationInput,
    RoutingConfiguration,
};
use meshway_protocol::resources::{PreparedRuleSet, ResourceProblem};
use rusqlite::Transaction;
use std::{collections::BTreeMap, sync::Arc};

pub(crate) struct Preparation {
    base: Bindings,
    definitions: BTreeMap<String, Source>,
    routing: RoutingConfiguration,
    prepared: BTreeMap<String, Prepared>,
    /// Loaded sets of `prepared`, for validation of the draft.
    pub contents: Contents,
}

fn issue(path: String, code: IssueCode) -> ConfigurationIssue {
    ConfigurationIssue::new(path, code)
}

fn reason(problem: ResourceProblem) -> String {
    serde_json::to_value(problem)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

impl Preparation {
    /// Whether some source must be downloaded before the commit.
    pub fn needs_download(&self) -> bool {
        self.definitions.values().any(Source::needs_download)
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
        let before = bindings::configuration_sources(&current.document.configuration);
        let definitions: BTreeMap<_, _> = bindings::input_sources(input)
            .into_iter()
            .filter(|(key, source)| {
                !base.contains_key(key)
                    || !before.get(key).is_some_and(|old| old.same_content(source))
            })
            .collect();
        if definitions.values().filter(|s| s.needs_download()).count()
            > crate::resources::MAX_REMOTE_RULE_SETS
        {
            return Err(bindings::budget_error(
                "maxSets",
                crate::resources::MAX_REMOTE_RULE_SETS as u64,
            ));
        }
        let mut preparation = Self {
            base,
            definitions,
            routing: input.routing.clone(),
            prepared: BTreeMap::new(),
            contents: BTreeMap::new(),
        };
        // Inline content needs no download. An invalid declaration is left
        // unbound; validation reports its entries.
        let inline: Vec<(String, Source)> = preparation
            .definitions
            .iter()
            .filter(|(_, source)| !source.needs_download())
            .map(|(k, s)| (k.clone(), s.clone()))
            .collect();
        for (key, source) in inline {
            if let Source::Inline(set) = &source
                && let Ok(prepared) = crate::resources::rulesets::inline(set)
            {
                preparation.insert(key, prepared)?;
            }
        }
        Ok(preparation)
    }

    fn insert(&mut self, key: String, prepared: Prepared) -> Result<(), Error> {
        let set = meshway_rules::RuleSet::from_vec(prepared.artifact.bytes().to_vec())
            .map_err(|_| Error::Internal("prepared rule-set artifact"))?;
        self.contents.insert(key.clone(), Arc::new(set));
        self.prepared.insert(key, prepared);
        Ok(())
    }

    pub async fn download(&mut self, downloads: &Downloads) -> Result<(), Error> {
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.download_all(downloads),
        )
        .await
        .map_err(|_| {
            Error::InvalidConfiguration(ConfigurationError::Invalid(vec![
                issue("/ruleSets".into(), IssueCode::RuleSetTimeout).with("seconds", 30),
            ]))
        })?
    }
    async fn download_all(&mut self, downloads: &Downloads) -> Result<(), Error> {
        let mut fetched: Vec<(Source, Prepared)> = Vec::new();
        let catalogs = crate::resources::catalogs::Catalogs::default();
        let pending: Vec<(String, Source)> = self
            .definitions
            .iter()
            .filter(|(key, source)| source.needs_download() && !self.prepared.contains_key(*key))
            .map(|(k, s)| (k.clone(), s.clone()))
            .collect();
        for (key, source) in pending {
            let prepared = if let Some((_, prepared)) =
                fetched.iter().find(|(seen, _)| seen.same_content(&source))
            {
                prepared.clone()
            } else {
                let prepared = downloads
                    .fetch(&source, &catalogs)
                    .await
                    .map_err(|problem| {
                        let problem = match problem {
                            crate::resources::downloads::FetchError::Resource(problem) => problem,
                            crate::resources::downloads::FetchError::Cancelled => {
                                return Error::CLOSED;
                            }
                        };
                        let at = pointer(&self.routing, &key);
                        Error::InvalidConfiguration(ConfigurationError::Invalid(vec![if problem
                            == ResourceProblem::UnknownRuleSet
                        {
                            issue(at, IssueCode::UnknownRuleSet).with("key", &key)
                        } else {
                            issue(at, IssueCode::RuleSetUnavailable).with("reason", reason(problem))
                        }]))
                    })?;
                fetched.push((source, prepared.clone()));
                prepared
            };
            self.insert(key, prepared)?;
        }
        bindings::check_budget(&self.contents)
    }
    pub fn revalidate(
        &self,
        tx: &Transaction<'_>,
        current: &Snapshot,
        input: &NetworkConfigurationInput,
    ) -> Result<(), Error> {
        let next = bindings::input_sources(input);
        if bindings::at_revision(
            tx,
            current.network_id,
            current.revision,
            &current.bindings.rule_sets,
        )? != self.base
            || self
                .definitions
                .iter()
                .any(|(key, source)| next.get(key) != Some(source))
            || self
                .definitions
                .iter()
                .any(|(key, source)| source.needs_download() && !self.prepared.contains_key(key))
        {
            return Err(Error::RevisionConflict);
        }
        Ok(())
    }
    /// Stores every prepared artifact and binds it. A catalog whose sequence
    /// regressed since the download fails the edit.
    pub fn bind(
        &self,
        tx: &Transaction<'_>,
        _current: &Snapshot,
        bindings: &mut Bindings,
    ) -> Result<(), Error> {
        let now = chrono::Utc::now().timestamp_millis();
        for (key, prepared) in &self.prepared {
            let source = &self.definitions[key];
            match crate::resources::artifacts::store(tx, source, prepared, now)? {
                Ok(digest) => {
                    bindings.insert(key.clone(), digest);
                }
                Err(problem) => {
                    return Err(Error::InvalidConfiguration(ConfigurationError::Invalid(
                        vec![
                            issue(pointer(&self.routing, key), IssueCode::RuleSetUnavailable)
                                .with("reason", reason(problem)),
                        ],
                    )));
                }
            }
        }
        Ok(())
    }
    /// Content addressing names the digest before anything is stored.
    pub fn preview_bindings(
        &self,
        tx: &Transaction<'_>,
        _current: &Snapshot,
        bindings: &mut Bindings,
    ) -> Result<bool, Error> {
        let mut new_content = false;
        for (key, prepared) in &self.prepared {
            let digest = prepared.artifact.digest();
            new_content |= !crate::store::resources::artifacts::exists(tx, &digest)?;
            bindings.insert(key.clone(), digest);
        }
        Ok(new_content)
    }
    pub fn preview(&self) -> Result<BTreeMap<String, PreparedRuleSet>, Error> {
        Ok(self
            .prepared
            .iter()
            .map(|(key, prepared)| {
                let artifact = &prepared.artifact;
                (
                    key.clone(),
                    PreparedRuleSet {
                        digest: artifact.digest().to_string(),
                        bytes: meshway_protocol::resources::count(artifact.bytes().len() as u64),
                        entries: meshway_protocol::resources::count(artifact.counts().entries()),
                    },
                )
            })
            .collect())
    }
    pub fn record(
        &self,
        tx: &Transaction<'_>,
        network: meshway_model::NetworkId,
        revision: meshway_model::Revision,
    ) -> Result<(), Error> {
        for (key, source) in &self.definitions {
            if source.needs_download() && self.prepared.contains_key(key) {
                crate::store::configuration::resources::record_check(
                    tx,
                    network,
                    revision,
                    key,
                    chrono::Utc::now().timestamp_millis(),
                )?;
            }
        }
        Ok(())
    }
}
