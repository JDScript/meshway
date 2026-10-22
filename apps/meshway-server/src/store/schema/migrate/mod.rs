//! The one-way migration of a database of an earlier schema version to
//! [`VERSION`]: the only code that reads an earlier version, and only through
//! that version's snapshot in `history/`. It runs before the SQLite worker
//! opens the database; afterwards the database is the current schema and
//! nothing at run time knows that it was migrated.
//!
//! Each level builds a staging file from its target version's canonical SQL
//! and copies every row from the read-only previous file, so the result is
//! indistinguishable from a new database and passes the same full check.
//! Sealed columns are copied byte for byte (their AAD binds domain and
//! business keys only); netmap payloads, netmap history and signing keys are
//! never decrypted or rewritten, so netmap serials keep their high-water mark.
//! The old file is never written after it was proved and quiesced: it stays
//! as the backup, and at every point the database name is the complete old or
//! the complete new file.
//!
//! Fail-closed classes: a database that is not a supported version, fails its
//! proof, or whose result fails validation is a persistent block (the error
//! names it; nothing is modified). I/O failures, a full disk or an
//! interruption are transient: the next start discards the staging files and
//! runs the same migration again.

use std::path::Path;
use std::time::Instant;

use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use super::{APPLICATION_ID, MIN_MIGRATABLE, SchemaError, VERSION, history, identity, validate_as};
use crate::app::{AppError, Migrated};
use crate::directory::{Directory, backup_name};
use crate::settings::Migration;

mod v9;

/// How long one migration step may take before the service manager's start
/// timeout applies again; every step renews it.
const STEP_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);

/// Startup is progressing: a service manager waiting for readiness extends
/// its start timeout by [`STEP_BUDGET`] (nothing outside a service).
fn progress(from: i64, to: i64, step: &str) {
    meshway_service::extend_startup(
        STEP_BUDGET,
        &format!("Migrating the database from schema {from} to {to}: {step}"),
    );
}

/// What a level does besides copying every table its input and output
/// versions share with the same columns. Anything else is declared here, so
/// a table that appears, disappears or changes columns without a declaration
/// fails the migration (and the fixture tests) instead of losing rows.
pub(super) struct Level {
    /// The version this level produces from `to - 1`.
    pub(super) to: i64,
    /// Tables first created by version `to`; they start empty.
    pub(super) new_tables: &'static [&'static str],
    /// Tables of version `to - 1` whose rows version `to` no longer keeps.
    pub(super) dropped_tables: &'static [&'static str],
    /// `(table, sql)`: the table is filled by `sql` (reading `old.*`) rather
    /// than copied. Its own checks are part of the level's fixture test.
    pub(super) transforms: &'static [(&'static str, &'static str)],
}

pub(super) const LEVELS: &[Level] = &[v9::LEVEL];

/// A target schema with its migration chain: this build's, or a test's.
pub(super) struct Schema {
    pub(super) version: i64,
    pub(super) levels: &'static [Level],
    pub(super) history: fn(i64) -> Option<&'static str>,
}

const CURRENT: Schema = Schema {
    version: VERSION,
    levels: LEVELS,
    history,
};

/// Before the database is opened: nothing for a current, empty or
/// unsupported database (normal initialization accepts or rejects those);
/// a database of an earlier supported version is migrated, or refused with
/// [`AppError::MigrationRequired`] when `migration` does not allow it.
pub(crate) fn prepare(
    directory: &mut Directory,
    migration: Migration,
) -> Result<Option<Migrated>, AppError> {
    prepare_as(directory, migration, &CURRENT)
}

pub(super) fn prepare_as(
    directory: &mut Directory,
    migration: Migration,
    schema: &Schema,
) -> Result<Option<Migrated>, AppError> {
    let target = schema.version;
    directory.discard_staging()?;
    let old = open(&directory.database_path())?;
    let (application, version) = identity(&old)?;
    if (application, version) == (APPLICATION_ID, target) || (application, version) == (0, 0) {
        return Ok(None);
    }
    let unsupported = if application != APPLICATION_ID {
        Some("not a Meshway Server database")
    } else if version > target {
        Some("written by a newer Server build")
    } else if version < MIN_MIGRATABLE {
        Some("older than the oldest version this build migrates")
    } else {
        None
    };
    if let Some(reason) = unsupported {
        tracing::error!(
            application_id = application,
            version,
            supported_from = MIN_MIGRATABLE,
            supported_to = target,
            reason,
            "Database schema is unsupported"
        );
        return Err(AppError::UnsupportedSchema);
    }
    if migration == Migration::Refuse {
        return Err(AppError::MigrationRequired);
    }
    migrate(directory, old, version, schema).map(Some)
}

fn migrate(
    directory: &mut Directory,
    mut old: Connection,
    from: i64,
    schema: &Schema,
) -> Result<Migrated, AppError> {
    let target = schema.version;
    let started = Instant::now();
    let started_at = crate::store::execution::now().map_err(|_| AppError::Database)?;
    tracing::info!(from, to = target, "Database migration started");
    progress(from, target, "checking the database");

    // Only a database that is exactly what version `from` writes is migrated.
    old.pragma_update(None, "query_only", true)
        .map_err(sqlite)?;
    let proof = (|| {
        let transaction = old.transaction_with_behavior(TransactionBehavior::Deferred)?;
        validate_as(
            &transaction,
            from,
            (schema.history)(from).ok_or(SchemaError::Unsupported)?,
        )?;
        transaction.rollback()?;
        Ok::<_, SchemaError>(())
    })();
    if let Err(error) = proof {
        tracing::error!(from, %error, "Database to migrate is not a valid database of its version");
        return Err(error.into());
    }
    #[cfg(test)]
    tests::fault(tests::Point::Proved);

    // Quiesce: the old file alone holds the database, with no WAL that could
    // be replayed onto another file under its name.
    old.pragma_update(None, "query_only", false)
        .map_err(sqlite)?;
    let busy: i64 = old
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .map_err(sqlite)?;
    let mode: String = old
        .pragma_update_and_check(None, "journal_mode", "DELETE", |row| row.get(0))
        .map_err(sqlite)?;
    if busy != 0 || !mode.eq_ignore_ascii_case("delete") {
        return Err(AppError::Database);
    }
    old.close().map_err(|(_, error)| sqlite(error))?;
    directory.ensure_no_journal()?;
    #[cfg(test)]
    tests::fault(tests::Point::Quiesced);

    // Each level's staging file is about the size of the database; while a
    // later level is built, the previous one still exists.
    let size = directory.database_len()?;
    let copies = if target - from > 1 { 2 } else { 1 };
    let needed = size.saturating_add(size / 10).saturating_mul(copies);
    let available = directory.available_bytes()?;
    if available < needed {
        tracing::error!(
            needed,
            available,
            "Not enough free space to migrate the database"
        );
        return Err(AppError::InsufficientSpace);
    }

    let backup = backup_name(from, started_at);
    let mut source: Option<String> = None;
    let (mut tables, mut rows) = (0, 0);
    for to in from + 1..=target {
        let level = schema
            .levels
            .iter()
            .find(|level| level.to == to)
            .ok_or(AppError::UnsupportedSchema)?;
        let (staging, staging_path) = directory.create_staging(to)?;
        let source_path = source.as_ref().map_or_else(
            || directory.database_path(),
            |name| directory.path().join(name),
        );
        let record = (to == target).then(|| Record {
            from,
            started_at,
            backup: &backup,
        });
        let sql = (schema.history)(to).ok_or(AppError::UnsupportedSchema)?;
        let step = |detail: &str| progress(from, target, &format!("schema {to}: {detail}"));
        match build(level, sql, &staging_path, &source_path, record, &step) {
            Ok(copied) => (tables, rows) = copied,
            Err(error) => {
                if let Err(cleanup) = directory.remove_staging(&staging) {
                    tracing::warn!(%cleanup, "Migration staging file not removed; the next start removes it");
                }
                tracing::error!(from, to, %error, "Database migration failed; the database is unchanged");
                return Err(error);
            }
        }
        if let Some(previous) = source.replace(staging) {
            directory.remove_staging(&previous)?;
        }
    }
    let staging = source.ok_or(AppError::UnsupportedSchema)?;
    directory.link_backup(&backup)?;
    #[cfg(test)]
    tests::fault(tests::Point::Linked);
    directory.replace_database(&staging)?;
    let elapsed = started.elapsed();
    tracing::info!(
        from,
        to = target,
        tables,
        rows,
        elapsed_ms = elapsed.as_millis() as u64,
        %backup,
        "Database migrated"
    );
    Ok(Migrated {
        from,
        to: target,
        backup: directory.path().join(backup),
        elapsed,
    })
}

struct Record<'a> {
    from: i64,
    started_at: i64,
    backup: &'a str,
}

/// Builds the database of `level.to` (canonical SQL `sql`) in the empty file `staging` from the
/// database of `level.to - 1` at `source`, as one transaction that also
/// validates the result. Returns the tables and rows copied.
fn build(
    level: &Level,
    sql: &str,
    staging: &Path,
    source: &Path,
    record: Option<Record<'_>>,
    progress: &dyn Fn(&str),
) -> Result<(usize, u64), AppError> {
    let mut db = Connection::open_with_flags(
        staging,
        OpenFlags::default() | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(sqlite)?;
    db.pragma_update(None, "foreign_keys", true)
        .map_err(sqlite)?;
    db.pragma_update(None, "synchronous", "FULL")
        .map_err(sqlite)?;
    let foreign_keys: bool = db
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .map_err(sqlite)?;
    if !foreign_keys {
        return Err(AppError::Database);
    }
    db.execute("ATTACH DATABASE ?1 AS old", [read_only_uri(source)])
        .map_err(sqlite)?;
    let transaction = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite)?;
    transaction.execute_batch(sql).map_err(sqlite)?;
    transaction
        .pragma_update(None, "application_id", APPLICATION_ID)
        .map_err(sqlite)?;
    transaction
        .pragma_update(None, "user_version", level.to)
        .map_err(sqlite)?;
    #[cfg(test)]
    tests::before_copy(&transaction);
    let copied = copy(&transaction, level, progress)?;
    if let Some(record) = record {
        let finished_at = crate::store::execution::now().map_err(|_| AppError::Database)?;
        let mut build = meshway_build::identity();
        while build.len() > 128 {
            build.pop();
        }
        transaction
            .execute(
                "INSERT INTO schema_migrations(sequence,from_version,to_version,build,started_at,finished_at,backup)
                 VALUES((SELECT coalesce(max(sequence),0)+1 FROM schema_migrations),?1,?2,?3,?4,?5,?6)",
                rusqlite::params![record.from, level.to, build, record.started_at, finished_at.max(record.started_at), record.backup],
            )
            .map_err(sqlite)?;
    }
    #[cfg(test)]
    tests::before_validate(&transaction);
    progress("checking the result");
    validate_as(&transaction, level.to, sql)?;
    transaction.commit().map_err(sqlite)?;
    db.execute("DETACH DATABASE old", []).map_err(sqlite)?;
    db.close().map_err(|(_, error)| sqlite(error))?;
    #[cfg(test)]
    tests::fault(tests::Point::Committed);
    Ok(copied)
}

/// Copies every table of `main` from `old`, parents before children (the
/// foreign keys of the target schema), each in primary-key order so that
/// ordering triggers (such as increasing netmap serials) accept the rows.
fn copy(db: &Connection, level: &Level, progress: &dyn Fn(&str)) -> Result<(usize, u64), AppError> {
    let targets = tables(db, "main")?;
    let sources = tables(db, "old")?;
    for source in &sources {
        if !targets.contains(source) && !level.dropped_tables.contains(&source.as_str()) {
            return Err(undeclared(format!(
                "table {source} of version {} is not in version {}",
                level.to - 1,
                level.to
            )));
        }
    }
    let (mut tables, mut rows) = (0, 0u64);
    for table in order(db, &targets)? {
        progress(&format!("copying {table}"));
        if level.new_tables.contains(&table.as_str()) {
            if sources.contains(&table) {
                return Err(undeclared(format!("new table {table} already exists")));
            }
            continue;
        }
        if let Some((_, sql)) = level.transforms.iter().find(|(name, _)| *name == table) {
            db.execute_batch(sql).map_err(sqlite)?;
            tables += 1;
            continue;
        }
        if !sources.contains(&table) {
            return Err(undeclared(format!("table {table} has no source")));
        }
        let columns = columns(db, "main", &table)?;
        let mut source_columns = columns_of(db, "old", &table)?;
        let mut target_columns: Vec<_> = columns.iter().map(|(name, _)| name.clone()).collect();
        source_columns.sort();
        target_columns.sort();
        if source_columns != target_columns {
            return Err(undeclared(format!("table {table} changed its columns")));
        }
        let list = columns
            .iter()
            .map(|(name, _)| quote(name))
            .collect::<Vec<_>>()
            .join(",");
        let mut key: Vec<_> = columns.iter().filter(|(_, pk)| *pk > 0).collect();
        key.sort_by_key(|(_, pk)| *pk);
        let key_list = if key.is_empty() {
            "rowid".to_owned()
        } else {
            key.iter()
                .map(|(name, _)| quote(name))
                .collect::<Vec<_>>()
                .join(",")
        };
        let name = quote(&table);
        let seeded: i64 = db
            .query_row(&format!("SELECT count(*) FROM main.{name}"), [], |row| {
                row.get(0)
            })
            .map_err(sqlite)?;
        if seeded > 0 {
            // Rows the schema itself creates (the challenge clock) take the
            // old values; their triggers allow only that update.
            if key.is_empty() {
                return Err(undeclared(format!(
                    "seeded table {table} has no primary key"
                )));
            }
            let same = key
                .iter()
                .map(|(column, _)| format!("s.{0}=t.{0}", quote(column)))
                .collect::<Vec<_>>()
                .join(" AND ");
            let values: Vec<_> = columns
                .iter()
                .filter(|(_, pk)| *pk == 0)
                .map(|(column, _)| quote(column))
                .collect();
            if !values.is_empty() {
                db.execute(
                    &format!(
                        "UPDATE main.{name} AS t SET ({0})=(SELECT {0} FROM old.{name} s WHERE {same}) WHERE EXISTS(SELECT 1 FROM old.{name} s WHERE {same})",
                        values.join(",")
                    ),
                    [],
                )
                .map_err(sqlite)?;
            }
            db.execute(
                &format!("INSERT INTO main.{name}({list}) SELECT {list} FROM old.{name} s WHERE NOT EXISTS(SELECT 1 FROM main.{name} t WHERE {same}) ORDER BY {key_list}"),
                [],
            )
            .map_err(sqlite)?;
        } else {
            db.execute(
                &format!("INSERT INTO main.{name}({list}) SELECT {list} FROM old.{name} ORDER BY {key_list}"),
                [],
            )
            .map_err(sqlite)?;
        }
        // Same rows, same values: every table has a primary key, so equal
        // counts and an empty difference are equal contents.
        let (copied, missing): (i64, i64) = db
            .query_row(
                &format!(
                    "SELECT (SELECT count(*) FROM main.{name})-(SELECT count(*) FROM old.{name}),
                            (SELECT count(*) FROM (SELECT {list} FROM old.{name} EXCEPT SELECT {list} FROM main.{name}))"
                ),
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(sqlite)?;
        if copied != 0 || missing != 0 {
            return Err(AppError::StorageIntegrity);
        }
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM main.{name}"), [], |row| {
                row.get(0)
            })
            .map_err(sqlite)?;
        tables += 1;
        rows += count as u64;
    }
    sequences(db, &targets)?;
    Ok((tables, rows))
}

/// AUTOINCREMENT high-water marks never go back: identifiers of deleted rows
/// (pagination cursors) are not handed out again.
fn sequences(db: &Connection, targets: &[String]) -> Result<(), AppError> {
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM old.sqlite_schema WHERE type='table' AND name='sqlite_sequence')",
            [],
            |row| row.get(0),
        )
        .map_err(sqlite)?;
    if !exists {
        return Ok(());
    }
    let old: Vec<(String, i64)> = db
        .prepare("SELECT name, seq FROM old.sqlite_sequence")
        .map_err(sqlite)?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(sqlite)?
        .collect::<Result<_, _>>()
        .map_err(sqlite)?;
    for (table, seq) in old {
        if !targets.contains(&table) {
            continue;
        }
        let updated = db
            .execute(
                "UPDATE main.sqlite_sequence SET seq=max(seq,?2) WHERE name=?1",
                rusqlite::params![table, seq],
            )
            .map_err(sqlite)?;
        if updated == 0 {
            db.execute(
                "INSERT INTO main.sqlite_sequence(name,seq) VALUES(?1,?2)",
                rusqlite::params![table, seq],
            )
            .map_err(sqlite)?;
        }
        let kept: i64 = db
            .query_row(
                "SELECT max(seq) FROM main.sqlite_sequence WHERE name=?1",
                [&table],
                |row| row.get(0),
            )
            .map_err(sqlite)?;
        if kept < seq {
            return Err(AppError::StorageIntegrity);
        }
    }
    Ok(())
}

/// Ordinary tables of `schema` in creation order.
fn tables(db: &Connection, schema: &str) -> Result<Vec<String>, AppError> {
    db.prepare(&format!(
        "SELECT name FROM {schema}.sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY rowid"
    ))
    .map_err(sqlite)?
    .query_map([], |row| row.get(0))
    .map_err(sqlite)?
    .collect::<Result<_, _>>()
    .map_err(sqlite)
}

/// `(name, primary-key position)` in declaration order.
fn columns(db: &Connection, schema: &str, table: &str) -> Result<Vec<(String, i64)>, AppError> {
    db.prepare(&format!("PRAGMA {schema}.table_info({})", quote(table)))
        .map_err(sqlite)?
        .query_map([], |row| Ok((row.get(1)?, row.get(5)?)))
        .map_err(sqlite)?
        .collect::<Result<_, _>>()
        .map_err(sqlite)
}

fn columns_of(db: &Connection, schema: &str, table: &str) -> Result<Vec<String>, AppError> {
    Ok(columns(db, schema, table)?
        .into_iter()
        .map(|(name, _)| name)
        .collect())
}

/// Parents before children by the foreign keys among `tables` (self
/// references ignored); creation order otherwise. A cycle continues with its
/// earliest table: only a deferred key (checked at commit) may close one.
fn order(db: &Connection, tables: &[String]) -> Result<Vec<String>, AppError> {
    let mut parents = Vec::with_capacity(tables.len());
    for table in tables {
        let referenced: Vec<String> = db
            .prepare(&format!("PRAGMA main.foreign_key_list({})", quote(table)))
            .map_err(sqlite)?
            .query_map([], |row| row.get(2))
            .map_err(sqlite)?
            .collect::<Result<_, _>>()
            .map_err(sqlite)?;
        parents.push(
            referenced
                .into_iter()
                .filter(|parent| parent != table && tables.contains(parent))
                .collect::<Vec<_>>(),
        );
    }
    let mut done = vec![false; tables.len()];
    let mut ordered = Vec::with_capacity(tables.len());
    while ordered.len() < tables.len() {
        let ready = (0..tables.len()).find(|&index| {
            !done[index]
                && parents[index].iter().all(|parent| {
                    tables
                        .iter()
                        .position(|t| t == parent)
                        .is_some_and(|p| done[p])
                })
        });
        let next = ready.unwrap_or_else(|| done.iter().position(|done| !done).unwrap_or(0));
        done[next] = true;
        ordered.push(tables[next].clone());
    }
    Ok(ordered)
}

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// `file:` URI of `path`, opened read-only.
fn read_only_uri(path: &Path) -> String {
    let mut uri = String::from("file:");
    for byte in path.as_os_str().as_encoded_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(*byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri.push_str("?mode=ro");
    uri
}

fn open(path: &Path) -> Result<Connection, AppError> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(sqlite)
}

/// A level that does not declare how a table changes is a defect of this
/// build, found before anything is published.
fn undeclared(detail: String) -> AppError {
    tracing::error!(detail, "Database migration level is incomplete");
    AppError::SchemaStructure(detail)
}

fn sqlite(error: rusqlite::Error) -> AppError {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DiskFull) => AppError::InsufficientSpace,
        _ => AppError::Database,
    }
}

#[cfg(test)]
pub(crate) mod tests;
