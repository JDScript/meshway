//! Recognize only the complete known unversioned account fragment. This is a
//! release-migration gate, not an IF NOT EXISTS repair of an unknown schema.
use crate::{Error, accounts};
use rusqlite::{Connection, types::Value};
use std::collections::BTreeMap;

const TABLES: [&str; 7] = [
    "account_users",
    "account_memberships",
    "account_invitations",
    "account_sessions",
    "account_password_resets",
    "node_ownership",
    "device_account_bindings",
];
const OBJECTS: &str = "tbl_name IN ('account_users','account_memberships','account_invitations','account_sessions','account_password_resets','node_ownership','device_account_bindings') OR name IN ('ownership_new_node','account_enrollment_scope')";
// SQLite allows a trigger and index to share a name. Preserve object type in
// its identity, including implicit indexes whose SQL is NULL, so no collected
// object can silently overwrite a different schema object.
type Definitions = BTreeMap<(String, String), (String, Option<String>)>;

fn incompatible() -> Error {
    Error::Configuration("incomplete or incompatible account integration schema")
}

pub(in crate::store) fn present(db: &Connection) -> Result<bool, Error> {
    Ok(db.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE {OBJECTS})"),
        [],
        |r| r.get(0),
    )?)
}

// Whitespace outside quoted literals is insignificant for recognized DDL.
// Preserve string/identifier contents and doubled quotes exactly; do not erase
// constraints or accept a same-name trigger with a different predicate/body.
fn canonical(sql: &str) -> String {
    let mut out = String::new();
    let mut quote = None;
    let mut space = false;
    let mut chars = sql.trim().trim_end_matches(';').chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(end) = quote {
            out.push(ch);
            if ch == end {
                if chars.peek() == Some(&end) {
                    out.push(chars.next().expect("peeked quote"));
                } else {
                    quote = None;
                }
            }
        } else if ch.is_ascii_whitespace() {
            space = true;
        } else {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(ch);
            if ch == '\'' || ch == '"' || ch == '`' {
                quote = Some(ch);
            } else if ch == '[' {
                quote = Some(']');
            }
        }
    }
    out
}

fn definitions(db: &Connection) -> Result<Definitions, Error> {
    let mut q = db.prepare(&format!(
        "SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE {OBJECTS} ORDER BY type,name"
    ))?;
    Ok(q.query_map([], |r| {
        Ok((
            (r.get::<_, String>(0)?, r.get::<_, String>(1)?),
            (
                r.get(2)?,
                r.get::<_, Option<String>>(3)?.map(|sql| canonical(&sql)),
            ),
        ))
    })?
    .collect::<rusqlite::Result<_>>()?)
}

fn rows(db: &Connection, sql: &str) -> Result<Vec<Vec<Value>>, Error> {
    let mut q = db.prepare(sql)?;
    let columns = q.column_count();
    Ok(q.query_map([], |r| {
        (0..columns)
            .map(|i| r.get(i))
            .collect::<rusqlite::Result<Vec<_>>>()
    })?
    .collect::<rusqlite::Result<_>>()?)
}

pub(in crate::store) fn validate(db: &Connection) -> Result<(), Error> {
    if !db.pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))? {
        return Err(incompatible());
    }
    // Build the expected structural manifest from the authoritative fragments,
    // with the actual v9 parent contracts. No serving database DDL is performed.
    let expected = Connection::open_in_memory()?;
    expected.execute_batch(include_str!("../schema_v9.sql"))?;
    expected.execute_batch(accounts::SCHEMA_FRAGMENT)?;
    expected.execute_batch(include_str!("../ownership.sql"))?;
    if definitions(db)? != definitions(&expected)? {
        return Err(incompatible());
    }
    for table in TABLES {
        // Compare parsed columns/PKs and ordered composite FK groups, not names
        // alone. Exact table definitions additionally cover CHECK/UNIQUE clauses.
        for pragma in ["table_xinfo", "foreign_key_list"] {
            let sql = format!("PRAGMA {pragma}('{table}')");
            if rows(db, &sql)? != rows(&expected, &sql)? {
                return Err(incompatible());
            }
        }
    }
    for index in [
        "account_active_membership",
        "account_membership_subject",
        "account_enrollment_scope",
    ] {
        let sql = format!("PRAGMA index_xinfo('{index}')");
        if rows(db, &sql)? != rows(&expected, &sql)? {
            return Err(incompatible());
        }
    }
    // Verify partial/unique flags and actual index columns as well as the WHERE
    // predicate in its recognized SQL. Other P2a indexes are outside this gate.
    for table in ["account_memberships", "enrollment_tokens"] {
        let sql = format!(
            "SELECT name,\"unique\",origin,partial FROM pragma_index_list('{table}') WHERE name IN ('account_active_membership','account_membership_subject','account_enrollment_scope') ORDER BY name"
        );
        if rows(db, &sql)? != rows(&expected, &sql)? {
            return Err(incompatible());
        }
    }
    if db.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(incompatible());
    }
    if db.prepare("SELECT n.id FROM nodes n LEFT JOIN node_ownership o ON o.node_id=n.id AND o.network_id=n.network_id GROUP BY n.network_id,n.id HAVING COUNT(o.node_id)<>1")?.exists([])? {return Err(incompatible());}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_preserves_both_types_and_implicit_indexes_for_legal_same_names() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE account_users(id TEXT PRIMARY KEY); CREATE INDEX account_twin ON account_users(id); CREATE TRIGGER account_twin BEFORE INSERT ON account_users BEGIN SELECT 1; END;").unwrap();
        let collected = definitions(&db).unwrap();
        assert_eq!(collected.len(), 4);
        assert!(collected.contains_key(&("table".into(), "account_users".into())));
        assert!(collected.contains_key(&("index".into(), "account_twin".into())));
        assert!(collected.contains_key(&("trigger".into(), "account_twin".into())));
        assert_eq!(
            collected
                .get(&("index".into(), "sqlite_autoindex_account_users_1".into()))
                .unwrap()
                .1,
            None
        );
        let expected = Connection::open_in_memory().unwrap();
        // A different creation order must preserve the same complete manifest.
        expected.execute_batch("CREATE TABLE account_users(id TEXT PRIMARY KEY); CREATE TRIGGER account_twin BEFORE INSERT ON account_users BEGIN SELECT 1; END; CREATE INDEX account_twin ON account_users(id);").unwrap();
        assert_eq!(collected, definitions(&expected).unwrap());
    }

    #[test]
    fn complete_schema_with_unrelated_same_name_extension_objects_is_compatible() {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        db.execute_batch(include_str!("../schema_v9.sql")).unwrap();
        db.execute_batch(accounts::SCHEMA_FRAGMENT).unwrap();
        db.execute_batch(include_str!("../ownership.sql")).unwrap();
        validate(&db).unwrap();
        db.execute_batch("CREATE TABLE unrelated_extension(id TEXT PRIMARY KEY); CREATE TRIGGER unrelated_twin BEFORE INSERT ON unrelated_extension BEGIN SELECT 1; END; CREATE INDEX unrelated_twin ON unrelated_extension(id);").unwrap();
        validate(&db).unwrap();
        // Scope is all owned table objects, not a whitelist of expected names.
        db.execute_batch("CREATE TRIGGER unexpected_owned BEFORE INSERT ON account_sessions BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(validate(&db).is_err());
    }
}
