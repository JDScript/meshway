//! Network authorization, persistence, and bounded reads on the SQLite owner.

pub(crate) mod management;
mod rows;

use super::authorization::current_session;
use crate::{
    accounts::random_uuid,
    networks::{
        MemberPage, NetworkError, NetworkPage, NetworkView, configuration::PreparedNetwork,
        pagination::Window,
    },
    secrets::{CursorScope, Sealer},
};
use chrono::Utc;
use meshway_model::{NetworkId, RequestId, UserId};
use rows::{RawMember, RawNetwork};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

const NETWORK_SELECT: &str = "SELECT n.sequence, n.id, n.name, n.created_at, n.current_revision, m.id, m.role FROM networks n JOIN memberships m ON m.network_id = n.id WHERE m.user_id = ?1 AND m.revoked_at IS NULL AND n.deleted_at IS NULL";

pub(crate) fn authenticate(
    connection: &mut Connection,
    digest: [u8; 32],
) -> Result<(), NetworkError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
    current_session(&transaction, &digest, now()?)?;
    transaction.commit()?;
    Ok(())
}

pub(crate) fn create(
    connection: &mut Connection,
    digest: [u8; 32],
    prepared: PreparedNetwork,
    request: RequestId,
    sealer: &Sealer,
    cache: &crate::configuration::SnapshotCache,
) -> Result<NetworkView, NetworkError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let session = current_session(&transaction, &digest, now)?;
    let user = session.user.id;
    let previous = transaction.query_row("SELECT id, creation_request_digest FROM networks WHERE created_by = ?1 AND creation_request_id = ?2", params![user.as_uuid().as_bytes().as_slice(), request.as_uuid().as_bytes().as_slice()], |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))).optional()?;
    if let Some((id, original_digest)) = previous {
        let id = rows::network_id(&id)?;
        // A creator is not an enduring authority. Check current membership
        // before disclosing either an old result or its parameter conflict.
        let view = view(&transaction, id, user, sealer, cache)?;
        if original_digest != prepared.request_digest {
            return Err(NetworkError::RequestConflict);
        }
        transaction.commit()?;
        return Ok(view);
    }
    let id = prepared.id.as_uuid().as_bytes().as_slice();
    let user_bytes = user.as_uuid().as_bytes().as_slice();
    let request_bytes = request.as_uuid().as_bytes().as_slice();
    transaction.execute("INSERT INTO networks (id, name, created_by, created_at, creation_request_id, creation_request_digest, current_revision, publication_sequence) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 0)", params![id, prepared.name, user_bytes, now, request_bytes, prepared.request_digest.as_slice()])?;
    transaction.execute("INSERT INTO memberships (id, network_id, user_id, role, version, joined_at) VALUES (?1, ?2, ?3, 'owner', 1, ?4)", params![random_uuid()?.as_bytes().as_slice(), id, user_bytes, now])?;
    transaction.execute("INSERT INTO configuration_versions (network_id, revision, source_jsonc, configuration, bindings, created_by, created_at, cause, request_id) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, 'networkCreated', ?7)", params![id, prepared.source.as_bytes(), prepared.configuration.as_bytes(), prepared.bindings.as_bytes(), user_bytes, now, request_bytes])?;
    record(
        &transaction,
        prepared.id,
        user,
        request,
        "network.create",
        now,
    )?;
    let view = view(&transaction, prepared.id, user, sealer, cache)?;
    transaction.commit()?;
    Ok(view)
}

pub(crate) fn get(
    connection: &mut Connection,
    digest: [u8; 32],
    id: NetworkId,
    sealer: &Sealer,
    cache: &crate::configuration::SnapshotCache,
) -> Result<NetworkView, NetworkError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let user = current_session(&transaction, &digest, now()?)?.user.id;
    let view = view(&transaction, id, user, sealer, cache)?;
    transaction.commit()?;
    Ok(view)
}

pub(crate) fn rename(
    connection: &mut Connection,
    digest: [u8; 32],
    id: NetworkId,
    change: crate::networks::NameChange,
    sealer: &Sealer,
    cache: &crate::configuration::SnapshotCache,
) -> Result<NetworkView, NetworkError> {
    let crate::networks::NameChange {
        name,
        expected,
        request,
    } = change;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let user = current_session(&transaction, &digest, now)?.user.id;
    let mut view = view(&transaction, id, user, sealer, cache)?;
    if !view.actions.rename {
        return Err(NetworkError::PermissionDenied);
    }
    if view.name != expected {
        return Err(NetworkError::NameConflict);
    }
    if view.name != name {
        transaction.execute(
            "UPDATE networks SET name = ?1 WHERE id = ?2",
            params![name, id.as_uuid().as_bytes().as_slice()],
        )?;
        record(&transaction, id, user, request, "network.rename", now)?;
        view.name = name;
    }
    transaction.commit()?;
    Ok(view)
}

pub(crate) fn list(
    connection: &mut Connection,
    digest: [u8; 32],
    limit: u16,
    cursor: Option<String>,
    sealer: &Sealer,
    cache: &crate::configuration::SnapshotCache,
) -> Result<NetworkPage, NetworkError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let now = now()?;
    let user = current_session(&transaction, &digest, now)?.user.id;
    let scope = CursorScope::Networks(user);
    let mut window = match cursor {
        Some(cursor) => Window::decode(sealer, scope, &cursor, now)?,
        None => Window::new(
            ceiling(&transaction, "networks")?,
            ceiling(&transaction, "memberships")?,
            now,
        )?,
    };
    let sql = format!(
        "{NETWORK_SELECT} AND n.sequence > ?2 AND n.sequence <= ?3 AND m.sequence <= ?4 ORDER BY n.sequence ASC LIMIT ?5"
    );
    let mut statement = transaction.prepare(&sql)?;
    let mut rows = statement
        .query_map(
            params![
                user.as_uuid().as_bytes().as_slice(),
                window.last,
                window.upper,
                window.memberships_upper,
                i64::from(limit) + 1
            ],
            RawNetwork::read,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let more = rows.len() > usize::from(limit);
    rows.truncate(usize::from(limit));
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        window.last = row.sequence;
        let view = row.into_view()?;
        check_configuration(&transaction, &view, sealer, cache)?;
        items.push(view);
    }
    let next_cursor = if more {
        Some(window.encode(sealer, scope)?)
    } else {
        None
    };
    let as_of = window.as_of()?;
    let total = transaction.query_row(
        &format!("SELECT count(*) FROM ({NETWORK_SELECT} AND n.sequence<=?2 AND m.sequence<=?3)"),
        params![
            user.as_uuid().as_bytes().as_slice(),
            window.upper,
            window.memberships_upper
        ],
        |r| r.get::<_, i64>(0),
    )?;
    let total = meshway_model::Count::new(u64::try_from(total).map_err(|_| NetworkError::Storage)?)
        .map_err(|_| NetworkError::Storage)?;
    transaction.commit()?;
    Ok(NetworkPage {
        items,
        next_cursor,
        as_of,
        total,
    })
}

pub(crate) fn members(
    connection: &mut Connection,
    digest: [u8; 32],
    id: NetworkId,
    limit: u16,
    cursor: Option<String>,
    sealer: &Sealer,
    cache: &crate::configuration::SnapshotCache,
) -> Result<MemberPage, NetworkError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let now = now()?;
    let user = current_session(&transaction, &digest, now)?.user.id;
    let caller = view(&transaction, id, user, sealer, cache)?;
    if !caller.actions.manage_members {
        return Err(NetworkError::PermissionDenied);
    }
    let scope = CursorScope::Members { user, network: id };
    let mut window = match cursor {
        Some(cursor) => Window::decode(sealer, scope, &cursor, now)?,
        None => {
            let upper = ceiling(&transaction, "memberships")?;
            Window::new(upper, upper, now)?
        }
    };
    let mut statement = transaction.prepare("SELECT m.sequence, m.id, m.user_id, u.email, m.role, m.version, m.joined_at FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.network_id = ?1 AND m.revoked_at IS NULL AND m.sequence > ?2 AND m.sequence <= ?3 ORDER BY m.sequence ASC LIMIT ?4")?;
    let mut rows = statement
        .query_map(
            params![
                id.as_uuid().as_bytes().as_slice(),
                window.last,
                window.upper,
                i64::from(limit) + 1
            ],
            RawMember::read,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let more = rows.len() > usize::from(limit);
    rows.truncate(usize::from(limit));
    let mut items = Vec::with_capacity(rows.len());
    let owner_count = management::owner_count(&transaction, id)?;
    for row in rows {
        window.last = row.sequence;
        items.push(row.into_view((caller.membership_id, caller.role), owner_count)?);
    }
    let next_cursor = if more {
        Some(window.encode(sealer, scope)?)
    } else {
        None
    };
    let as_of = window.as_of()?;
    let total=transaction.query_row("SELECT count(*) FROM memberships m JOIN users u ON u.id=m.user_id WHERE m.network_id=?1 AND m.revoked_at IS NULL AND m.sequence<=?2",params![id.as_uuid().as_bytes().as_slice(),window.upper],|r|r.get::<_,i64>(0))?;
    let total = meshway_model::Count::new(u64::try_from(total).map_err(|_| NetworkError::Storage)?)
        .map_err(|_| NetworkError::Storage)?;
    transaction.commit()?;
    Ok(MemberPage {
        items,
        next_cursor,
        as_of,
        total,
    })
}

fn view(
    transaction: &Transaction<'_>,
    id: NetworkId,
    user: UserId,
    sealer: &Sealer,
    cache: &crate::configuration::SnapshotCache,
) -> Result<NetworkView, NetworkError> {
    let sql = format!("{NETWORK_SELECT} AND n.id = ?2");
    let raw = transaction
        .query_row(
            &sql,
            params![
                user.as_uuid().as_bytes().as_slice(),
                id.as_uuid().as_bytes().as_slice()
            ],
            RawNetwork::read,
        )
        .optional()?
        .ok_or(NetworkError::NotFound)?;
    let view = raw.into_view()?;
    check_configuration(transaction, &view, sealer, cache)?;
    Ok(view)
}

fn check_configuration(
    transaction: &Transaction<'_>,
    view: &NetworkView,
    sealer: &Sealer,
    cache: &crate::configuration::SnapshotCache,
) -> Result<(), NetworkError> {
    cache
        .load(transaction, view.id, view.current_revision, sealer)
        .map(|_| ())
        .map_err(|_| NetworkError::Storage)
}

fn ceiling(transaction: &Transaction<'_>, table: &str) -> Result<i64, NetworkError> {
    // sqlite_sequence retains the allocation high-water mark after deletion.
    Ok(transaction
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = ?1",
            [table],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

fn record(
    transaction: &Transaction<'_>,
    network: NetworkId,
    user: UserId,
    request: RequestId,
    action: &str,
    now: i64,
) -> Result<(), NetworkError> {
    transaction.execute("INSERT INTO activity_events (id, scope, network_id, at, actor_user_id, action, request_id) VALUES (?1, 'network', ?2, ?3, ?4, ?5, ?6)", params![random_uuid()?.as_bytes().as_slice(), network.as_uuid().as_bytes().as_slice(), now, user.as_uuid().as_bytes().as_slice(), action, request.as_uuid().as_bytes().as_slice()])?;
    Ok(())
}

fn now() -> Result<i64, NetworkError> {
    let now = Utc::now().timestamp_millis();
    if now < 0 {
        Err(NetworkError::Unavailable)
    } else {
        Ok(now)
    }
}
