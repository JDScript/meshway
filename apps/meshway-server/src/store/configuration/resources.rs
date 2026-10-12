use crate::error::Error;
use meshway_model::{NetworkId, Revision};
use rusqlite::{Transaction, params};

pub(crate) fn record_check(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
    key: &str,
    now: i64,
) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO rule_set_checks(network_id,revision,rule_set_key,checked_at,last_success_at,problem,failures)
         VALUES(?1,?2,?3,?4,?4,NULL,0) ON CONFLICT(network_id,revision,rule_set_key)
         DO UPDATE SET checked_at=excluded.checked_at,last_success_at=excluded.last_success_at,problem=NULL,failures=0,
             check_version=rule_set_checks.check_version+1",
        params![network.as_uuid().as_bytes().as_slice(),revision.get() as i64,key,now],
    )?;
    Ok(())
}
