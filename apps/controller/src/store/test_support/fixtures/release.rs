//! Independent pre-account rows, including opaque persisted material bytes.
use rusqlite::{Connection, params};
pub(crate) const NETWORK: &str = "0123456789abcdef0123456789abcdef";
pub(crate) const NODE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub(crate) const PEER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

pub(crate) fn v10() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    db.execute_batch(include_str!("schema_v9.sql")).unwrap();
    db.execute_batch(include_str!("schema_v10.sql")).unwrap();
    db.execute(
        "INSERT INTO networks VALUES(?1,'2026-10-01T00:00:00Z',1)",
        [NETWORK],
    )
    .unwrap();
    db.execute("INSERT INTO nodes(id,network_id,credential_hash,name,agent_version,registered_at,last_seen,mesh_ipv4,mesh_ipv6,country,acknowledged_revision,acknowledged_digest) VALUES(?1,?2,?3,'legacy','old-build','2026-10-01T00:00:00Z','2026-10-01T00:00:00Z','100.96.0.2','fd73::2','US',42,'saved-digest')",params![NODE,NETWORK,vec![7_u8;32]]).unwrap();
    db.execute("INSERT INTO nodes(id,network_id,credential_hash,name,agent_version,registered_at,last_seen,mesh_ipv4,mesh_ipv6,acknowledged_revision) VALUES(?1,?2,?3,'peer','old-build','2026-10-01T00:00:00Z','2026-10-01T00:00:00Z','100.96.0.3','fd73::3',42)",params![PEER,NETWORK,vec![8_u8;32]]).unwrap();
    db.execute(
        "INSERT INTO configuration VALUES(?1,1,42,'100.96.0.0/16','fd73::/64',?2,?3)",
        params![
            NETWORK,
            r#"{"default_action":"direct","rules":[]}"#,
            "// preserve comments\n{\"default_action\":\"direct\",\"rules\":[]}"
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO connection_identities VALUES(?1,?2,?3,'[]')",
        params![NODE, NETWORK, "11".repeat(32)],
    )
    .unwrap();
    db.execute(
        "INSERT INTO connection_runtime_leases VALUES(?1,?2,'old-instance',3)",
        params![NODE, NETWORK],
    )
    .unwrap();
    db.execute(
        "INSERT INTO tag_definitions VALUES(?1,'tag:existing','preserve this description')",
        [NETWORK],
    )
    .unwrap();
    db.execute(
        "INSERT INTO node_tags VALUES(?1,?2,'tag:existing')",
        params![NETWORK, NODE],
    )
    .unwrap();
    db.execute(
        "INSERT INTO managed_listeners VALUES(?1,'existing',?2,'{}',4,'fingerprint',?3,?4)",
        params![NETWORK, NODE, vec![5_u8; 12], vec![9_u8; 80]],
    )
    .unwrap();
    db
}

pub(crate) fn v11() -> Connection {
    let db = v10();
    db.execute_batch(include_str!("schema_v11.sql")).unwrap();
    db
}
