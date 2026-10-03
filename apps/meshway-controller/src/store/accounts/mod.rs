//! Private account persistence. All final decisions are made on the SQLite
//! worker, in the same transaction as any mutation and its activity record.

mod activity;
pub(crate) mod management;

use std::time::Duration;

use chrono::{DateTime, Utc};
use meshway_model::{RequestId, SessionId, UserId};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::authorization::current_session;
use crate::accounts::{
    AccountError, AuthenticatedSession, IssuedSession, User, issue_token, random_uuid,
};
use activity::{AccountActivity, record};

// No Debug: contains a password verifier. Read only by the KDF and compared in
// the issuing transaction; never treated as an authorization snapshot.
pub(crate) struct LoginCandidate {
    user_id: UserId,
    pub(crate) password_hash: Zeroizing<String>,
    credential_version: i64,
}

pub(crate) fn login_candidate(
    connection: &Connection,
    email: Option<&str>,
) -> Result<Option<LoginCandidate>, AccountError> {
    let Some(email) = email else {
        return Ok(None);
    };
    connection.query_row(
        // Bound even a damaged database's PHC allocation before the KDF parser.
        "SELECT id, password_hash, credential_version FROM users WHERE email = ?1 AND length(CAST(password_hash AS BLOB)) <= 256",
        [email], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, Zeroizing::new(row.get::<_, String>(1)?), row.get::<_, i64>(2)?))
        },
    ).optional()?.map(|(id, password_hash, credential_version)| {
        Ok(LoginCandidate { user_id: user_id(&id)?, password_hash, credential_version })
    }).transpose()
}

pub(crate) fn register(
    connection: &mut Connection,
    email: String,
    password_hash: Zeroizing<String>,
    ttl: Duration,
    request: RequestId,
    client_label: Option<&str>,
) -> Result<IssuedSession, AccountError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE email = ?1)",
        [&email],
        |row| row.get::<_, bool>(0),
    )? {
        return Err(AccountError::EmailTaken);
    }
    let now = now()?;
    let user = User {
        id: UserId::new(random_uuid()?).map_err(|_| AccountError::Unavailable)?,
        email,
    };
    transaction.execute("INSERT INTO users (id, email, password_hash, credential_version, created_at) VALUES (?1, ?2, ?3, 1, ?4)",
        params![user.id.as_uuid().as_bytes().as_slice(), user.email, password_hash.as_str(), now])?;
    let issued = insert_session(&transaction, user, 1, now, ttl, client_label)?;
    record(
        &transaction,
        &issued.session,
        now,
        request,
        AccountActivity::Register,
    )?;
    transaction.commit()?;
    Ok(issued)
}

pub(crate) fn login(
    connection: &mut Connection,
    verified: LoginCandidate,
    ttl: Duration,
    request: RequestId,
    client_label: Option<&str>,
) -> Result<IssuedSession, AccountError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Revalidate every security input read before the KDF, including the PHC
    // itself. A credential-version increment is not the sole race protection.
    let email = transaction.query_row(
        "SELECT email FROM users WHERE id = ?1 AND password_hash = ?2 AND credential_version = ?3 AND disabled_at IS NULL",
        params![verified.user_id.as_uuid().as_bytes().as_slice(), verified.password_hash.as_str(), verified.credential_version],
        |row| row.get::<_, String>(0),
    ).optional()?.ok_or(AccountError::InvalidCredentials)?;
    let now = now()?;
    let issued = insert_session(
        &transaction,
        User {
            id: verified.user_id,
            email,
        },
        verified.credential_version,
        now,
        ttl,
        client_label,
    )?;
    record(
        &transaction,
        &issued.session,
        now,
        request,
        AccountActivity::Login,
    )?;
    transaction.commit()?;
    Ok(issued)
}

fn insert_session(
    transaction: &Transaction<'_>,
    user: User,
    version: i64,
    now: i64,
    ttl: Duration,
    client_label: Option<&str>,
) -> Result<IssuedSession, AccountError> {
    let session_id = SessionId::new(random_uuid()?).map_err(|_| AccountError::Unavailable)?;
    let (token, digest) = issue_token()?;
    let expires = now
        .checked_add(i64::try_from(ttl.as_millis()).map_err(|_| AccountError::Unavailable)?)
        .ok_or(AccountError::Unavailable)?;
    transaction.execute(
        "INSERT INTO user_sessions (id, token_hash, user_id, credential_version, created_at, expires_at, client_label) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![session_id.as_uuid().as_bytes().as_slice(), digest.as_slice(), user.id.as_uuid().as_bytes().as_slice(), version, now, expires, client_label],
    )?;
    Ok(IssuedSession {
        session: AuthenticatedSession {
            user,
            session_id,
            created_at: timestamp(now)?,
            expires_at: timestamp(expires)?,
        },
        token,
    })
}

pub(crate) fn authenticate(
    connection: &mut Connection,
    digest: [u8; 32],
) -> Result<AuthenticatedSession, AccountError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let session = current_session(&transaction, &digest, now()?)?;
    transaction.commit()?;
    Ok(session)
}

pub(crate) fn logout(
    connection: &mut Connection,
    digest: [u8; 32],
    request: RequestId,
) -> Result<(), AccountError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let session = current_session(&transaction, &digest, now)?;
    transaction.execute(
        "UPDATE user_sessions SET revoked_at = ?1 WHERE id = ?2",
        params![now, session.session_id.as_uuid().as_bytes().as_slice()],
    )?;
    record(
        &transaction,
        &session,
        now,
        request,
        AccountActivity::Logout,
    )?;
    transaction.commit()?;
    Ok(())
}

fn now() -> Result<i64, AccountError> {
    let now = Utc::now().timestamp_millis();
    if now < 0 {
        return Err(AccountError::Unavailable);
    }
    Ok(now)
}

fn timestamp(value: i64) -> Result<DateTime<Utc>, AccountError> {
    DateTime::from_timestamp_millis(value).ok_or(AccountError::Storage)
}

fn uuid(bytes: &[u8]) -> Result<Uuid, AccountError> {
    Uuid::from_slice(bytes).map_err(|_| AccountError::Storage)
}
fn user_id(bytes: &[u8]) -> Result<UserId, AccountError> {
    UserId::new(uuid(bytes)?).map_err(|_| AccountError::Storage)
}

#[cfg(test)]
mod tests;
