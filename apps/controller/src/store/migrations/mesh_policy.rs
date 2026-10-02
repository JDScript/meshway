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
    tx.execute_batch(include_str!("sql/schema_v12.sql"))?;
    validate(tx)
}

pub(super) fn validate(db: &Connection) -> Result<(), Error> {
    let reference = Connection::open_in_memory()?;
    reference.execute_batch(include_str!("sql/schema_v12.sql"))?;
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

#[cfg(test)]
mod tests {
    use crate::store::migrations::execution_and_accounts::upgrade;
    use crate::store::test_support::fixtures::release::{NODE, v11};

    #[test]
    fn v12_schema_gate_rolls_back_fragments_and_preserves_v11_rows() {
        let mut db = v11();
        let before: u32 = db
            .query_row("SELECT revision FROM configuration", [], |r| r.get(0))
            .unwrap();
        db.execute_batch("CREATE TABLE mesh_execution_receipts(dummy TEXT)")
            .unwrap();
        assert!(upgrade(&mut db).is_err());
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            11
        );
        assert!(
            !db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='network_policy_sources')",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
        db.execute_batch("DROP TABLE mesh_execution_receipts")
            .unwrap();
        upgrade(&mut db).unwrap();
        assert_eq!(
            db.query_row("SELECT revision FROM configuration", [], |r| r
                .get::<_, u32>(0))
                .unwrap(),
            before
        );
        assert_eq!(
            db.query_row("SELECT mesh_ipv4 FROM nodes WHERE id=?1", [NODE], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "100.96.0.2"
        );
        assert!(
            !db.prepare("SELECT 1 FROM network_policy_sources WHERE source_mode='mesh_v3'")
                .unwrap()
                .exists([])
                .unwrap()
        );
        upgrade(&mut db).unwrap();
        db.execute_batch("ALTER TABLE mesh_execution_receipts ADD COLUMN unexpected TEXT")
            .unwrap();
        assert!(upgrade(&mut db).is_err());
    }
}
