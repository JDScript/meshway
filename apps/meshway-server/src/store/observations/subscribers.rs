//! Durable consumption of subscriber IDs. Live replay is checked by the sole
//! observation owner; after it forgets a task the ID cannot issue a new lease.
use crate::error::Error;
use meshway_model::ObservationSubscriptionId;
use rusqlite::Transaction;

pub(crate) fn claim(tx: &Transaction<'_>, id: ObservationSubscriptionId) -> Result<(), Error> {
    let inserted = tx.execute(
        "INSERT INTO observation_subscriber_ids(id) VALUES(?1) ON CONFLICT(id) DO NOTHING",
        [id.as_uuid().as_bytes().as_slice()],
    )?;
    if inserted != 1 {
        return Err(Error::StateConflict);
    }
    Ok(())
}
