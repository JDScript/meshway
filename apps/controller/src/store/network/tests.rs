use meshway_protocol::*;
use rusqlite::{Connection, types::Value};

use super::super::{
    Store,
    tests::{directory, enroll, request},
};
use crate::{Error, NetworkOptions};

fn old_rows(connection: &Connection) -> Vec<(String, Vec<Vec<Value>>)> {
    let tables: Vec<String> = connection.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != 'network_identity' ORDER BY name")
        .unwrap().query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    tables
        .into_iter()
        .map(|table| {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM \"{table}\" ORDER BY rowid"))
                .unwrap();
            let count = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..count)
                        .map(|index| row.get(index))
                        .collect::<Result<Vec<Value>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (table, rows)
        })
        .collect()
}

#[test]
fn additive_v6_upgrade_preserves_all_rows_revision_and_old_payloads_then_id_survives_restart() {
    let dir = directory();
    let mut store = Store::open_v8(dir.path(), &NetworkOptions::default()).unwrap();
    let first = request("one");
    let second = request("two");
    let node = enroll(&mut store, &first);
    let peer = enroll(&mut store, &second);
    store.revoke(&peer.id).unwrap();
    store.connection.execute("UPDATE nodes SET country='US', acknowledged_revision=2, acknowledged_digest=?1 WHERE id=?2",
        rusqlite::params!["a".repeat(64), node.id]).unwrap();
    // Real persisted identity/capability rows are retained even though DNS does
    // not filter inventory by key presence.
    let key = x_key();
    store
        .connection
        .execute(
            "INSERT INTO connection_identities VALUES(?1,?2,'[]')",
            rusqlite::params![node.id, key],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO tag_definitions VALUES ('tag:test','preserve')",
            [],
        )
        .unwrap();
    store
        .connection
        .execute("INSERT INTO node_tags VALUES (?1,'tag:test')", [&node.id])
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO connection_runtime_leases VALUES (?1,?2,3)",
            rusqlite::params![node.id, "b".repeat(64)],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO connection_runtime_reports VALUES (?1,4,?2,'2026-10-01T00:00:00Z')",
            rusqlite::params![node.id, b"preserve opaque persisted report".as_slice()],
        )
        .unwrap();
    let configuration =
        serde_json::to_vec(&store.configuration(&first.node_credential).unwrap()).unwrap();
    let dataplane =
        serde_json::to_vec(&store.agent_dataplane(&first.node_credential).unwrap()).unwrap();
    let rows = old_rows(&store.connection);
    let revision = store.revision().unwrap();
    let admin = std::fs::read(dir.path().join("admin.token")).unwrap();
    store
        .connection
        .execute_batch("ALTER TABLE enrollment_tokens DROP COLUMN device_binding; DROP TABLE network_identity; PRAGMA user_version=6;")
        .unwrap();
    drop(store);
    let mut store = Store::open_v8(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(old_rows(&store.connection), rows);
    assert_eq!(
        std::fs::read(dir.path().join("admin.token")).unwrap(),
        admin
    );
    assert_eq!(
        serde_json::to_vec(&store.configuration(&first.node_credential).unwrap()).unwrap(),
        configuration
    );
    assert_eq!(
        serde_json::to_vec(&store.agent_dataplane(&first.node_credential).unwrap()).unwrap(),
        dataplane
    );
    let network = store.network().unwrap();
    assert_eq!(network.revision, revision);
    assert_eq!(network.name, "Meshway");
    assert_eq!(network.dns, MeshDnsConfiguration::default());
    assert_eq!(network.network_id.len(), 32);
    let dns = store.agent_dns(&first.node_credential).unwrap();
    assert_eq!(dns.payload.revision, revision);
    assert!(dns.payload.records.is_empty());
    drop(store);
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(store.network().unwrap().network_id, network.network_id);
    assert_eq!(
        store.agent_dns(&first.node_credential).unwrap().digest,
        dns.digest
    );
}

fn x_key() -> String {
    // Valid public key already used throughout the controller behavior tests.
    "09".to_owned() + &"00".repeat(31)
}

#[test]
fn global_cas_normalized_noop_and_zone_withdrawal_are_durable() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let req = request("BÜCHER");
    enroll(&mut store, &req);
    let initial = store.network().unwrap();
    let mut save = SaveNetworkRequest {
        expected_revision: initial.revision,
        name: "网络".into(),
        dns: MeshDnsConfiguration {
            enabled: true,
            zone: "BÜCHER.Test.".into(),
            ttl_seconds: 30,
        },
    };
    let saved = store.save_network(&save).unwrap();
    assert_eq!(saved.revision, initial.revision + 1);
    assert_eq!(saved.network, initial.network);
    let enabled = store.agent_dns(&req.node_credential).unwrap();
    assert_eq!(enabled.payload.records.len(), 1);
    assert_eq!(
        enabled.payload.records[0].aliases,
        ["xn--bcher-kva.xn--bcher-kva.test"]
    );
    assert!(matches!(store.save_network(&save), Err(Error::Conflict)));
    save.expected_revision = saved.revision;
    assert_eq!(store.save_network(&save).unwrap().revision, saved.revision);
    assert_eq!(
        store.agent_dns(&req.node_credential).unwrap().digest,
        enabled.digest
    );
    save.dns.zone = "new.test".into();
    let changed = store.save_network(&save).unwrap();
    let moved = store.agent_dns(&req.node_credential).unwrap();
    assert_ne!(enabled.digest, moved.digest);
    assert!(
        moved
            .payload
            .records
            .iter()
            .all(|r| r.canonical_name.ends_with(".new.test"))
    );
    save.expected_revision = changed.revision;
    save.dns.enabled = false;
    let disabled = store.save_network(&save).unwrap();
    assert_eq!(disabled.revision, changed.revision + 1);
    let withdrawn = store.agent_dns(&req.node_credential).unwrap();
    assert!(withdrawn.payload.records.is_empty());
    assert_ne!(withdrawn.digest, moved.digest);
    save.expected_revision = disabled.revision;
    save.name = "display rename".into();
    store.save_network(&save).unwrap();
    assert_ne!(
        store.agent_dns(&req.node_credential).unwrap().digest,
        withdrawn.digest
    );
    // Other global mutations make network CAS stale as well.
    enroll(&mut store, &request("peer"));
    assert!(matches!(store.save_network(&save), Err(Error::Conflict)));
}
