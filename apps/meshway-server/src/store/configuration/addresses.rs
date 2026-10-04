use crate::error::Error;
use meshway_model::NetworkId;
use rusqlite::Transaction;
use std::{collections::BTreeSet, net::IpAddr};

pub(crate) fn retired_addresses(
    tx: &Transaction<'_>,
    network: NetworkId,
) -> Result<BTreeSet<IpAddr>, Error> {
    let mut statement = tx.prepare(
        "SELECT address FROM address_retirements WHERE network_id=?1 AND (release_after IS NULL OR release_after>?2)",
    )?;
    statement
        .query_map(
            rusqlite::params![
                network.as_uuid().as_bytes().as_slice(),
                crate::store::execution::now()?
            ],
            |r| r.get::<_, String>(0),
        )?
        .map(|r| {
            r?.parse::<IpAddr>()
                .map_err(|_| Error::decode("address_retirements.address"))
        })
        .collect()
}
