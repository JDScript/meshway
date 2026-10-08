//! Current identity checks for use cases sharing an existing transaction.
//!
//! A previous HTTP read or cached user view cannot authorize a later mutation.
//! Network operations call this before checking membership in the same transaction.

use chrono::{DateTime, Utc};
use meshway_model::{MembershipId, MembershipRole, NetworkId, SessionId, UserId};
use rusqlite::{OptionalExtension, Transaction, params};
use uuid::Uuid;

use crate::accounts::{AuthenticatedSession, User};

use crate::error::Error;

const SESSION_USE_RESOLUTION_MS: i64 = 30_000;

pub(crate) fn network_actions(role: MembershipRole) -> meshway_model::NetworkActions {
    let manage = matches!(role, MembershipRole::Owner | MembershipRole::Admin);
    meshway_model::NetworkActions {
        rename: manage,
        read_configuration: manage,
        edit_configuration: manage,
        manage_members: manage,
        join_link_roles: [
            MembershipRole::Owner,
            MembershipRole::Admin,
            MembershipRole::Member,
        ]
        .into_iter()
        .filter(|target| can_grant(role, *target))
        .collect(),
    }
}
pub(crate) fn can_grant(issuer: MembershipRole, role: MembershipRole) -> bool {
    issuer == MembershipRole::Owner
        || (issuer == MembershipRole::Admin && role == MembershipRole::Member)
}
pub(crate) fn member_actions(
    caller: (MembershipId, MembershipRole),
    target: (MembershipId, MembershipRole),
    owner_count: i64,
) -> meshway_model::MemberActions {
    let last_owner = target.1 == MembershipRole::Owner && owner_count <= 1;
    let assignable_roles: Vec<_> = [
        MembershipRole::Owner,
        MembershipRole::Admin,
        MembershipRole::Member,
    ]
    .into_iter()
    .filter(|role| {
        can_grant(caller.1, target.1)
            && can_grant(caller.1, *role)
            && (!last_owner || *role == MembershipRole::Owner)
    })
    .collect();
    meshway_model::MemberActions {
        change_role: assignable_roles.iter().any(|role| *role != target.1),
        assignable_roles,
        remove: !last_owner && (caller.0 == target.0 || can_grant(caller.1, target.1)),
    }
}

pub(crate) fn membership(
    tx: &Transaction<'_>,
    network: NetworkId,
    user: UserId,
) -> Result<(MembershipId, MembershipRole), Error> {
    let (id, role) = tx.query_row(
        "SELECT m.id,m.role FROM memberships m JOIN networks n ON n.id=m.network_id JOIN users u ON u.id=m.user_id WHERE m.network_id=?1 AND m.user_id=?2 AND m.revoked_at IS NULL AND n.deleted_at IS NULL AND u.disabled_at IS NULL",
        params![network.as_uuid().as_bytes().as_slice(), user.as_uuid().as_bytes().as_slice()],
        |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
    ).optional()?.ok_or(Error::NotFound)?;
    Ok((
        MembershipId::new(Uuid::from_slice(&id).map_err(|_| Error::decode("memberships.id"))?)
            .map_err(|_| Error::decode("memberships.id"))?,
        parse_role(&role)?,
    ))
}

pub(crate) fn parse_role(role: &str) -> Result<MembershipRole, Error> {
    match role {
        "owner" => Ok(MembershipRole::Owner),
        "admin" => Ok(MembershipRole::Admin),
        "member" => Ok(MembershipRole::Member),
        _ => Err(Error::decode("memberships.role")),
    }
}

pub(crate) fn authorize_manager(
    tx: &Transaction<'_>,
    network: NetworkId,
    user: UserId,
) -> Result<(), Error> {
    require_manager(membership(tx, network, user)?.1)
}

pub(crate) fn require_manager(role: MembershipRole) -> Result<(), Error> {
    match role {
        MembershipRole::Owner | MembershipRole::Admin => Ok(()),
        MembershipRole::Member => Err(Error::PermissionDenied),
    }
}

pub(crate) fn owner_count(tx: &Transaction<'_>, network: NetworkId) -> Result<i64, Error> {
    Ok(tx.query_row(
        "SELECT count(*) FROM memberships WHERE network_id=?1 AND revoked_at IS NULL AND role='owner'",
        [network.as_uuid().as_bytes().as_slice()], |row| row.get::<_, i64>(0),
    )?)
}

pub(crate) fn protect_owner(tx: &Transaction<'_>, network: NetworkId) -> Result<(), Error> {
    if owner_count(tx, network)? <= 1 {
        Err(Error::LastOwner)
    } else {
        Ok(())
    }
}

pub(crate) fn current_session(
    transaction: &Transaction<'_>,
    digest: &[u8; 32],
    now: i64,
) -> Result<AuthenticatedSession, Error> {
    let row = transaction.query_row(
        "SELECT u.id, u.email, s.id, s.created_at, s.expires_at, s.delegation_client FROM user_sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash = ?1 AND s.revoked_at IS NULL AND s.expires_at > ?2 AND s.created_at <= ?2 AND u.disabled_at IS NULL AND s.credential_version = u.credential_version",
        params![digest.as_slice(), now], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?, row.get::<_, Vec<u8>>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?, row.get::<_, Option<String>>(5)?))
        },
    ).optional()?.ok_or(Error::InvalidSession)?;
    let user_id = Uuid::from_slice(&row.0).map_err(|_| Error::decode("users.id"))?;
    let session_id = Uuid::from_slice(&row.2).map_err(|_| Error::decode("user_sessions.id"))?;
    let session = AuthenticatedSession {
        user: User {
            id: UserId::new(user_id).map_err(|_| Error::decode("users.id"))?,
            email: row.1,
        },
        session_id: SessionId::new(session_id).map_err(|_| Error::decode("user_sessions.id"))?,
        created_at: DateTime::<Utc>::from_timestamp_millis(row.3)
            .ok_or(Error::decode("user_sessions.created_at"))?,
        expires_at: DateTime::<Utc>::from_timestamp_millis(row.4)
            .ok_or(Error::decode("user_sessions.expires_at"))?,
        delegation_client: row.5,
    };
    // This is observed use, not a sliding auth lease. Coalesce metadata writes
    // while rechecking the actual session and credential version on every call.
    transaction.execute(
        "UPDATE user_sessions SET last_used_at = ?1
         WHERE id = ?2 AND (last_used_at IS NULL OR last_used_at < ?3)",
        params![
            now,
            session.session_id.as_uuid().as_bytes().as_slice(),
            now.saturating_sub(SESSION_USE_RESOLUTION_MS)
        ],
    )?;
    Ok(session)
}
