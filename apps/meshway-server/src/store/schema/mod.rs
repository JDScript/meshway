//! The current private schema: accounts, networks, configuration, execution, state and netmaps.
//! At run time only this exact schema is accepted. A database of an earlier
//! version is converted once, before it is opened, by the one-way migration
//! in [`migrate`], which reads that version only through its frozen snapshot
//! in `history/`.

use rusqlite::{Connection, TransactionBehavior};
use thiserror::Error;

pub(crate) const APPLICATION_ID: i64 = 0x4d534857; // MSHW: stable Meshway application identity.
// 2: netmap receipts replace execution publications, demands and receipts.
// 3: rule-set artifacts are global `MWRS` content referenced by digest.
// 4: accepted rule-set catalogs keep their verified document.
// 5: rule-set record heads beside immutable provenance.
// 6: routing selections are activities (Node actor, selection target).
// 7: provider catalog checks; unreferenced rule-set artifacts are collected.
// 8: `routing.autoResolve` activities (Server actor, selection target).
// 9: `schema_migrations`; first version migrated rather than reset.
//
// Every SQL change raises VERSION in the same commit, adds the snapshot
// `history/v{VERSION}.sql` (equal to SQL, see `HISTORY`) and its migration
// level in `migrate/`. A new NOT NULL column must be derivable from rows that
// already exist; a fact that cannot be derived does not get a NOT NULL column.
pub(crate) const VERSION: i64 = 9;
/// The oldest version a database is migrated from. Raised only on a minor
/// release boundary, and named in the changelog.
pub(crate) const MIN_MIGRATABLE: i64 = 8;
/// The canonical schema of every version from MIN_MIGRATABLE through VERSION.
/// A snapshot is never modified once committed (its SHA-256 is pinned in the
/// tests); the last one is the current schema.
const HISTORY: &[(i64, &str)] = &[
    (8, include_str!("history/v8.sql")),
    (9, include_str!("history/v9.sql")),
];
const SQL: &str = concat!(
    include_str!("current.sql"),
    "\n",
    include_str!("execution.sql"),
    "\n",
    include_str!("state.sql"),
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

/// SQLite proving the file is not a database or is damaged is a proven
/// inconsistency (persistent); I/O, busy, a full disk and the rest are
/// transient.
impl From<rusqlite::Error> for SchemaError {
    fn from(error: rusqlite::Error) -> Self {
        if damaged(error.sqlite_error_code()) {
            Self::Integrity
        } else {
            Self::Sqlite
        }
    }
}

/// `SQLITE_CORRUPT` or `SQLITE_NOTADB`: the database file itself is damaged
/// or not a database, which no restart changes.
pub(crate) fn damaged(code: Option<rusqlite::ErrorCode>) -> bool {
    matches!(
        code,
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
    )
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

/// The canonical SQL of `version`, from its snapshot.
pub(crate) fn history(version: i64) -> Option<&'static str> {
    HISTORY
        .iter()
        .find(|(candidate, _)| *candidate == version)
        .map(|(_, sql)| *sql)
}

fn validate(connection: &Connection) -> Result<(), SchemaError> {
    validate_as(connection, VERSION, SQL)
}

/// The full check of a database written by schema `version` with canonical
/// SQL `sql`: the current schema, or a snapshot when a migration proves its
/// input and output. The row invariants below hold for every version from
/// MIN_MIGRATABLE on; a version that changes them selects them by `version`.
pub(crate) fn validate_as(
    connection: &Connection,
    version: i64,
    sql: &str,
) -> Result<(), SchemaError> {
    if identity(connection)? != (APPLICATION_ID, version) {
        return Err(SchemaError::Unsupported);
    }
    // Compare every table/index/trigger/view, including SQLite's implicit unique
    // indexes. Exact canonical SQL includes CHECK/FK/STRICT constraints; column
    // names and version numbers alone cannot attest those constraints.
    let expected = Connection::open_in_memory()?;
    expected.execute_batch(sql)?;
    if let Some(difference) = structure_difference(&objects(connection)?, &objects(&expected)?) {
        return Err(SchemaError::Structure(difference));
    }
    let mut integrity = connection.prepare("PRAGMA main.integrity_check")?;
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
            .prepare("PRAGMA main.foreign_key_check")?
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
    Ok(())
}

pub(crate) fn identity(connection: &Connection) -> Result<(i64, i64), SchemaError> {
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

pub(crate) mod migrate;

#[cfg(test)]
pub(crate) mod tests;
