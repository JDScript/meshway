//! Pure validation of the supplied connection. A separate in-memory reference
//! checks every column/PK/FK/index/trigger definition; no schema is modified.
use crate::Error;
use rusqlite::{Connection, OptionalExtension, types::Value};
use std::collections::BTreeMap;

const TABLES: [&str; 4] = [
    "latency_runtime_epochs",
    "latency_samples",
    "latency_admissions",
    "latency_coverage",
];

#[derive(Debug, PartialEq)]
struct Shape {
    sql: String,
    columns: Vec<Vec<Value>>,
    foreign_keys: Vec<Vec<Value>>,
    indexes: BTreeMap<String, (Vec<Value>, Vec<Vec<Value>>)>,
    triggers: BTreeMap<(String, String), (String, String)>,
}

fn rows(connection: &Connection, sql: &str) -> Result<Vec<Vec<Value>>, Error> {
    let mut statement = connection.prepare(sql)?;
    let count = statement.column_count();
    Ok(statement
        .query_map([], |row| (0..count).map(|i| row.get(i)).collect())?
        .collect::<Result<_, _>>()?)
}

fn shape(connection: &Connection, table: &str) -> Result<Shape, Error> {
    let sql: Option<String> = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |r| r.get(0),
        )
        .optional()?;
    let sql = sql.ok_or(Error::Configuration("missing latency schema table"))?;
    let mut indexes = BTreeMap::new();
    for index in rows(connection, &format!("PRAGMA index_list({table})"))? {
        let Value::Text(name) = &index[1] else {
            return Err(Error::Configuration("invalid latency index metadata"));
        };
        // PRAGMA index_list seq is enumeration order, not schema semantics.
        indexes.insert(
            name.clone(),
            (
                index[1..].to_vec(),
                rows(
                    connection,
                    &format!("PRAGMA index_xinfo(\"{}\")", name.replace('"', "\"\"")),
                )?,
            ),
        );
    }
    let triggers = connection.prepare(
        "SELECT 'main',name,tbl_name,sql FROM sqlite_master WHERE type='trigger' AND tbl_name=?1 COLLATE NOCASE
         UNION ALL SELECT 'temp',name,tbl_name,sql FROM sqlite_temp_master WHERE type='trigger' AND tbl_name=?1 COLLATE NOCASE",
    )?.query_map([table], |r| Ok(((r.get::<_,String>(0)?,r.get::<_,String>(1)?),(r.get::<_,String>(2)?,r.get::<_,String>(3)?))))?
        .collect::<Result<BTreeMap<_,_>,_>>()?;
    Ok(Shape {
        sql: sql
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_uppercase(),
        columns: rows(connection, &format!("PRAGMA table_xinfo({table})"))?,
        foreign_keys: rows(connection, &format!("PRAGMA foreign_key_list({table})"))?,
        indexes,
        // Compare the complete set, ownership and exact SQL (including literal
        // case). The current reference has no history triggers at all. Base and
        // account triggers attached to other tables are deliberately untouched.
        triggers,
    })
}

pub(crate) fn validate_schema(connection: &Connection) -> Result<(), Error> {
    let reference = Connection::open_in_memory()?;
    reference.execute_batch(include_str!("schema.sql"))?;
    for table in TABLES {
        if shape(connection, table)? != shape(&reference, table)? {
            return Err(Error::Configuration(
                "incompatible latency columns, PK/AUTOINCREMENT, UNIQUE, FK, indexes or triggers",
            ));
        }
    }
    if connection.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(Error::Configuration(
            "latency migration foreign key integrity failure",
        ));
    }
    Ok(())
}

pub(crate) fn validate_if_present(connection: &Connection) -> Result<(), Error> {
    let any: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name IN ('latency_runtime_epochs','latency_samples','latency_admissions','latency_coverage'))",[],|r|r.get(0))?;
    if any {
        super::validate_schema(connection)?;
    }
    Ok(())
}
