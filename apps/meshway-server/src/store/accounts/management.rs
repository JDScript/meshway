use super::{LoginCandidate, login_candidate, now, timestamp, user_id, uuid};
use crate::accounts::{AccountSession, IssuedReset, ResetInspection, RevokeSessions};
use crate::accounts::{User, random_uuid};
use crate::effect::Effect;
use crate::error::Error;
use crate::store::authorization::current_session;
use meshway_model::{RequestId, SessionId, UserId};
use rusqlite::{OptionalExtension, Transaction, params};
use zeroize::Zeroizing;

pub(super) fn authorization_changes(
    tx: &Transaction<'_>,
    user: UserId,
) -> Result<Vec<Effect>, Error> {
    let mut statement =
        tx.prepare("SELECT network_id FROM memberships WHERE user_id=?1 AND revoked_at IS NULL")?;
    statement
        .query_map([user.as_uuid().as_bytes().as_slice()], |r| {
            r.get::<_, Vec<u8>>(0)
        })?
        .map(|row| {
            Ok(Effect::Authorization {
                network_id: meshway_model::NetworkId::new(uuid(&row?)?)
                    .map_err(|_| Error::decode("memberships.network_id"))?,
            })
        })
        .collect()
}

fn active_sessions(tx: &Transaction<'_>, user: UserId, now: i64) -> Result<Vec<SessionId>, Error> {
    let mut statement = tx.prepare("SELECT id FROM user_sessions WHERE user_id=?1 AND revoked_at IS NULL AND expires_at>?2 AND credential_version=(SELECT credential_version FROM users WHERE id=?1)")?;
    statement
        .query_map(params![user.as_uuid().as_bytes().as_slice(), now], |r| {
            r.get::<_, Vec<u8>>(0)
        })?
        .map(|row| SessionId::new(uuid(&row?)?).map_err(|_| Error::decode("user_sessions.id")))
        .collect()
}

fn revocation_effects(mut changes: Vec<Effect>, sessions: Vec<SessionId>) -> Vec<Effect> {
    changes.extend(
        sessions
            .into_iter()
            .map(|session_id| Effect::SessionRevoked { session_id }),
    );
    changes
}

pub(crate) fn sessions(
    tx: &Transaction<'_>,
    digest: [u8; 32],
    limit: u16,
    cursor: Option<String>,
    sealer: &crate::secrets::Sealer,
) -> Result<crate::accounts::AccountSessionPage, Error> {
    let now = now()?;
    let session = current_session(tx, &digest, now)?;
    let scope = crate::secrets::CursorScope::AccountSessions(session.user.id);
    let mut window = match cursor {
        Some(cursor) => crate::networks::pagination::Window::decode(sealer, scope, &cursor, now)?,
        None => crate::networks::pagination::Window::new(
            tx.query_row(
                "SELECT coalesce(max(rowid),0) FROM user_sessions",
                [],
                |r| r.get(0),
            )?,
            0,
            now,
        )?,
    };
    let mut stmt = tx.prepare("SELECT id, created_at, expires_at, last_used_at, client_label, rowid FROM user_sessions WHERE user_id=?1 AND revoked_at IS NULL AND expires_at>?2 AND credential_version=(SELECT credential_version FROM users WHERE id=?1) AND rowid<=?3 ORDER BY rowid DESC LIMIT ?4")?;
    let mut rows = stmt
        .query_map(
            params![
                session.user.id.as_uuid().as_bytes().as_slice(),
                now,
                window.upper - window.last,
                i64::from(limit) + 1
            ],
            |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let more = rows.len() > usize::from(limit);
    rows.truncate(usize::from(limit));
    let items = rows
        .into_iter()
        .map(|(id, created, expires, used, label, sequence)| {
            window.last = window.upper - sequence + 1;
            let id = SessionId::new(uuid(&id)?).map_err(|_| Error::decode("user_sessions.id"))?;
            Ok(AccountSession {
                id,
                created_at: timestamp(created)?,
                expires_at: timestamp(expires)?,
                last_used_at: used.map(timestamp).transpose()?,
                client_label: label,
                current: id == session.session_id,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let next_cursor = if more {
        Some(window.encode(sealer, scope)?)
    } else {
        None
    };
    let as_of = window.as_of()?;
    drop(stmt);
    Ok(crate::accounts::AccountSessionPage {
        items,
        next_cursor,
        as_of,
    })
}

pub(crate) fn record_management(
    tx: &Transaction<'_>,
    user: UserId,
    request: RequestId,
    action: &str,
    target: Option<SessionId>,
    previous_email: Option<&str>,
    now: i64,
) -> Result<(), Error> {
    tx.execute("INSERT INTO activity_events (id,scope,account_user_id,at,actor_user_id,action,target_user_id,target_session_id,request_id,actor_kind,actor_email,previous_email) VALUES (?1,'account',?2,?3,CASE WHEN ?4='account.resetIssued' THEN NULL ELSE ?2 END,?4,?5,?6,?7,CASE WHEN ?4='account.resetIssued' THEN 'system' ELSE 'user' END,CASE WHEN ?4='account.resetIssued' THEN NULL ELSE (SELECT email FROM users WHERE id=?2) END,?8)",params![random_uuid()?.as_bytes().as_slice(),user.as_uuid().as_bytes().as_slice(),now,action, target.is_none().then_some(user.as_uuid().as_bytes().as_slice()),target.map(|s|s.as_uuid().as_bytes().to_vec()),request.as_uuid().as_bytes().as_slice(),previous_email])?;
    Ok(())
}

pub(crate) fn revoke(
    tx: &Transaction<'_>,
    digest: [u8; 32],
    target: RevokeSessions,
    request: RequestId,
) -> Result<((), Vec<Effect>), Error> {
    let now = now()?;
    let session = current_session(tx, &digest, now)?;
    let revoked = active_sessions(tx, session.user.id, now)?
        .into_iter()
        .filter(|id| match target {
            RevokeSessions::One(target) => *id == target,
            RevokeSessions::Others => *id != session.session_id,
            RevokeSessions::All => true,
        })
        .collect::<Vec<_>>();
    let changes = if revoked.is_empty() {
        Vec::new()
    } else {
        authorization_changes(tx, session.user.id)?
    };
    let user = session.user.id.as_uuid().as_bytes().as_slice();
    let (action, target_id) = match target {
        RevokeSessions::One(id) => {
            let exists = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM user_sessions WHERE id=?1 AND user_id=?2)",
                params![id.as_uuid().as_bytes().as_slice(), user],
                |r| r.get::<_, bool>(0),
            )?;
            if !exists {
                return Err(Error::NotFound);
            }
            let changed=tx.execute("UPDATE user_sessions SET revoked_at=?1 WHERE id=?2 AND user_id=?3 AND revoked_at IS NULL",params![now,id.as_uuid().as_bytes().as_slice(),user])?;
            if changed == 0 {
                return Ok(((), Vec::new()));
            }
            ("session.revoke", Some(id))
        }
        RevokeSessions::Others => {
            tx.execute("UPDATE user_sessions SET revoked_at=?1 WHERE user_id=?2 AND id!=?3 AND revoked_at IS NULL",params![now,user,session.session_id.as_uuid().as_bytes().as_slice()])?;
            ("session.revokeOthers", None)
        }
        RevokeSessions::All => {
            tx.execute(
                "UPDATE users SET credential_version=credential_version+1 WHERE id=?1",
                [user],
            )?;
            tx.execute(
                "UPDATE user_sessions SET revoked_at=?1 WHERE user_id=?2 AND revoked_at IS NULL",
                params![now, user],
            )?;
            ("session.revokeAll", None)
        }
    };
    record_management(tx, session.user.id, request, action, target_id, None, now)?;
    Ok(((), revocation_effects(changes, revoked)))
}

pub(crate) fn candidate(tx: &Transaction<'_>, digest: [u8; 32]) -> Result<LoginCandidate, Error> {
    let session = current_session(tx, &digest, now()?)?;
    login_candidate(tx, Some(&session.user.email))?.ok_or(Error::InvalidCredentials)
}

pub(crate) fn verified_user(
    tx: &Transaction<'_>,
    digest: &[u8; 32],
    verified: &LoginCandidate,
    now: i64,
) -> Result<UserId, Error> {
    let session = current_session(tx, digest, now)?;
    if session.user.id != verified.user_id {
        return Err(Error::InvalidCredentials);
    }
    let valid=tx.query_row("SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND password_hash=?2 AND credential_version=?3 AND disabled_at IS NULL)",params![verified.user_id.as_uuid().as_bytes().as_slice(),verified.password_hash.as_str(),verified.credential_version],|r|r.get::<_,bool>(0))?;
    if !valid {
        return Err(Error::InvalidCredentials);
    }
    Ok(session.user.id)
}

fn replace_password(tx: &Transaction<'_>, user: UserId, hash: &str, now: i64) -> Result<(), Error> {
    tx.execute(
        "UPDATE users SET password_hash=?1, credential_version=credential_version+1 WHERE id=?2",
        params![hash, user.as_uuid().as_bytes().as_slice()],
    )?;
    tx.execute(
        "UPDATE user_sessions SET revoked_at=?1 WHERE user_id=?2 AND revoked_at IS NULL",
        params![now, user.as_uuid().as_bytes().as_slice()],
    )?;
    Ok(())
}

pub(crate) fn change_password(
    tx: &Transaction<'_>,
    digest: [u8; 32],
    verified: LoginCandidate,
    hash: Zeroizing<String>,
    request: RequestId,
) -> Result<((), Vec<Effect>), Error> {
    let now = now()?;
    let user = verified_user(tx, &digest, &verified, now)?;
    let sessions = active_sessions(tx, user, now)?;
    let changes = authorization_changes(tx, user)?;
    replace_password(tx, user, &hash, now)?;
    record_management(tx, user, request, "account.password", None, None, now)?;
    Ok(((), revocation_effects(changes, sessions)))
}

pub(crate) fn change_email(
    tx: &Transaction<'_>,
    digest: [u8; 32],
    verified: LoginCandidate,
    email: String,
    request: RequestId,
    configurations: &crate::configuration::Configurations,
) -> Result<(User, Vec<Effect>), Error> {
    let now = now()?;
    let user = verified_user(tx, &digest, &verified, now)?;
    let old = tx.query_row(
        "SELECT email FROM users WHERE id=?1",
        [user.as_uuid().as_bytes().as_slice()],
        |r| r.get::<_, String>(0),
    )?;
    let mut changes = Vec::new();
    if old != email {
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE email=?1)",
            [&email],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::EmailTaken);
        }
        tx.execute(
            "UPDATE users SET email=?1 WHERE id=?2",
            params![email, user.as_uuid().as_bytes().as_slice()],
        )?;
        let effects = configurations.synchronize_email(tx, user, &old, &email, request)?;
        changes.extend(effects.iter().flat_map(|effect| effect.changes()));
        changes.extend(authorization_changes(tx, user)?);
        record_management(tx, user, request, "account.email", None, Some(&old), now)?;
    }
    Ok((User { id: user, email }, changes))
}

pub(crate) fn issue_reset(
    tx: &Transaction<'_>,
    email: String,
    ttl: u32,
    request: RequestId,
) -> Result<IssuedReset, Error> {
    let now = now()?;
    let (user, version) = tx
        .query_row(
            "SELECT id,credential_version FROM users WHERE email=?1 AND disabled_at IS NULL",
            [email],
            |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let user = user_id(&user)?;
    let expires = now
        .checked_add(i64::from(ttl) * 1000)
        .ok_or(Error::Internal("password reset expiry overflow"))?;
    let (token, digest) = crate::accounts::issue_one_time("mwr_")?;
    tx.execute("UPDATE password_resets SET revoked_at=?1 WHERE user_id=?2 AND consumed_at IS NULL AND revoked_at IS NULL",params![now,user.as_uuid().as_bytes().as_slice()])?;
    tx.execute("INSERT INTO password_resets(token_hash,user_id,credential_version,expires_at) VALUES(?1,?2,?3,?4)",params![digest.as_slice(),user.as_uuid().as_bytes().as_slice(),version,expires])?;
    record_management(tx, user, request, "account.resetIssued", None, None, now)?;
    Ok(IssuedReset {
        token,
        expires_at: timestamp(expires)?,
    })
}

fn reset(tx: &Transaction<'_>, digest: [u8; 32], now: i64) -> Result<(UserId, i64), Error> {
    let row = tx
        .query_row(
            "SELECT p.user_id, p.expires_at, p.consumed_at, p.revoked_at,
                p.credential_version = u.credential_version
         FROM password_resets p JOIN users u ON u.id = p.user_id
         WHERE p.token_hash = ?1 AND u.disabled_at IS NULL",
            [digest.as_slice()],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            },
        )
        .optional()?
        .ok_or(Error::InvalidReset)?;
    // Only an exact credential hash match can expose a lifecycle reason. Used
    // takes precedence because consuming a reset also advances credential_version.
    if row.2.is_some() {
        return Err(Error::ResetUsed);
    }
    if row.3.is_some() || !row.4 {
        return Err(Error::ResetReplaced);
    }
    if row.1 <= now {
        return Err(Error::ResetExpired);
    }
    Ok((user_id(&row.0)?, row.1))
}

pub(crate) fn inspect_reset(
    tx: &Transaction<'_>,
    digest: [u8; 32],
) -> Result<ResetInspection, Error> {
    let (_, expires) = reset(tx, digest, now()?)?;
    Ok(ResetInspection {
        expires_at: timestamp(expires)?,
    })
}

pub(crate) fn redeem_reset(
    tx: &Transaction<'_>,
    digest: [u8; 32],
    hash: Zeroizing<String>,
    request: RequestId,
) -> Result<((), Vec<Effect>), Error> {
    let now = now()?;
    let (user, _) = reset(tx, digest, now)?;
    let sessions = active_sessions(tx, user, now)?;
    let changes = authorization_changes(tx, user)?;
    replace_password(tx, user, &hash, now)?;
    tx.execute(
        "UPDATE password_resets SET consumed_at=?1 WHERE token_hash=?2",
        params![now, digest.as_slice()],
    )?;
    record_management(tx, user, request, "account.passwordReset", None, None, now)?;
    Ok(((), revocation_effects(changes, sessions)))
}
