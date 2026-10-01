use std::collections::{BTreeMap, BTreeSet};

use meshway_protocol::{
    NodeTagAssignment, SaveTagsRequest, SetNodeTagsRequest, TagDefinition, TagSnapshot,
};
use rusqlite::{Connection, TransactionBehavior, params};

use super::{Store, access, configuration};
use crate::Error;

pub(super) fn catalog(connection: &Connection) -> Result<BTreeSet<String>, Error> {
    Ok(connection
        .prepare("SELECT tag FROM tag_definitions ORDER BY tag")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?)
}

pub(super) fn assignments(
    connection: &Connection,
) -> Result<BTreeMap<String, BTreeSet<String>>, Error> {
    let mut result = BTreeMap::new();
    let mut statement =
        connection.prepare("SELECT node_id, tag FROM node_tags ORDER BY node_id, tag")?;
    for row in statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (node, tag) = row?;
        result.entry(node).or_insert_with(BTreeSet::new).insert(tag);
    }
    Ok(result)
}

fn snapshot(connection: &Connection) -> Result<TagSnapshot, Error> {
    let tags = connection
        .prepare("SELECT tag, description FROM tag_definitions ORDER BY tag")?
        .query_map([], |row| {
            Ok(TagDefinition {
                tag: row.get(0)?,
                description: row.get(1)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    let mut membership = assignments(connection)?;
    let assignments = configuration::active_nodes(connection)?
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
        revision: configuration::revision(connection)?,
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
    pub fn tags(&self) -> Result<TagSnapshot, Error> {
        snapshot(&self.connection)
    }

    pub fn save_tags(&mut self, request: &SaveTagsRequest) -> Result<TagSnapshot, Error> {
        let normalized = validate_definitions(&request.tags)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if configuration::revision(&transaction)? != request.expected_revision {
            return Err(Error::Conflict);
        }
        let retained: BTreeSet<_> = normalized.iter().map(|tag| tag.tag.as_str()).collect();
        let mut referenced: BTreeSet<String> =
            assignments(&transaction)?.into_values().flatten().collect();
        referenced.extend(
            access::source(&transaction)?
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
        let current = snapshot(&transaction)?.tags;
        if current != normalized {
            transaction.execute("DELETE FROM tag_definitions", [])?;
            for definition in &normalized {
                transaction.execute(
                    "INSERT INTO tag_definitions(tag,description) VALUES (?1,?2)",
                    params![definition.tag, definition.description],
                )?;
            }
            configuration::bump(&transaction)?;
            configuration::ensure_publishable(&transaction)?;
            access::ensure_publishable(&transaction)?;
        }
        let result = snapshot(&transaction)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn set_node_tags(&mut self, request: &SetNodeTagsRequest) -> Result<TagSnapshot, Error> {
        if request.tags.len() > 32 {
            return Err(Error::Unprocessable("a node may have at most 32 tags"));
        }
        let requested: BTreeSet<_> = request.tags.iter().cloned().collect();
        if requested.len() != request.tags.len() {
            return Err(Error::Unprocessable("node tags must be unique"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if configuration::revision(&transaction)? != request.expected_revision {
            return Err(Error::Conflict);
        }
        let active: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM nodes WHERE id=?1 AND revoked_at IS NULL)",
            [&request.node_id],
            |row| row.get(0),
        )?;
        if !active {
            return Err(Error::Unprocessable(
                "node tags require an active enrolled node",
            ));
        }
        let catalog = catalog(&transaction)?;
        if !requested.is_subset(&catalog) {
            return Err(Error::Unprocessable("all assigned tags must exist"));
        }
        let current = assignments(&transaction)?
            .remove(&request.node_id)
            .unwrap_or_default();
        if current != requested {
            transaction.execute("DELETE FROM node_tags WHERE node_id=?1", [&request.node_id])?;
            for tag in &requested {
                transaction.execute(
                    "INSERT INTO node_tags(node_id,tag) VALUES (?1,?2)",
                    params![request.node_id, tag],
                )?;
            }
            configuration::bump(&transaction)?;
            configuration::ensure_publishable(&transaction)?;
            access::ensure_publishable(&transaction)?;
        }
        let result = snapshot(&transaction)?;
        transaction.commit()?;
        Ok(result)
    }
}
