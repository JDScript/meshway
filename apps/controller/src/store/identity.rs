//! Immutable public-key bindings in the active network inventory.
use super::NetworkId;
use crate::Error;
use rusqlite::Connection;
use std::collections::BTreeMap;

pub(super) fn bindings(
    db: &Connection,
    scope: &NetworkId,
) -> Result<BTreeMap<String, String>, Error> {
    Ok(db.prepare("SELECT node_id, public_key FROM connection_identities JOIN nodes ON nodes.id=node_id AND nodes.network_id=connection_identities.network_id WHERE nodes.revoked_at IS NULL AND nodes.network_id=?1 ORDER BY node_id")?
        .query_map([scope.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?)
}
