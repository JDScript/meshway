//! Authorization codes and delegated sessions. Every decision is made in the
//! issuing or exchanging transaction against the current session, user and
//! code row; a code is consumed exactly once.
use super::{insert_session, now, timestamp};
use crate::accounts::delegation::{self, DelegationClients};
use crate::accounts::{IssuedCode, IssuedSession, User};
use crate::error::Error;
use crate::store::authorization::current_session;
use meshway_model::RequestId;
use rusqlite::{OptionalExtension, Transaction, params};

/// What a signed-in user asks to authorize, as the Console sends it.
pub struct Authorization {
    pub client: String,
    pub redirect_uri: String,
    pub code_challenge: String,
}

pub(crate) fn authorize(
    tx: &Transaction<'_>,
    digest: [u8; 32],
    clients: &DelegationClients,
    authorization: &Authorization,
    request: RequestId,
) -> Result<IssuedCode, Error> {
    let Authorization {
        client,
        redirect_uri,
        code_challenge,
    } = authorization;
    let now = now()?;
    let session = current_session(tx, &digest, now)?;
    if session.delegated() {
        return Err(Error::PermissionDenied);
    }
    let target = clients.redirect_target(client, redirect_uri)?;
    if !delegation::valid_code_challenge(code_challenge) {
        return Err(Error::InvalidRequest);
    }
    tx.execute("DELETE FROM delegation_codes WHERE expires_at<=?1", [now])?;
    let (code, code_hash) = delegation::issue_code()?;
    let expires = now
        .checked_add(
            i64::try_from(delegation::CODE_TTL.as_millis())
                .map_err(|_| Error::Internal("delegation code TTL overflow"))?,
        )
        .ok_or(Error::Internal("delegation code expiry overflow"))?;
    tx.execute(
        "INSERT INTO delegation_codes (code_hash, user_id, session_id, client, redirect_uri, code_challenge, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            code_hash.as_slice(),
            session.user.id.as_uuid().as_bytes().as_slice(),
            session.session_id.as_uuid().as_bytes().as_slice(),
            client,
            target.as_str(),
            code_challenge,
            now,
            expires
        ],
    )?;
    super::management::record_management(
        tx,
        session.user.id,
        request,
        "session.delegate",
        Some(session.session_id),
        None,
        now,
    )?;
    Ok(IssuedCode {
        code,
        expires_at: timestamp(expires)?,
    })
}

/// The committed result of an exchange attempt: a mismatch still consumes the
/// code, so it is a value the transaction commits, not an error that rolls back.
pub(crate) enum Exchange {
    Issued(IssuedSession, String),
    Rejected,
}

/// Consumes the code and issues the delegated session in one transaction. Any
/// mismatch is the same rejection so a caller learns nothing about other codes.
pub(crate) fn exchange(
    tx: &Transaction<'_>,
    clients: &DelegationClients,
    client: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    request: RequestId,
) -> Result<Exchange, Error> {
    let now = now()?;
    let Ok(target) = clients.redirect_target(client, redirect_uri) else {
        return Ok(Exchange::Rejected);
    };
    if !delegation::valid_code_verifier(code_verifier) {
        return Ok(Exchange::Rejected);
    }
    let Ok(code_hash) = delegation::code_digest(code) else {
        return Ok(Exchange::Rejected);
    };
    let consumed = tx.execute(
        "UPDATE delegation_codes SET consumed_at=?1 WHERE code_hash=?2 AND consumed_at IS NULL AND expires_at>?1 AND client=?3 AND redirect_uri=?4 AND code_challenge=?5",
        params![
            now,
            code_hash.as_slice(),
            client,
            target.as_str(),
            delegation::challenge_of(code_verifier)
        ],
    )?;
    if consumed != 1 {
        // A valid but mismatched code is spent as well: it cannot be retried
        // with a different verifier or target.
        tx.execute(
            "UPDATE delegation_codes SET consumed_at=?1 WHERE code_hash=?2 AND consumed_at IS NULL",
            params![now, code_hash.as_slice()],
        )?;
        return Ok(Exchange::Rejected);
    }
    // The authorizing session must still be valid: a user who signed out or
    // was signed out everywhere has withdrawn the authorization as well.
    let row = tx
        .query_row(
            "SELECT u.id, u.email, u.credential_version FROM delegation_codes c JOIN users u ON u.id=c.user_id JOIN user_sessions s ON s.id=c.session_id AND s.user_id=c.user_id WHERE c.code_hash=?1 AND u.disabled_at IS NULL AND s.revoked_at IS NULL AND s.expires_at>?2 AND s.credential_version=u.credential_version",
            params![code_hash.as_slice(), now],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(Exchange::Rejected);
    };
    let user = User {
        id: meshway_model::UserId::new(super::uuid(&row.0)?)
            .map_err(|_| Error::decode("users.id"))?,
        email: row.1,
    };
    let issued = insert_session(
        tx,
        user,
        row.2,
        now,
        delegation::SESSION_TTL,
        Some(client),
        Some(client),
    )?;
    super::activity::record(
        tx,
        &issued.session,
        now,
        request,
        super::activity::AccountActivity::Login,
    )?;
    Ok(Exchange::Issued(issued, client.to_owned()))
}
