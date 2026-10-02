use crate::Error;
use rusqlite::{Connection, Transaction};

const TABLES: [&str; 3] = [
    "network_policy_sources",
    "mesh_execution_capabilities",
    "mesh_execution_receipts",
];

pub(super) fn install(tx: &Transaction<'_>) -> Result<(), Error> {
    for name in TABLES {
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name=?1)",
            [name],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::Configuration(
                "unversioned policy v3 schema fragment",
            ));
        }
    }
    tx.execute_batch(include_str!("schema_v12.sql"))?;
    validate(tx)
}

pub(super) fn validate(db: &Connection) -> Result<(), Error> {
    let reference = Connection::open_in_memory()?;
    reference.execute_batch(include_str!("schema_v12.sql"))?;
    for name in TABLES {
        let expected: String =
            reference.query_row("SELECT sql FROM sqlite_schema WHERE name=?1", [name], |r| {
                r.get(0)
            })?;
        let actual: Option<String> = db
            .query_row("SELECT sql FROM sqlite_schema WHERE name=?1", [name], |r| {
                r.get(0)
            })
            .ok();
        if actual.as_ref() != Some(&expected) {
            return Err(Error::Configuration("unsupported policy v3 schema"));
        }
    }
    Ok(())
}
