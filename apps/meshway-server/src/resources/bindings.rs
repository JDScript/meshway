//! Exact immutable contents selected for a source version and each publication.
use crate::configuration::Snapshot;
use crate::error::Error;
use crate::store;
use meshway_configuration::{NetworkConfigurationInput, RuleSet};
use meshway_model::resources::RuleSetData;
use meshway_model::{ArtifactId, NetworkId, Revision};
use rusqlite::Transaction;
use std::collections::BTreeMap;

pub(crate) type Bindings = BTreeMap<String, ArtifactId>;
pub(crate) type Contents = BTreeMap<String, RuleSetData>;
#[derive(Default)]
struct ContentBudget {
    bytes: usize,
    entries: usize,
}
impl ContentBudget {
    fn include(&mut self, data: &RuleSetData) -> Result<(), Error> {
        self.bytes = self.bytes.saturating_add(
            serde_json::to_vec(data)
                .map_err(|_| Error::encode("rule set data"))?
                .len(),
        );
        self.entries = self.entries.saturating_add(super::entries(data));
        if self.bytes > super::MAX_DATASET_BYTES || self.entries > super::MAX_RULE_ENTRIES {
            return Err(Error::InvalidConfiguration(
                meshway_configuration::ConfigurationError::Invalid(vec![
                    meshway_configuration::ConfigurationIssue::new(
                        "/ruleSets",
                        meshway_configuration::IssueCode::RuleSetBudget,
                    )
                    .with("maxBytes", super::MAX_DATASET_BYTES)
                    .with("maxEntries", super::MAX_RULE_ENTRIES),
                ]),
            ));
        }
        Ok(())
    }
}
pub(crate) fn check_budget(contents: &Contents) -> Result<(), Error> {
    let mut budget = ContentBudget::default();
    for data in contents.values() {
        budget.include(data)?;
    }
    Ok(())
}
pub(crate) fn same_source(a: &RuleSet, b: &RuleSet) -> bool {
    match (a, b) {
        (
            RuleSet::Remote {
                url: a, format: af, ..
            },
            RuleSet::Remote {
                url: b, format: bf, ..
            },
        ) => a == b && af == bf,
        _ => a == b,
    }
}

pub(crate) fn inherit_checks(
    tx: &Transaction<'_>,
    current: &Snapshot,
    prepared: &crate::configuration::PreparedConfiguration,
    revision: Revision,
) -> Result<(), Error> {
    let previous = at_revision(
        tx,
        current.network_id,
        current.revision,
        &current.bindings.rule_sets,
    )?;
    // A check record follows its rule set into the next version whenever the
    // definition and the bound content are unchanged, including a set that has
    // no content yet because every check so far failed.
    for (key, definition) in &prepared.document.configuration.rule_sets {
        let unchanged_source = current
            .document
            .configuration
            .rule_sets
            .get(key)
            .is_some_and(|before| same_source(before, definition));
        if unchanged_source && previous.get(key) == prepared.bindings.rule_sets.get(key) {
            store::resources::checks::inherit(
                tx,
                current.network_id,
                current.revision,
                revision,
                key,
            )?;
        }
    }
    Ok(())
}

/// The version's current validated binding: the last refreshed binding
/// recorded for it, or its own initial binding.
pub(crate) fn at_revision(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
    initial: &Bindings,
) -> Result<Bindings, Error> {
    Ok(store::resources::bindings::current(tx, network, revision)?
        .unwrap_or_else(|| initial.clone()))
}

pub(crate) fn contents(
    tx: &Transaction<'_>,
    network: NetworkId,
    bindings: &Bindings,
) -> Result<Contents, Error> {
    let mut budget = ContentBudget::default();
    bindings
        .iter()
        .map(|(key, id)| {
            let data = super::load_artifact(tx, network, *id)?;
            budget.include(&data)?;
            Ok((key.clone(), data))
        })
        .collect()
}
pub(crate) fn current_contents(
    tx: &Transaction<'_>,
    current: &Snapshot,
) -> Result<Contents, Error> {
    contents(
        tx,
        current.network_id,
        &at_revision(
            tx,
            current.network_id,
            current.revision,
            &current.bindings.rule_sets,
        )?,
    )
}
pub(crate) fn for_input(
    tx: &Transaction<'_>,
    current: &Snapshot,
    input: &NetworkConfigurationInput,
) -> Result<(Bindings, Contents), Error> {
    let mut bindings = at_revision(
        tx,
        current.network_id,
        current.revision,
        &current.bindings.rule_sets,
    )?;
    bindings.retain(|key, _| {
        input
            .rule_sets
            .get(key)
            .zip(current.document.configuration.rule_sets.get(key))
            .is_some_and(|(a, b)| same_source(a, b))
    });
    let data = contents(tx, current.network_id, &bindings)?;
    Ok((bindings, data))
}

pub(crate) fn compilation_contents(
    tx: &Transaction<'_>,
    network: NetworkId,
    bindings: &Bindings,
) -> Result<BTreeMap<String, meshway_configuration::execution::RuleSetContent>, crate::error::Error>
{
    contents(tx, network, bindings)?
        .into_iter()
        .map(|(key, data)| {
            let artifact = bindings[&key];
            Ok((
                key.clone(),
                meshway_configuration::execution::RuleSetContent {
                    network_id: network,
                    artifact,
                    ip_cidr: data.ip_cidr,
                    domain_suffix: data.domain_suffix,
                    domain: data.domain,
                },
            ))
        })
        .collect()
}
