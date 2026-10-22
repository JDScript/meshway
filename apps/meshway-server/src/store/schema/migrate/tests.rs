use std::cell::Cell;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::*;
use crate::store::schema::{HISTORY, SQL};

/// SHA-256 of every committed snapshot. A snapshot is never edited: a schema
/// change raises VERSION and adds the next one. Changing the SQL without that
/// fails `the_current_schema_is_the_newest_snapshot`; editing the snapshot to
/// match fails this pin.
const SNAPSHOTS: &[(i64, &str)] = &[
    (
        8,
        "5bd8df9f022f9e4d626eda6b001da574ed8e916de1b071312d9667682682a28f",
    ),
    (
        9,
        "fc8d822bcbcc220cf1b1860f5ffff3fcffa2a26d870041ccdac76f501f98720a",
    ),
];

const FIXTURE_V8: &str = include_str!("fixture-v8.sql");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Point {
    Proved,
    Quiesced,
    Committed,
    Linked,
}

thread_local! {
    static CRASH: Cell<Option<Point>> = const { Cell::new(None) };
    static FULL: Cell<bool> = const { Cell::new(false) };
    static CORRUPT: Cell<bool> = const { Cell::new(false) };
}

/// An injected crash: the panic unwinds without any further filesystem
/// operation, as a killed process would leave the directory.
pub(super) fn fault(point: Point) {
    if CRASH.get() == Some(point) {
        panic!("injected crash at {point:?}");
    }
}

/// The disk fills up while rows are copied: the schema fits, the rule-set
/// artifact of the fixture does not.
pub(super) fn before_copy(db: &Connection) {
    if FULL.get() {
        let pages: i64 = db
            .pragma_query_value(Some("main"), "page_count", |row| row.get(0))
            .unwrap();
        db.pragma_update(Some("main"), "max_page_count", pages + 4)
            .unwrap();
    }
}

pub(super) fn before_validate(db: &Connection) {
    if CORRUPT.get() {
        db.execute_batch("DROP INDEX main.user_sessions_user_id")
            .unwrap();
    }
}

#[test]
fn the_current_schema_is_the_newest_snapshot() {
    assert!(
        history(VERSION) == Some(SQL),
        "the schema SQL changed without a new VERSION and history/v{{VERSION}}.sql"
    );
    assert_eq!(HISTORY.last().map(|(version, _)| *version), Some(VERSION));
}

#[test]
fn snapshots_are_pinned_contiguous_and_each_has_its_level() {
    let pinned: Vec<_> = SNAPSHOTS.iter().map(|(version, _)| *version).collect();
    let present: Vec<_> = HISTORY.iter().map(|(version, _)| *version).collect();
    assert_eq!(present, pinned);
    assert_eq!(present, (MIN_MIGRATABLE..=VERSION).collect::<Vec<_>>());
    for ((version, sql), (_, digest)) in HISTORY.iter().zip(SNAPSHOTS) {
        assert_eq!(
            format!("{:x}", Sha256::digest(sql.as_bytes())),
            *digest,
            "history/v{version}.sql was modified"
        );
    }
    let levels: Vec<_> = LEVELS.iter().map(|level| level.to).collect();
    assert_eq!(levels, (MIN_MIGRATABLE + 1..=VERSION).collect::<Vec<_>>());
}

fn snapshot_tables(sql: &str) -> Vec<(String, Vec<String>)> {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(sql).unwrap();
    let names = tables(&db, "main").unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut columns = columns_of(&db, "main", &name).unwrap();
            columns.sort();
            (name, columns)
        })
        .collect()
}

/// A level declares exactly how its two snapshots differ, so that nothing
/// is copied by accident or dropped silently.
#[test]
fn levels_declare_exactly_the_differences_of_their_snapshots() {
    for level in LEVELS {
        let before = snapshot_tables(history(level.to - 1).unwrap());
        let after = snapshot_tables(history(level.to).unwrap());
        let named = |tables: &[(String, Vec<String>)], name: &str| {
            tables
                .iter()
                .find(|(table, _)| table == name)
                .map(|(_, columns)| columns.clone())
        };
        let mut new: Vec<_> = after
            .iter()
            .filter(|(name, _)| named(&before, name).is_none())
            .map(|(name, _)| name.as_str())
            .collect();
        let mut dropped: Vec<_> = before
            .iter()
            .filter(|(name, _)| named(&after, name).is_none())
            .map(|(name, _)| name.as_str())
            .collect();
        let mut changed: Vec<_> = after
            .iter()
            .filter(|(name, columns)| named(&before, name).is_some_and(|old| &old != columns))
            .map(|(name, _)| name.as_str())
            .collect();
        fn sorted<'a>(values: &[&'a str]) -> Vec<&'a str> {
            let mut values = values.to_vec();
            values.sort();
            values
        }
        new.sort();
        dropped.sort();
        changed.sort();
        assert_eq!(new, sorted(level.new_tables), "level {}", level.to);
        assert_eq!(dropped, sorted(level.dropped_tables), "level {}", level.to);
        // A transform may also rewrite a table whose columns stay.
        for table in changed {
            assert!(
                level.transforms.iter().any(|(name, _)| *name == table),
                "level {}: {table} changed columns without a transform",
                level.to
            );
        }
    }
}

pub(crate) fn location() -> (tempfile::TempDir, PathBuf) {
    let parent = tempfile::tempdir().unwrap();
    let path = parent.path().canonicalize().unwrap().join("data");
    (parent, path)
}

/// A data directory whose database is the v8 fixture, in DELETE mode.
fn fixture_directory() -> (tempfile::TempDir, PathBuf) {
    let (parent, path) = location();
    let directory = Directory::open(&path).unwrap();
    let db = Connection::open(directory.database_path()).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    db.execute_batch(&format!(
        "BEGIN; {} PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = 8; {FIXTURE_V8} COMMIT;",
        history(8).unwrap()
    ))
    .unwrap();
    // Every table of the oldest supported version has rows to migrate.
    for table in tables(&db, "main").unwrap() {
        let rows: i64 = db
            .query_row(
                &format!("SELECT count(*) FROM {}", quote(&table)),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(rows > 0, "fixture has no {table} row");
    }
    validate_as(&db, 8, history(8).unwrap()).unwrap();
    drop(db);
    drop(directory);
    (parent, path)
}

fn digest(path: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}

fn names(path: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn backups(path: &Path) -> Vec<String> {
    names(path)
        .into_iter()
        .filter(|name| name.starts_with("controller.sqlite.v"))
        .collect()
}

/// Every table of the backup has the same rows in the migrated database,
/// high-water marks did not go back, and the result is a valid database
/// of this build with one migration record.
fn assert_migrated(path: &Path, from_backup: &str) {
    let db = Connection::open(path.join("controller.sqlite")).unwrap();
    validate_as(&db, VERSION, SQL).unwrap();
    db.execute(
        "ATTACH DATABASE ?1 AS old",
        [read_only_uri(&path.join(from_backup))],
    )
    .unwrap();
    validate_as(&db, VERSION, SQL).unwrap();
    for table in tables(&db, "old").unwrap() {
        let name = quote(&table);
        let difference: i64 = db
            .query_row(
                &format!(
                    "SELECT (SELECT count(*) FROM (SELECT * FROM old.{name} EXCEPT SELECT * FROM main.{name}))
                          + (SELECT count(*) FROM (SELECT * FROM main.{name} EXCEPT SELECT * FROM old.{name}))
                          + abs((SELECT count(*) FROM old.{name}) - (SELECT count(*) FROM main.{name}))"
                ),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(difference, 0, "{table} differs");
    }
    let regressed: i64 = db
        .query_row(
            "SELECT count(*) FROM old.sqlite_sequence o LEFT JOIN main.sqlite_sequence n ON n.name=o.name WHERE n.seq IS NULL OR n.seq<o.seq",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(regressed, 0);
    let record: (i64, i64, String, String) = db
        .query_row(
            "SELECT from_version,to_version,build,backup FROM schema_migrations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        record,
        (
            8,
            VERSION,
            meshway_build::identity(),
            from_backup.to_owned()
        )
    );
    let journal: String = db
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(journal, "delete");
    // The backup is the complete old database, an owner-only single file.
    let metadata = std::fs::metadata(path.join(from_backup)).unwrap();
    assert_eq!(
        (metadata.permissions().mode() & 0o7777, metadata.nlink()),
        (0o600, 1)
    );
    db.execute_batch("DETACH DATABASE old").unwrap();
    let old = Connection::open(path.join(from_backup)).unwrap();
    validate_as(&old, 8, history(8).unwrap()).unwrap();
    // What schema 8 opening did with a newer database: refuse it.
    assert_eq!(
        validate_as(&db, 8, history(8).unwrap()),
        Err(SchemaError::Unsupported)
    );
    assert!(
        names(path)
            .iter()
            .all(|name| !name.starts_with("controller.sqlite.migrating")
                && name != "controller.sqlite-wal"
                && name != "controller.sqlite-journal")
    );
}

fn migrate_directory(path: &Path) -> Result<Option<Migrated>, AppError> {
    let mut directory = Directory::open(path)?;
    prepare(&mut directory, Migration::Automatic)
}

#[test]
fn a_schema_8_database_becomes_a_database_equal_to_a_new_one_with_the_same_rows() {
    let (_parent, path) = fixture_directory();
    let before = digest(&path.join("controller.sqlite"));
    let migrated = migrate_directory(&path).unwrap().unwrap();
    assert_eq!((migrated.from, migrated.to), (8, VERSION));
    let backup = backups(&path);
    assert_eq!(backup.len(), 1);
    assert_eq!(migrated.backup, path.join(&backup[0]));
    // The old file is the backup, byte for byte (it was already quiesced).
    assert_eq!(digest(&path.join(&backup[0])), before);
    assert_migrated(&path, &backup[0]);

    // Opening again is a current database: nothing happens.
    assert_eq!(migrate_directory(&path).unwrap(), None);
    assert_eq!(backups(&path), backup);

    // The same writes as before, with sequences above the old high-water.
    let db = Connection::open(path.join("controller.sqlite")).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    db.execute("INSERT INTO activity_events (id, scope, account_user_id, at, actor_user_id, action, target_user_id, request_id) VALUES (x'000000000000000000000000000000f9', 'account', x'00000000000000000000000000000011', 2000, x'00000000000000000000000000000011', 'account.password', x'00000000000000000000000000000011', x'000000000000000000000000000000eb')", []).unwrap();
    let sequence: i64 = db
        .query_row("SELECT max(sequence) FROM activity_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(sequence, 91);
    let row: i64 = db
        .query_row("INSERT INTO observation_events VALUES (NULL, x'00000000000000000000000000000031', 'node', 'instance', 'event-3', 1002, 1002, NULL, '0000000000000000000000000000000000000000000000000000000000000004', 2002) RETURNING row_id", [], |row| row.get(0))
        .unwrap();
    assert_eq!(row, 3);
    for mutation in [
        "UPDATE node_challenge_clock SET utc_ms = 4999",
        "UPDATE schema_migrations SET backup = 'other'",
        "DELETE FROM schema_migrations",
        "INSERT INTO node_netmaps SELECT network_id,node_id,1,revision,digest,dependency_digest,payload,created_at,NULL FROM node_netmaps WHERE serial=1",
    ] {
        assert!(db.execute(mutation, []).is_err(), "{mutation}");
    }
}

#[test]
fn unsupported_damaged_or_refused_databases_are_left_unchanged() {
    for (mutation, expected) in [
        ("PRAGMA user_version = 10", AppError::UnsupportedSchema),
        ("PRAGMA user_version = 7", AppError::UnsupportedSchema),
        ("PRAGMA application_id = 17", AppError::UnsupportedSchema),
        (
            "DROP INDEX user_sessions_user_id",
            AppError::SchemaStructure("missing index user_sessions_user_id".into()),
        ),
        (
            "PRAGMA ignore_check_constraints = ON; UPDATE users SET credential_version = 0",
            AppError::StorageIntegrity,
        ),
        (
            "DROP TRIGGER node_challenge_clock_no_delete; DELETE FROM node_challenge_clock; CREATE TRIGGER node_challenge_clock_no_delete BEFORE DELETE ON node_challenge_clock\nBEGIN SELECT RAISE(ABORT, 'challenge clock cannot be deleted'); END",
            AppError::StorageIntegrity,
        ),
    ] {
        let (_parent, path) = fixture_directory();
        let db = Connection::open(path.join("controller.sqlite")).unwrap();
        db.execute_batch(mutation).unwrap();
        drop(db);
        let before = digest(&path.join("controller.sqlite"));
        let entries = names(&path);
        assert_eq!(
            migrate_directory(&path),
            Err(expected.clone()),
            "{mutation}"
        );
        assert!(expected.is_persistent());
        assert_eq!(digest(&path.join("controller.sqlite")), before);
        assert_eq!(names(&path), entries);
    }
    let (_parent, path) = fixture_directory();
    let before = digest(&path.join("controller.sqlite"));
    let mut directory = Directory::open(&path).unwrap();
    assert_eq!(
        prepare(&mut directory, Migration::Refuse),
        Err(AppError::MigrationRequired)
    );
    assert_eq!(digest(&path.join("controller.sqlite")), before);
}

/// Interrupted at any step, the database name holds the complete old file;
/// the next start removes what the attempt left and migrates the same way.
#[test]
fn an_interrupted_migration_leaves_the_old_database_and_completes_on_the_next_start() {
    for point in [
        Point::Proved,
        Point::Quiesced,
        Point::Committed,
        Point::Linked,
    ] {
        let (_parent, path) = fixture_directory();
        let crashed = std::panic::catch_unwind(|| {
            CRASH.set(Some(point));
            let result = migrate_directory(&path);
            CRASH.set(None);
            result
        });
        CRASH.set(None);
        assert!(crashed.is_err(), "{point:?}");
        let old = Connection::open(path.join("controller.sqlite")).unwrap();
        assert_eq!(identity(&old).unwrap(), (APPLICATION_ID, 8), "{point:?}");
        drop(old);
        if point == Point::Linked {
            // Two names of one file: not a checked database until the second
            // name is removed by the next start.
            assert_eq!(backups(&path).len(), 1);
        }
        let migrated = migrate_directory(&path).unwrap().unwrap();
        let backup = backups(&path);
        assert_eq!(backup.len(), 1, "{point:?}");
        assert_eq!(migrated.backup, path.join(&backup[0]));
        assert_migrated(&path, &backup[0]);
    }
}

#[test]
fn a_full_disk_or_an_invalid_result_publishes_nothing() {
    for (flag, expected, persistent) in [
        (&FULL, AppError::InsufficientSpace, false),
        (
            &CORRUPT,
            AppError::SchemaStructure("missing index user_sessions_user_id".into()),
            true,
        ),
    ] {
        let (_parent, path) = fixture_directory();
        flag.with(|flag| flag.set(true));
        let failed = migrate_directory(&path);
        flag.with(|flag| flag.set(false));
        assert_eq!(failed, Err(expected.clone()));
        assert_eq!(expected.is_persistent(), persistent);
        assert!(backups(&path).is_empty());
        assert!(
            names(&path)
                .iter()
                .all(|name| !name.starts_with("controller.sqlite.migrating"))
        );
        let old = Connection::open(path.join("controller.sqlite")).unwrap();
        validate_as(&old, 8, history(8).unwrap()).unwrap();
        drop(old);
        migrate_directory(&path).unwrap().unwrap();
        assert_migrated(&path, &backups(&path)[0]);
    }
}

#[test]
fn leftover_staging_files_are_never_read() {
    let (_parent, path) = fixture_directory();
    for name in [
        "controller.sqlite.migrating.v9",
        "controller.sqlite.migrating.v9-journal",
    ] {
        std::fs::write(path.join(name), b"not a database").unwrap();
        std::fs::set_permissions(path.join(name), std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    migrate_directory(&path).unwrap().unwrap();
    assert_migrated(&path, &backups(&path)[0]);
}

/// The WAL of the old database belongs to it: its committed frames are
/// migrated, and no WAL is left that could be replayed onto the new file.
#[test]
fn committed_wal_frames_of_the_old_database_are_migrated_and_not_left_behind() {
    let (_parent, source) = fixture_directory();
    let (_copy_parent, path) = location();
    drop(Directory::open(&path).unwrap());
    let db = Connection::open(source.join("controller.sqlite")).unwrap();
    db.pragma_update(None, "journal_mode", "WAL").unwrap();
    db.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    db.execute("INSERT INTO users VALUES (x'00000000000000000000000000000012', 'wal@example.test', '$argon2id$v=19$test', 1, 1000, NULL)", []).unwrap();
    // A copy taken while the writer is alive: the row is only in the WAL.
    for name in ["controller.sqlite", "controller.sqlite-wal"] {
        std::fs::copy(source.join(name), path.join(name)).unwrap();
        std::fs::set_permissions(path.join(name), std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    drop(db);
    migrate_directory(&path).unwrap().unwrap();
    assert!(!path.join("controller.sqlite-wal").exists());
    assert!(!path.join("controller.sqlite-shm").exists());
    let db = Connection::open(path.join("controller.sqlite")).unwrap();
    let email: String = db
        .query_row(
            "SELECT email FROM users WHERE email LIKE 'wal@%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(email, "wal@example.test");
    validate_as(&db, VERSION, SQL).unwrap();
}

#[test]
fn only_the_newest_backups_are_kept() {
    let (_parent, path) = fixture_directory();
    for stamp in [1, 2, 3] {
        let name = format!("controller.sqlite.v7-{stamp}");
        std::fs::write(path.join(&name), b"older").unwrap();
        std::fs::set_permissions(path.join(&name), std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    migrate_directory(&path).unwrap().unwrap();
    let kept = backups(&path);
    assert_eq!(kept.len(), crate::directory::MIGRATION_BACKUPS);
    assert!(!kept.contains(&"controller.sqlite.v7-1".to_owned()));
    assert!(
        kept.iter()
            .any(|name| name.starts_with("controller.sqlite.v8-"))
    );
}

/// A chain of levels: the synthetic version 10 drops a table, adds one and
/// rewrites one with a NOT NULL column derived from existing rows. 8 → 10 in
/// one run equals 8 → 9 followed by 9 → 10.
#[test]
fn a_chain_of_levels_equals_its_levels_one_after_another() {
    static V10: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let start = SQL.find("-- Consumed subscriber identifiers").unwrap();
        let end = SQL.find("CREATE TABLE observation_events").unwrap();
        let mut sql = format!("{}{}", &SQL[..start], &SQL[end..]);
        sql = sql.replace(
            "    failures INTEGER NOT NULL CHECK(failures BETWEEN 0 AND 16),\n    PRIMARY KEY(catalog, trust)",
            "    failures INTEGER NOT NULL CHECK(failures BETWEEN 0 AND 16),\n    origin TEXT NOT NULL,\n    PRIMARY KEY(catalog, trust)",
        );
        assert!(sql.contains("origin TEXT NOT NULL"));
        sql.push_str("\nCREATE TABLE added (id INTEGER PRIMARY KEY) STRICT;\n");
        sql
    });
    fn chain_history(version: i64) -> Option<&'static str> {
        match version {
            10 => Some(V10.as_str()),
            version => history(version),
        }
    }
    static CHAIN: &[Level] = &[
        super::v9::LEVEL,
        Level {
            to: 10,
            new_tables: &["added"],
            dropped_tables: &["observation_subscriber_ids"],
            transforms: &[(
                "rule_set_catalog_checks",
                "INSERT INTO main.rule_set_catalog_checks SELECT catalog,trust,problem,checked_at,failures,'checked' FROM old.rule_set_catalog_checks ORDER BY catalog,trust",
            )],
        },
    ];
    let ten = Schema {
        version: 10,
        levels: CHAIN,
        history: chain_history,
    };
    let nine = Schema {
        version: 9,
        levels: &CHAIN[..1],
        history: chain_history,
    };
    let contents = |path: &Path| {
        let db = Connection::open(path.join("controller.sqlite")).unwrap();
        validate_as(&db, 10, V10.as_str()).unwrap();
        tables(&db, "main")
            .unwrap()
            .into_iter()
            .map(|table| {
                let rows: i64 = db
                    .query_row(
                        &format!("SELECT count(*) FROM {}", quote(&table)),
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                (table, rows)
            })
            .filter(|(table, _)| table != "schema_migrations")
            .collect::<Vec<_>>()
    };
    let (_one, chained) = fixture_directory();
    let mut directory = Directory::open(&chained).unwrap();
    let migrated = prepare_as(&mut directory, Migration::Automatic, &ten)
        .unwrap()
        .unwrap();
    drop(directory);
    assert_eq!((migrated.from, migrated.to), (8, 10));
    let (_two, stepped) = fixture_directory();
    let mut directory = Directory::open(&stepped).unwrap();
    prepare_as(&mut directory, Migration::Automatic, &nine)
        .unwrap()
        .unwrap();
    prepare_as(&mut directory, Migration::Automatic, &ten)
        .unwrap()
        .unwrap();
    drop(directory);
    // The fixtures differ only in random bytes: the same tables and rows.
    assert_eq!(contents(&chained), contents(&stepped));
    for path in [&chained, &stepped] {
        let db = Connection::open(path.join("controller.sqlite")).unwrap();
        let origin: String = db
            .query_row("SELECT origin FROM rule_set_catalog_checks", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(origin, "checked");
        let records: Vec<(i64, i64)> = db
            .prepare("SELECT from_version,to_version FROM schema_migrations ORDER BY sequence")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let expected = if path == &chained {
            vec![(8, 10)]
        } else {
            vec![(8, 9), (9, 10)]
        };
        assert_eq!(records, expected);
    }
    // Only the original file of the chained run is a backup: no intermediate
    // staging file remains.
    assert_eq!(backups(&chained).len(), 1);
    assert_eq!(backups(&stepped).len(), 2);
}

/// The App level test's input: the database a real App wrote at `path`,
/// converted to what schema 8 would have written (every table but the one
/// schema 9 added). The same copy as a migration level, run backwards.
pub(crate) fn downgrade_to_v8(path: &Path) {
    const BACK: Level = Level {
        to: 8,
        new_tables: &[],
        dropped_tables: &["schema_migrations"],
        transforms: &[],
    };
    let database = path.join("controller.sqlite");
    let staging = path.join("controller.sqlite.downgrade");
    {
        let db = Connection::open(&database).unwrap();
        db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
        db.pragma_update(None, "journal_mode", "DELETE").unwrap();
    }
    let mut db = Connection::open(&staging).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    db.execute("ATTACH DATABASE ?1 AS old", [read_only_uri(&database)])
        .unwrap();
    let transaction = db.transaction().unwrap();
    transaction.execute_batch(history(8).unwrap()).unwrap();
    transaction
        .pragma_update(None, "application_id", APPLICATION_ID)
        .unwrap();
    transaction.pragma_update(None, "user_version", 8).unwrap();
    copy(&transaction, &BACK).unwrap();
    validate_as(&transaction, 8, history(8).unwrap()).unwrap();
    transaction.commit().unwrap();
    db.execute_batch("DETACH DATABASE old").unwrap();
    drop(db);
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::rename(&staging, &database).unwrap();
    for name in ["controller.sqlite-wal", "controller.sqlite-shm"] {
        let _ = std::fs::remove_file(path.join(name));
    }
}
