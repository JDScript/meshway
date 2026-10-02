//! One atomic v10 -> v11 installation of accounts, ownership and probe history.
//! Recognized unversioned development fragments may be adopted without rewriting
//! their rows; partial or unknown structures are rejected before versioning.
use super::{account_schema, latency_history};
use crate::Error;
use rusqlite::{Connection, TransactionBehavior, params};

pub(super) const VERSION: i64 = 12;

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
            super::policy_v3_schema::install(&tx)?;
            tx.pragma_update(None, "user_version", VERSION)?;
        }
        11 => {
            validate(&tx)?;
            super::policy_v3_schema::install(&tx)?;
            tx.pragma_update(None, "user_version", VERSION)?;
        }
        VERSION => {
            validate(&tx)?;
            super::policy_v3_schema::validate(&tx)?;
        }
        _ => {
            return Err(Error::Configuration(
                "release migration requires schema 10, 11 or 12",
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

    use crate::store::test_support::fixtures::release::{NETWORK, NODE, PEER, v10};

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
        let mut db = crate::store::test_support::fixtures::release::v11();
        db.execute_batch("DROP TRIGGER ownership_new_node").unwrap();
        assert!(upgrade(&mut db).is_err());
        assert_eq!(db.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger' AND name='ownership_new_node'",[],|row|row.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(
            db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            11
        );
    }
}
