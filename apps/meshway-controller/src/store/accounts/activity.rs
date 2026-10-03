use meshway_model::{ActivityId, RequestId};
use rusqlite::{Transaction, params};

use crate::accounts::{AccountError, AuthenticatedSession, random_uuid};

/// Server-determined account actions. There is no caller-controlled action,
/// actor, scope, arbitrary metadata, or request body in the activity writer.
pub(super) enum AccountActivity {
    Register,
    Login,
    Logout,
}

pub(super) fn record(
    transaction: &Transaction<'_>,
    session: &AuthenticatedSession,
    at: i64,
    request: RequestId,
    activity: AccountActivity,
) -> Result<(), AccountError> {
    let id = ActivityId::new(random_uuid()?).map_err(|_| AccountError::Unavailable)?;
    let user_id = session.user.id.as_uuid().as_bytes().as_slice();
    let session_id = session.session_id.as_uuid().as_bytes().as_slice();
    let (action, target_user, target_session) = match activity {
        AccountActivity::Register => ("account.register", Some(user_id), None),
        AccountActivity::Login => ("session.login", None, Some(session_id)),
        AccountActivity::Logout => ("session.logout", None, Some(session_id)),
    };
    transaction.execute(
        "INSERT INTO activity_events (id, scope, account_user_id, at, actor_user_id, action, target_user_id, target_session_id, request_id) VALUES (?1, 'account', ?2, ?3, ?2, ?4, ?5, ?6, ?7)",
        params![id.as_uuid().as_bytes().as_slice(), user_id, at, action, target_user, target_session, request.as_uuid().as_bytes().as_slice()],
    )?;
    Ok(())
}
