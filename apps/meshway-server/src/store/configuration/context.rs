use crate::error::Error;
use meshway_configuration::UserBinding;
use meshway_model::{NetworkId, NodeId};
use rusqlite::Transaction;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn inventory_ids(
    tx: &Transaction<'_>,
    network: NetworkId,
) -> Result<BTreeSet<NodeId>, Error> {
    let mut statement =
        tx.prepare("SELECT id FROM nodes WHERE network_id=?1 AND credentials_revoked_at IS NULL")?;
    statement
        .query_map([network.as_uuid().as_bytes().as_slice()], |r| {
            r.get::<_, Vec<u8>>(0)
        })?
        .map(|row| crate::store::execution::id(row?))
        .collect()
}

pub(crate) fn members(
    tx: &Transaction<'_>,
    network: NetworkId,
) -> Result<BTreeMap<String, UserBinding>, Error> {
    let mut statement = tx.prepare(
        "SELECT u.email,u.id,m.id FROM memberships m JOIN users u ON u.id=m.user_id WHERE m.network_id=?1 AND m.revoked_at IS NULL AND u.disabled_at IS NULL",
    )?;
    statement
        .query_map([network.as_uuid().as_bytes().as_slice()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })?
        .map(|row| {
            let (email, user, member) = row?;
            Ok((
                email,
                UserBinding {
                    user_id: crate::store::execution::id(user)?,
                    membership_id: crate::store::execution::id(member)?,
                },
            ))
        })
        .collect()
}
