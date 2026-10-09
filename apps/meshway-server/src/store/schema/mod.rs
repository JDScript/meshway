//! The current private schema: accounts, networks, configuration, execution, state and diagnostics.
//! Only this exact schema is accepted; there is no historical migration chain.

use rusqlite::{Connection, TransactionBehavior};
use thiserror::Error;

const APPLICATION_ID: i64 = 0x4d534857; // MSHW: stable Meshway application identity.
const VERSION: i64 = 1;
const SQL: &str = concat!(
    include_str!("current.sql"),
    "\n",
    include_str!("execution.sql"),
    "\n",
    include_str!("state.sql"),
    "\n",
    include_str!("diagnostics.sql"),
    "\n",
    include_str!("netmap.sql"),
);

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum SchemaError {
    #[error("database application or schema version is unsupported")]
    Unsupported,
    /// Names the first object that differs; object names and kinds only,
    /// never row data.
    #[error("database structure does not match the current schema: {0}")]
    Structure(String),
    #[error("database integrity check failed")]
    Integrity,
    #[error("database schema operation failed")]
    Sqlite,
}

impl From<rusqlite::Error> for SchemaError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Sqlite
    }
}

pub(crate) fn initialize<T, E: From<SchemaError>>(
    connection: &mut Connection,
    initialize_key: impl FnOnce(&Connection, bool) -> Result<T, E>,
) -> Result<T, E> {
    // Existing databases are diagnosed without taking a write transaction.
    if identity(connection)? == (APPLICATION_ID, VERSION) {
        check(connection)?;
        return initialize_key(connection, false);
    }
    // Acquire the writer before testing emptiness; metadata and DDL commit as
    // one unit. A dropped/previously initialized schema is not a fresh database.
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(SchemaError::from)?;
    let schema_version: i64 = transaction
        .pragma_query_value(None, "schema_version", |r| r.get(0))
        .map_err(SchemaError::from)?;
    if identity(&transaction)? != (0, 0)
        || schema_version != 0
        || !objects(&transaction)?.is_empty()
    {
        return Err(SchemaError::Unsupported.into());
    }
    transaction.execute_batch(SQL).map_err(SchemaError::from)?;
    transaction
        .pragma_update(None, "application_id", APPLICATION_ID)
        .map_err(SchemaError::from)?;
    transaction
        .pragma_update(None, "user_version", VERSION)
        .map_err(SchemaError::from)?;
    let value = initialize_key(&transaction, true)?;
    validate(&transaction)?;
    transaction.commit().map_err(SchemaError::from)?;
    Ok(value)
}

/// A read transaction gives all diagnostics one consistent database snapshot.
/// No repair, schema initialization, checkpoint, or write is performed here.
pub(crate) fn check(connection: &mut Connection) -> Result<(), SchemaError> {
    check_with(connection, |_| Ok(()))
}

pub(crate) fn check_with<T, E: From<SchemaError>>(
    connection: &mut Connection,
    check_key: impl FnOnce(&Connection) -> Result<T, E>,
) -> Result<T, E> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(SchemaError::from)?;
    validate(&transaction)?;
    let result = check_key(&transaction)?;
    transaction.rollback().map_err(SchemaError::from)?;
    Ok(result)
}

fn validate(connection: &Connection) -> Result<(), SchemaError> {
    if identity(connection)? != (APPLICATION_ID, VERSION) {
        return Err(SchemaError::Unsupported);
    }
    // Compare every table/index/trigger/view, including SQLite's implicit unique
    // indexes. Exact canonical SQL includes CHECK/FK/STRICT constraints; column
    // names and version numbers alone cannot attest those constraints.
    let expected = Connection::open_in_memory()?;
    expected.execute_batch(SQL)?;
    if let Some(difference) = structure_difference(&objects(connection)?, &objects(&expected)?) {
        return Err(SchemaError::Structure(difference));
    }
    let mut integrity = connection.prepare("PRAGMA integrity_check")?;
    let mut rows = integrity.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        count += 1;
        if row.get::<_, String>(0)? != "ok" {
            return Err(SchemaError::Integrity);
        }
    }
    if count != 1
        || connection
            .prepare("PRAGMA foreign_key_check")?
            .query([])?
            .next()?
            .is_some()
    {
        return Err(SchemaError::Integrity);
    }
    let bindings: i64 =
        connection.query_row("SELECT count(*) FROM master_key_binding", [], |row| {
            row.get(0)
        })?;
    if bindings != 1 {
        return Err(SchemaError::Integrity);
    }
    let challenge_clock_valid: bool = connection.query_row(
        "SELECT (SELECT count(*) FROM node_challenge_clock)=1 AND NOT EXISTS(
          SELECT 1 FROM node_challenges c JOIN node_challenge_clock clock
          WHERE c.created_at>clock.utc_ms OR c.consumed_at>clock.utc_ms)",
        [],
        |row| row.get(0),
    )?;
    if !challenge_clock_valid {
        return Err(SchemaError::Integrity);
    }
    // Triggers validate normal writes. Also reject existing rows corrupted
    // while triggers/FKs were disabled, including a missing current head.
    let invalid_datasets: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM ip_dataset_heads h LEFT JOIN dataset_artifacts a ON a.id=h.artifact_id WHERE a.id IS NULL OR a.kind!=h.kind OR a.network_id IS NOT NULL OR a.format!='mmdb') OR EXISTS(SELECT 1 FROM dataset_artifacts a WHERE a.network_id IS NULL AND a.kind IN ('geoip','asn') AND a.format='mmdb' AND NOT EXISTS(SELECT 1 FROM ip_dataset_heads h WHERE h.kind=a.kind))",
        [],
        |row| row.get(0),
    )?;
    if invalid_datasets {
        return Err(SchemaError::Integrity);
    }
    let invalid_diagnostics: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM diagnostic_subscriptions s JOIN diagnostic_tasks t ON t.id=s.task_id WHERE s.cancelled=0 AND t.compilation IS NULL)
         OR EXISTS(SELECT 1 FROM diagnostic_subscriptions s JOIN diagnostic_tasks t ON t.id=s.task_id
         LEFT JOIN node_keys k ON k.id=s.control_key_id
         LEFT JOIN node_instance_claims i ON i.network_id=s.node_network_id AND i.node_id=s.node_id AND i.instance_id=s.instance_id
         WHERE s.principal_kind='node_instance' AND (s.node_network_id!=t.network_id OR s.node_id!=t.node_id OR s.instance_id!=t.instance_id OR s.boot_id!=t.boot_id OR k.purpose!='control' OR i.generation!=s.instance_generation))",
        [], |row| row.get(0),
    )?;
    if invalid_diagnostics {
        return Err(SchemaError::Integrity);
    }
    Ok(())
}

fn identity(connection: &Connection) -> Result<(i64, i64), SchemaError> {
    Ok((
        connection.pragma_query_value(None, "application_id", |r| r.get(0))?,
        connection.pragma_query_value(None, "user_version", |r| r.get(0))?,
    ))
}

/// The first object that is missing, unexpected or defined differently.
/// Objects are compared by (kind, name); the definition is the canonical SQL.
fn structure_difference(actual: &[SchemaObject], expected: &[SchemaObject]) -> Option<String> {
    let key = |o: &SchemaObject| (o.kind.clone(), o.name.clone());
    let actual_keys: std::collections::BTreeSet<_> = actual.iter().map(key).collect();
    let expected_keys: std::collections::BTreeSet<_> = expected.iter().map(key).collect();
    if let Some((kind, name)) = expected_keys.difference(&actual_keys).next() {
        return Some(format!("missing {kind} {name}"));
    }
    if let Some((kind, name)) = actual_keys.difference(&expected_keys).next() {
        return Some(format!("unexpected {kind} {name}"));
    }
    for object in expected {
        let found = actual
            .iter()
            .find(|candidate| key(candidate) == key(object))?;
        if found.sql != object.sql || found.table != object.table {
            return Some(format!(
                "{} {} is defined differently from the current schema",
                object.kind, object.name
            ));
        }
    }
    None
}

#[derive(PartialEq, Eq)]
struct SchemaObject {
    kind: String,
    name: String,
    table: String,
    sql: Option<String>,
}

fn objects(connection: &Connection) -> Result<Vec<SchemaObject>, SchemaError> {
    connection
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY type, name")?
        .query_map([], |row| {
            Ok(SchemaObject {
                kind: row.get(0)?,
                name: row.get(1)?,
                table: row.get(2)?,
                sql: row.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()
        .map_err(Into::into)
}

#[cfg(test)]
pub(crate) mod tests;
