//! Exact immutable contents selected for a source version and each publication.
use crate::configuration::Snapshot;
use crate::error::Error;
use meshway_configuration::{NetworkConfigurationInput, RuleSet};
use meshway_model::resources::RuleSetData;
use meshway_model::{ArtifactId, NetworkId, Revision};
use rusqlite::{OptionalExtension, Transaction, params};
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
        self.entries = self
            .entries
            .saturating_add(data.ip_cidr.len())
            .saturating_add(data.domain_suffix.len());
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
    for (key, artifact) in &prepared.bindings.rule_sets {
        if previous.get(key) == Some(artifact)
            && current
                .document
                .configuration
                .rule_sets
                .get(key)
                .zip(prepared.document.configuration.rule_sets.get(key))
                .is_some_and(|(a, b)| same_source(a, b))
        {
            tx.execute(
                "INSERT INTO rule_set_checks(network_id,revision,rule_set_key,problem,checked_at,last_success_at,failures,check_version)
                 SELECT network_id,?1,rule_set_key,problem,checked_at,last_success_at,failures,check_version FROM rule_set_checks
                 WHERE network_id=?2 AND revision=?3 AND rule_set_key=?4",
                params![
                    revision.get() as i64,
                    current.network_id.as_uuid().as_bytes().as_slice(),
                    current.revision.get() as i64,
                    key,
                ],
            )?;
        }
    }
    Ok(())
}

pub(crate) fn latest_publication(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
) -> Result<Option<meshway_model::PublicationId>, Error> {
    let id: Option<Vec<u8>> = tx.query_row(
        "SELECT id FROM execution_publications WHERE network_id=?1 AND revision=?2 ORDER BY sequence DESC LIMIT 1",
        params![network.as_uuid().as_bytes().as_slice(), revision.get() as i64],
        |row| row.get(0),
    ).optional()?;
    id.map(crate::store::execution::id).transpose()
}

pub(crate) fn at_publication(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
    publication: meshway_model::PublicationId,
) -> Result<Bindings, Error> {
    let bytes: Vec<u8> = tx.query_row(
        "SELECT rule_set_bindings FROM execution_publications WHERE network_id=?1 AND revision=?2 AND id=?3",
        params![
            network.as_uuid().as_bytes().as_slice(),
            revision.get() as i64,
            publication.as_uuid().as_bytes().as_slice(),
        ],
        |row| row.get(0),
    ).optional()?.ok_or(Error::DependencyUnavailable)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| Error::decode("execution_publications.rule_set_bindings"))
}

/// The latest publication for this exact source revision owns the current
/// validated binding. Withdrawn publications remain valid historical evidence.
pub(crate) fn at_revision(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
    initial: &Bindings,
) -> Result<Bindings, Error> {
    match latest_publication(tx, network, revision)? {
        Some(publication) => at_publication(tx, network, revision, publication),
        None => Ok(initial.clone()),
    }
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
                },
            ))
        })
        .collect()
}
