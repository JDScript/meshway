//! One atomic v10 -> v11 installation of accounts, ownership and probe history.
//! Recognized unversioned development fragments may be adopted without rewriting
//! their rows; partial or unknown structures are rejected before versioning.
use super::{account_schema, latency_history};
use crate::Error;
use rusqlite::{Connection, TransactionBehavior, params};

pub(super) const VERSION: i64 = 11;

pub(super) fn upgrade(connection: &mut Connection) -> Result<(), Error> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
    match version {
        10 => {
            latency_history::validate_if_present(&tx)?;
            if account_schema::present(&tx)? {
                account_schema::validate(&tx)?;
            } else {
                account_schema::install(&tx)?;
            }
            let history_present: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name GLOB 'latency_*' OR tbl_name GLOB 'latency_*')",
                [],
                |row| row.get(0),
            )?;
            if !history_present {
                tx.execute_batch(include_str!("latency_history/schema.sql"))?;
            }
            // A pre-v11 lease has no recorded acquisition time. Establish a
            // conservative activation floor, not a fabricated earlier probe or
            // lease timestamp. Preserve known starts from recognized fragments.
            tx.execute(
                "INSERT OR IGNORE INTO latency_runtime_epochs(node_id,network_id,epoch,started_ms)
                 SELECT node_id,network_id,epoch,?1 FROM connection_runtime_leases",
                params![chrono::Utc::now().timestamp_millis()],
            )?;
            validate(&tx)?;
            tx.pragma_update(None, "user_version", VERSION)?;
        }
        VERSION => validate(&tx)?,
        _ => {
            return Err(Error::Configuration(
                "account and history migration requires schema 10 or 11",
            ));
        }
    }
    tx.commit()?;
    Ok(())
}

fn validate(connection: &Connection) -> Result<(), Error> {
    account_schema::validate(connection)?;
    latency_history::validate_schema(connection)?;
    if connection.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(Error::Configuration("schema 11 foreign key failure"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::types::Value;

    const NETWORK: &str = "0123456789abcdef0123456789abcdef";
    const NODE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const PEER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn v10() -> Connection {
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
        // Synthetic opaque material bytes prove preservation without pretending
        // this fixture generated a real private certificate.
        db.execute(
            "INSERT INTO managed_listeners VALUES(?1,'existing',?2,'{}',4,'fingerprint',?3,?4)",
            params![NETWORK, NODE, vec![5_u8; 12], vec![9_u8; 80]],
        )
        .unwrap();
        db
    }

    fn legacy_rows(db: &Connection) -> Vec<(String, Vec<Vec<Value>>)> {
        let mut tables: Vec<_> = super::super::networks::TABLES
            .iter()
            .map(|(name, _)| *name)
            .collect();
        tables.extend([
            "managed_listeners",
            "managed_transport_capabilities",
            "managed_enrollment_intents",
        ]);
        tables
            .into_iter()
            .map(|name| {
                let mut query = db
                    .prepare(&format!("SELECT * FROM {name} ORDER BY 1,2"))
                    .unwrap();
                let columns = query.column_count();
                let rows = query
                    .query_map([], |row| {
                        (0..columns)
                            .map(|column| row.get(column))
                            .collect::<rusqlite::Result<Vec<Value>>>()
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                (name.to_owned(), rows)
            })
            .collect()
    }

    #[test]
    fn v10_upgrade_preserves_legacy_rows_and_sets_only_a_history_activation_floor() {
        let mut db = v10();
        let before = legacy_rows(&db);
        let lower = chrono::Utc::now().timestamp_millis();
        upgrade(&mut db).unwrap();
        let upper = chrono::Utc::now().timestamp_millis();
        assert_eq!(legacy_rows(&db), before);
        assert_eq!(
            db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            VERSION
        );
        let floor: i64 = db
            .query_row(
                "SELECT started_ms FROM latency_runtime_epochs WHERE node_id=?1 AND epoch=3",
                [NODE],
                |row| row.get(0),
            )
            .unwrap();
        assert!((lower..=upper).contains(&floor));
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM account_users", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT membership_id FROM node_ownership WHERE node_id=?1",
                [NODE],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap(),
            None
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM latency_samples", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        upgrade(&mut db).unwrap();
        assert_eq!(legacy_rows(&db), before);
        assert_eq!(
            db.query_row(
                "SELECT started_ms FROM latency_runtime_epochs WHERE node_id=?1",
                [NODE],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            floor
        );
    }

    #[test]
    fn recognized_unversioned_fragments_keep_their_epoch_and_row_cursor() {
        let mut db = v10();
        let measured_at = chrono::Utc::now() - chrono::Duration::seconds(1);
        let completed_at = measured_at + chrono::Duration::milliseconds(1);
        let sample = meshway_protocol::LatencySample {
            measurement_id: "d".repeat(64),
            peer_node_id: PEER.into(),
            direction: meshway_protocol::LatencyDirection::SourceToPeer,
            runtime_epoch: 3,
            revision: 42,
            digest: "f".repeat(64),
            sampler_generation: "e".repeat(64),
            measured_at,
            completed_at,
            outcome: meshway_protocol::LatencyOutcome::Success,
            failure_reasons: vec![],
            channel_associations: vec![meshway_protocol::LatencyChannelAssociation {
                transport: meshway_protocol::ConnectionTransport::NativeUdp,
                initiator_node_id: NODE.into(),
                listener_id: "legacy-listener".into(),
                candidate_id: Some("legacy-candidate".into()),
            }],
            rtt_us: Some(1000),
        };
        let body = serde_json::to_vec(&sample).unwrap();
        {
            let tx = db.transaction().unwrap();
            account_schema::install(&tx).unwrap();
            tx.execute_batch(include_str!("latency_history/schema.sql"))
                .unwrap();
            tx.execute(
                "INSERT INTO latency_runtime_epochs VALUES(?1,?2,3,12345)",
                params![NODE, NETWORK],
            )
            .unwrap();
            tx.execute("INSERT INTO latency_samples(row_id,network_id,source_node_id,peer_node_id,measurement_id,runtime_epoch,measured_ms,received_ms,body) VALUES(73,?1,?2,?3,?4,3,?5,?6,?7)",params![NETWORK,NODE,PEER,sample.measurement_id,measured_at.timestamp_millis(),completed_at.timestamp_millis()+1,body]).unwrap();
            tx.commit().unwrap();
        }
        let before = legacy_rows(&db);
        upgrade(&mut db).unwrap();
        assert_eq!(legacy_rows(&db), before);
        assert_eq!(
            db.query_row(
                "SELECT started_ms FROM latency_runtime_epochs WHERE node_id=?1",
                [NODE],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            12345
        );
        assert_eq!(
            db.query_row("SELECT row_id,body FROM latency_samples", [], |row| Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?
            )))
            .unwrap(),
            (73, body)
        );
    }

    #[test]
    fn partial_history_rolls_back_account_install_and_schema_version() {
        let mut db = v10();
        let before = legacy_rows(&db);
        db.execute_batch("CREATE TABLE latency_samples(incomplete INTEGER)")
            .unwrap();
        assert!(upgrade(&mut db).is_err());
        assert_eq!(legacy_rows(&db), before);
        assert_eq!(
            db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            10
        );
        assert!(!account_schema::present(&db).unwrap());
    }

    #[test]
    fn version_eleven_restart_rejects_partial_schema_without_repairing_it() {
        let mut db = v10();
        upgrade(&mut db).unwrap();
        db.execute_batch("DROP TRIGGER ownership_new_node").unwrap();
        assert!(upgrade(&mut db).is_err());
        assert_eq!(db.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger' AND name='ownership_new_node'",[],|row|row.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(
            db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            VERSION
        );
    }
}
