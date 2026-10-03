//! The first account-storage slice, not the complete V2 persistence model.
//! Only this exact schema is accepted; there is no historical migration chain.

use rusqlite::{Connection, TransactionBehavior};
use thiserror::Error;

const APPLICATION_ID: i64 = 0x4d534857; // MSHW: stable Meshway application identity.
const VERSION: i64 = 1;
const SQL: &str = include_str!("current.sql");

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum SchemaError {
    #[error("database application or schema version is unsupported")]
    Unsupported,
    #[error("database structure does not match the current schema")]
    Structure,
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
    if objects(connection)? != objects(&expected)? {
        return Err(SchemaError::Structure);
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
    Ok(())
}

fn identity(connection: &Connection) -> Result<(i64, i64), SchemaError> {
    Ok((
        connection.pragma_query_value(None, "application_id", |r| r.get(0))?,
        connection.pragma_query_value(None, "user_version", |r| r.get(0))?,
    ))
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
