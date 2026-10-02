//! Original v1 DDL with active/revoked identities and consumed token bindings.
use meshway_protocol::{EnrollAgentRequest, local};
use rusqlite::params;

pub(crate) fn create(path: &std::path::Path) -> (String, Vec<EnrollAgentRequest>, Vec<String>) {
    let admin = local::credential().unwrap();
    local::atomic_write(&path.join("admin.token"), admin.as_bytes(), false).unwrap();
    local::atomic_write(&path.join("controller.sqlite3"), &[], false).unwrap();
    let db = rusqlite::Connection::open(path.join("controller.sqlite3")).unwrap();
    db.execute_batch("CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK (id = 1), admin_hash BLOB NOT NULL);
        CREATE TABLE nodes (id TEXT PRIMARY KEY, credential_hash BLOB NOT NULL UNIQUE, name TEXT NOT NULL, agent_version TEXT NOT NULL, registered_at TEXT NOT NULL, last_seen TEXT NOT NULL, revoked_at TEXT);
        CREATE TABLE enrollment_tokens (id TEXT PRIMARY KEY, token_hash BLOB NOT NULL UNIQUE, expires_at INTEGER NOT NULL, used_node_id TEXT REFERENCES nodes(id), binding_hash BLOB, CHECK ((used_node_id IS NULL) = (binding_hash IS NULL)));
        PRAGMA user_version = 1;").unwrap();
    db.execute(
        "INSERT INTO settings VALUES (1, ?1)",
        [local::digest(&admin)],
    )
    .unwrap();
    let mut requests = Vec::new();
    let mut tokens = Vec::new();
    for (id, revoked) in [
        ("z", None),
        ("a", Some("2026-09-30T00:00:00Z")),
        ("m", None),
    ] {
        let request = super::super::request(id);
        let token = local::credential().unwrap();
        db.execute("INSERT INTO nodes VALUES (?1, ?2, ?1, 'v1', '2026-09-30T00:00:00Z', '2026-09-30T00:00:00Z', ?3)", params![id, local::digest(&request.node_credential), revoked]).unwrap();
        db.execute(
            "INSERT INTO enrollment_tokens VALUES (?1, ?2, 0, ?1, ?3)",
            params![
                id,
                local::digest(&token),
                local::digest(&serde_json::to_string(&request).unwrap())
            ],
        )
        .unwrap();
        requests.push(request);
        tokens.push(token);
    }
    (admin, requests, tokens)
}
