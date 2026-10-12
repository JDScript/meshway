//! The artifact each rule-set key of a configuration version is bound to.
//! Bindings name content by digest; inline, remote and catalog sets alike.
use super::rulesets::{Source, sources};
use crate::configuration::Snapshot;
use crate::error::Error;
use crate::store;
use meshway_configuration::{
    ConfigurationError, ConfigurationIssue, IssueCode, NetworkConfiguration,
    NetworkConfigurationInput,
};
use meshway_model::{NetworkId, Revision};
use meshway_policy::compile::RuleSets;
use meshway_rules::Digest;
use rusqlite::Transaction;
use std::collections::BTreeMap;

pub(crate) type Bindings = BTreeMap<String, Digest>;
pub(crate) type Contents = RuleSets;

/// Distinct artifacts one network binds, counted once per digest.
pub(crate) const MAX_BOUND_BYTES: u64 = 64 << 20;
pub(crate) const MAX_BOUND_SETS: usize = 128;

pub(crate) fn budget_error(limit: &'static str, value: u64) -> Error {
    Error::InvalidConfiguration(ConfigurationError::Invalid(vec![
        ConfigurationIssue::new("/ruleSets", IssueCode::RuleSetBudget).with(limit, value),
    ]))
}

/// Per-network budget over distinct digests.
pub(crate) fn check_budget(contents: &Contents) -> Result<(), Error> {
    let mut distinct = BTreeMap::new();
    for set in contents.values() {
        distinct.insert(set.digest(), set.as_bytes().len() as u64);
    }
    if distinct.len() > MAX_BOUND_SETS {
        return Err(budget_error("maxSets", MAX_BOUND_SETS as u64));
    }
    if distinct.values().sum::<u64>() > MAX_BOUND_BYTES {
        return Err(budget_error("maxBytes", MAX_BOUND_BYTES));
    }
    Ok(())
}

pub(crate) fn configuration_sources(cfg: &NetworkConfiguration) -> BTreeMap<String, Source> {
    sources(&cfg.rule_sets, &cfg.rule_set_providers, &cfg.routing)
}
pub(crate) fn input_sources(input: &NetworkConfigurationInput) -> BTreeMap<String, Source> {
    sources(&input.rule_sets, &input.rule_set_providers, &input.routing)
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
    let before = configuration_sources(&current.document.configuration);
    // A check record follows its rule set into the next version whenever the
    // source and the bound content are unchanged, including a set that has
    // no content yet because every check so far failed.
    for (key, source) in configuration_sources(&prepared.document.configuration) {
        let unchanged_source = before
            .get(&key)
            .is_some_and(|old| old.same_content(&source));
        if unchanged_source && previous.get(&key) == prepared.bindings.rule_sets.get(&key) {
            store::resources::checks::inherit(
                tx,
                current.network_id,
                current.revision,
                revision,
                &key,
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

/// The loaded sets of a binding, within the network budget.
pub(crate) fn contents(tx: &Transaction<'_>, bindings: &Bindings) -> Result<Contents, Error> {
    let contents = bindings
        .iter()
        .map(|(key, digest)| Ok((key.clone(), super::artifacts::load(tx, digest)?)))
        .collect::<Result<Contents, Error>>()?;
    check_budget(&contents)?;
    Ok(contents)
}

pub(crate) fn current_contents(
    tx: &Transaction<'_>,
    current: &Snapshot,
) -> Result<Contents, Error> {
    contents(
        tx,
        &at_revision(
            tx,
            current.network_id,
            current.revision,
            &current.bindings.rule_sets,
        )?,
    )
}

/// The current bindings an input keeps: keys whose source is unchanged.
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
    let (next, before) = (
        input_sources(input),
        configuration_sources(&current.document.configuration),
    );
    bindings.retain(|key, _| {
        next.get(key)
            .zip(before.get(key))
            .is_some_and(|(a, b)| a.same_content(b))
    });
    let data = contents(tx, &bindings)?;
    Ok((bindings, data))
}
