use crate::Error;
use rusqlite::{Connection, TransactionBehavior};

pub(in crate::store) fn upgrade(connection: &mut Connection) -> Result<(), Error> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version == 10 {
        return Ok(());
    }
    if version != 9 {
        return Err(Error::Configuration("managed transports require schema 9"));
    }
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(include_str!("sql/schema_v10.sql"))?;
    if tx.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(Error::Configuration(
            "managed transport migration foreign key failure",
        ));
    }
    tx.commit()?;
    Ok(())
}
