use std::collections::{BTreeMap, BTreeSet};

use meshway_protocol::{
    NodeTagAssignment, SaveTagsRequest, SetNodeTagsRequest, TagDefinition, TagSnapshot,
};
use rusqlite::{Connection, TransactionBehavior, params};

use super::{NetworkId, Store, access, configuration};
use crate::Error;

pub(super) fn catalog(
    connection: &Connection,
    network: &NetworkId,
) -> Result<BTreeSet<String>, Error> {
    Ok(connection
        .prepare("SELECT tag FROM tag_definitions WHERE network_id=?1 ORDER BY tag")?
        .query_map([network.as_str()], |row| row.get(0))?
        .collect::<Result<_, _>>()?)
}

pub(super) fn assignments(
    connection: &Connection,
    network: &NetworkId,
) -> Result<BTreeMap<String, BTreeSet<String>>, Error> {
    let mut result = BTreeMap::new();
    let mut statement = connection
        .prepare("SELECT node_id, tag FROM node_tags WHERE network_id=?1 ORDER BY node_id, tag")?;
    for row in statement.query_map([network.as_str()], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (node, tag) = row?;
        result.entry(node).or_insert_with(BTreeSet::new).insert(tag);
    }
    Ok(result)
}

fn snapshot(connection: &Connection, network: &NetworkId) -> Result<TagSnapshot, Error> {
    let tags = connection
        .prepare("SELECT tag, description FROM tag_definitions WHERE network_id=?1 ORDER BY tag")?
        .query_map([network.as_str()], |row| {
            Ok(TagDefinition {
                tag: row.get(0)?,
                description: row.get(1)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    let mut membership = assignments(connection, network)?;
    let assignments = configuration::active_nodes(connection, network)?
        .into_iter()
        .map(|node| NodeTagAssignment {
            tags: membership
                .remove(&node.id)
                .unwrap_or_default()
                .into_iter()
                .collect(),
            node_id: node.id,
        })
        .collect();
    Ok(TagSnapshot {
        revision: configuration::revision(connection, network)?,
        tags,
        assignments,
    })
}

fn validate_definitions(tags: &[TagDefinition]) -> Result<Vec<TagDefinition>, Error> {
    if tags.len() > 64 {
        return Err(Error::Unprocessable("tag catalog exceeds 64 definitions"));
    }
    let mut normalized = tags.to_vec();
    normalized.sort_by(|a, b| a.tag.cmp(&b.tag));
    let mut names = BTreeSet::new();
    for definition in &normalized {
        if !meshway_policy::valid_tag_selector(&definition.tag)
            || definition.description.chars().count() > 256
            || !names.insert(definition.tag.as_str())
        {
            return Err(Error::Unprocessable(
                "tags must be unique lowercase tag selectors with descriptions of at most 256 characters",
            ));
        }
    }
    Ok(normalized)
}

impl Store {
    pub fn tags(&self, network: &NetworkId) -> Result<TagSnapshot, Error> {
        snapshot(&self.connection, network)
    }

    pub fn save_tags(
        &mut self,
        network: &NetworkId,
        request: &SaveTagsRequest,
    ) -> Result<TagSnapshot, Error> {
        let normalized = validate_definitions(&request.tags)?;
        let transaction = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        if configuration::revision(&transaction, network)? != request.expected_revision {
            return Err(Error::Conflict);
        }
        let retained: BTreeSet<_> = normalized.iter().map(|tag| tag.tag.as_str()).collect();
        let mut referenced: BTreeSet<String> = assignments(&transaction, network)?
            .into_values()
            .flatten()
            .collect();
        referenced.extend(
            access::source(&transaction, network)?
                .rules
                .into_iter()
                .flat_map(|rule| rule.source.into_iter().chain(rule.destination))
                .filter(|selector| selector.starts_with("tag:")),
        );
        if !referenced.iter().all(|tag| retained.contains(tag.as_str())) {
            return Err(Error::Unprocessable(
                "an assigned or access-policy-referenced tag cannot be deleted",
            ));
        }
        let current = snapshot(&transaction, network)?.tags;
        if current != normalized {
            for definition in &normalized {
                if current.iter().any(|tag| tag.tag == definition.tag) {
                    transaction.execute(
                        "UPDATE tag_definitions SET description=?2 WHERE tag=?1 AND network_id=?3",
                        params![definition.tag, definition.description, network.as_str()],
                    )?;
                } else {
                    transaction.execute(
                        "INSERT INTO tag_definitions(tag,description,network_id) VALUES (?1,?2,?3)",
                        params![definition.tag, definition.description, network.as_str()],
                    )?;
                }
            }
            for removed in current
                .iter()
                .filter(|definition| !retained.contains(definition.tag.as_str()))
            {
                transaction.execute(
                    "DELETE FROM tag_definitions WHERE tag=?1 AND network_id=?2",
                    params![removed.tag, network.as_str()],
                )?;
            }
            configuration::bump(&transaction, network)?;
            super::publication::finish_change(
                &transaction,
                network,
                super::publication::ChangeKind::RoutingOrSelectors,
            )?;
        }
        let result = snapshot(&transaction, network)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn set_node_tags(
        &mut self,
        network: &NetworkId,
        request: &SetNodeTagsRequest,
    ) -> Result<TagSnapshot, Error> {
        if request.tags.len() > 32 {
            return Err(Error::Unprocessable("a node may have at most 32 tags"));
        }
        let requested: BTreeSet<_> = request.tags.iter().cloned().collect();
        if requested.len() != request.tags.len() {
            return Err(Error::Unprocessable("node tags must be unique"));
        }
        let transaction = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        if configuration::revision(&transaction, network)? != request.expected_revision {
            return Err(Error::Conflict);
        }
        let active: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM nodes WHERE id=?1 AND revoked_at IS NULL AND network_id=?2)",
            params![request.node_id, network.as_str()],
            |row| row.get(0),
        )?;
        if !active {
            return Err(Error::Unprocessable(
                "node tags require an active enrolled node",
            ));
        }
        let catalog = catalog(&transaction, network)?;
        if !requested.is_subset(&catalog) {
            return Err(Error::Unprocessable("all assigned tags must exist"));
        }
        let current = assignments(&transaction, network)?
            .remove(&request.node_id)
            .unwrap_or_default();
        if current != requested {
            transaction.execute(
                "DELETE FROM node_tags WHERE node_id=?1 AND network_id=?2",
                params![request.node_id, network.as_str()],
            )?;
            for tag in &requested {
                transaction.execute(
                    "INSERT INTO node_tags(node_id,tag,network_id) VALUES (?1,?2,?3)",
                    params![request.node_id, tag, network.as_str()],
                )?;
            }
            configuration::bump(&transaction, network)?;
            super::publication::finish_change(
                &transaction,
                network,
                super::publication::ChangeKind::RoutingOrSelectors,
            )?;
        }
        let result = snapshot(&transaction, network)?;
        transaction.commit()?;
        Ok(result)
    }
}
