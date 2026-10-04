//! SQL behind report admission: live instance, lease and admissible configurations.
use crate::error::Error;
use meshway_model::{InstanceId, MembershipId, NetworkId, NodeId, UserId};
use rusqlite::{OptionalExtension, Transaction, params};

pub(crate) fn membership_role(
    tx: &Transaction<'_>,
    network: NetworkId,
    user: UserId,
) -> Result<(MembershipId, String), Error> {
    let (membership, role): (Vec<u8>, String) = tx.query_row(
        "SELECT m.id,m.role FROM memberships m JOIN networks n ON n.id=m.network_id WHERE m.network_id=?1 AND m.user_id=?2 AND m.revoked_at IS NULL AND n.deleted_at IS NULL",
        params![network.as_uuid().as_bytes().as_slice(), user.as_uuid().as_bytes().as_slice()],
        |r| Ok((r.get(0)?, r.get(1)?))).optional()?.ok_or(Error::NotFound)?;
    Ok((super::id(membership)?, role))
}

pub(crate) fn credentials_revoked(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
) -> Result<bool, Error> {
    Ok(tx.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM nodes n JOIN node_keys c ON c.network_id=n.network_id AND c.node_id=n.id AND c.purpose='control' AND c.retired_at IS NULL AND c.revoked_at IS NULL JOIN node_keys d ON d.network_id=n.network_id AND d.node_id=n.id AND d.purpose='dataplane' AND d.retired_at IS NULL AND d.revoked_at IS NULL WHERE n.network_id=?1 AND n.id=?2 AND n.credentials_revoked_at IS NULL)",
        params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice()], |r| r.get(0))?)
}

pub(crate) fn has_live_instance(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
    boot: InstanceId,
    now: i64,
) -> Result<bool, Error> {
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM node_execution WHERE network_id=?1 AND node_id=?2 AND boot_id=?3 AND instance_expires_at>?4)",
        params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice(), boot.as_uuid().as_bytes().as_slice(), now], |r| r.get(0))?)
}
