use crate::error::Error;
use crate::store::execution::id;
use meshway_model::{MembershipId, MembershipRole, NetworkId, UserId, Version};
use rusqlite::{OptionalExtension, Transaction, params};

pub(crate) fn for_user(
    tx: &Transaction<'_>,
    user: UserId,
) -> Result<Vec<(NetworkId, MembershipId)>, Error> {
    let mut stmt = tx.prepare(
        "SELECT m.network_id, m.id FROM memberships m JOIN networks n ON n.id=m.network_id
         WHERE m.user_id=?1 AND m.revoked_at IS NULL AND n.deleted_at IS NULL",
    )?;
    stmt.query_map([user.as_uuid().as_bytes().as_slice()], |r| {
        Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
    })?
    .map(|row| {
        let (network, membership) = row?;
        Ok((id(network)?, id(membership)?))
    })
    .collect()
}

pub(crate) fn email_sync_status(
    tx: &Transaction<'_>,
    network: NetworkId,
    user: UserId,
    membership: MembershipId,
    new_email: &str,
) -> Result<(bool, bool), Error> {
    let active: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM memberships WHERE id=?1 AND network_id=?2 AND user_id=?3 AND revoked_at IS NULL)",
        params![
            membership.as_uuid().as_bytes().as_slice(),
            network.as_uuid().as_bytes().as_slice(),
            user.as_uuid().as_bytes().as_slice()
        ],
        |r| r.get(0),
    )?;
    let valid: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND email=?2 AND disabled_at IS NULL)",
        params![user.as_uuid().as_bytes().as_slice(), new_email],
        |r| r.get(0),
    )?;
    Ok((active, valid))
}

pub(crate) struct RemovalParticipants {
    caller_id: Vec<u8>,
    caller_role: String,
    target_role: String,
    version: i64,
}

impl RemovalParticipants {
    pub(crate) fn version_matches(&self, version: Version) -> bool {
        self.version == version.get() as i64
    }

    pub(crate) fn roles(self) -> Result<((MembershipId, MembershipRole), MembershipRole), Error> {
        Ok((
            (id(self.caller_id)?, role(&self.caller_role)?),
            role(&self.target_role)?,
        ))
    }
}

pub(crate) fn removal_participants(
    tx: &Transaction<'_>,
    network: NetworkId,
    actor: UserId,
    membership: MembershipId,
) -> Result<RemovalParticipants, Error> {
    let network = network.as_uuid().as_bytes().as_slice();
    let (caller_id, caller_role): (Vec<u8>, String) = tx
        .query_row(
            "SELECT id,role FROM memberships WHERE network_id=?1 AND user_id=?2 AND revoked_at IS NULL",
            params![network, actor.as_uuid().as_bytes().as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::PermissionDenied)?;
    let (target_role, version): (String, i64) = tx
        .query_row(
            "SELECT role,version FROM memberships WHERE network_id=?1 AND id=?2 AND revoked_at IS NULL",
            params![network, membership.as_uuid().as_bytes().as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::PermissionDenied)?;
    Ok(RemovalParticipants {
        caller_id,
        caller_role,
        target_role,
        version,
    })
}

pub(crate) fn owner_count(tx: &Transaction<'_>, network: NetworkId) -> Result<i64, Error> {
    Ok(tx.query_row(
        "SELECT count(*) FROM memberships WHERE network_id=?1 AND role='owner' AND revoked_at IS NULL",
        [network.as_uuid().as_bytes().as_slice()],
        |r| r.get(0),
    )?)
}

fn role(value: &str) -> Result<MembershipRole, Error> {
    match value {
        "owner" => Ok(MembershipRole::Owner),
        "admin" => Ok(MembershipRole::Admin),
        "member" => Ok(MembershipRole::Member),
        _ => Err(Error::decode("memberships.role")),
    }
}
