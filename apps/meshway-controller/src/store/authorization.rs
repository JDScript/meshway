//! Current identity checks for use cases sharing an existing transaction.
//!
//! A previous HTTP read or cached user view cannot authorize a later mutation.
//! Network operations call this before checking membership in the same transaction.

use chrono::{DateTime, Utc};
use meshway_model::{SessionId, UserId};
use rusqlite::{OptionalExtension, Transaction, params};
use uuid::Uuid;

use crate::accounts::{AccountError, AuthenticatedSession, User};

pub(crate) fn current_session(
    transaction: &Transaction<'_>,
    digest: &[u8; 32],
    now: i64,
) -> Result<AuthenticatedSession, AccountError> {
    let row = transaction.query_row(
        "SELECT u.id, u.email, s.id, s.created_at, s.expires_at FROM user_sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash = ?1 AND s.revoked_at IS NULL AND s.expires_at > ?2 AND s.created_at <= ?2 AND u.disabled_at IS NULL AND s.credential_version = u.credential_version",
        params![digest.as_slice(), now], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?, row.get::<_, Vec<u8>>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?))
        },
    ).optional()?.ok_or(AccountError::InvalidSession)?;
    let user_id = Uuid::from_slice(&row.0).map_err(|_| AccountError::Storage)?;
    let session_id = Uuid::from_slice(&row.2).map_err(|_| AccountError::Storage)?;
    Ok(AuthenticatedSession {
        user: User {
            id: UserId::new(user_id).map_err(|_| AccountError::Storage)?,
            email: row.1,
        },
        session_id: SessionId::new(session_id).map_err(|_| AccountError::Storage)?,
        created_at: DateTime::<Utc>::from_timestamp_millis(row.3).ok_or(AccountError::Storage)?,
        expires_at: DateTime::<Utc>::from_timestamp_millis(row.4).ok_or(AccountError::Storage)?,
    })
}
