//! Permission-checked, creation-bounded management history queries.
use crate::error::Error;
use crate::{
    accounts::token_digest,
    networks::pagination::Window,
    secrets::{CursorScope, Sealer},
    store::authorization::authorize_manager,
    store::{Handle, authorization::current_session},
};
use meshway_model::{ActivityId, NetworkId, RequestId, Revision, UserId};
use meshway_protocol::history::*;
use rusqlite::{OptionalExtension, Transaction, params};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct History {
    handle: Handle,
    sealer: Arc<Sealer>,
}
impl History {
    pub(crate) fn new(handle: Handle, sealer: Arc<Sealer>) -> Self {
        Self { handle, sealer }
    }
    pub(crate) async fn activities(
        &self,
        token: &str,
        network: Option<NetworkId>,
        limit: u16,
        cursor: Option<String>,
        id: RequestId,
    ) -> Result<ActivityListResponse, Error> {
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidRequest);
        }
        let digest = token_digest(token)?;
        let sealer = Arc::clone(&self.sealer);
        self.handle
            .read(move |tx| {
                let now = chrono::Utc::now().timestamp_millis();
                let user = current_session(tx, &digest, now)?.user.id;
                if let Some(network) = network {
                    authorize_manager(tx, network, user)?;
                }
                let scope = match network {
                    Some(network) => CursorScope::NetworkActivity { user, network },
                    None => CursorScope::AccountActivity(user),
                };
                let upper = tx.query_row(
                    "SELECT coalesce(max(sequence),0) FROM activity_events",
                    [],
                    |r| r.get::<_, i64>(0),
                )?;
                let mut window = match cursor {
                    Some(c) => Window::decode(&sealer, scope, &c, now)?,
                    None => Window::new(upper, 0, now)?,
                };
                let field = if network.is_some() {
                    "network_id"
                } else {
                    "account_user_id"
                };
                let subject = network
                    .map(|n| n.as_uuid().as_bytes().to_vec())
                    .unwrap_or_else(|| user.as_uuid().as_bytes().to_vec());
                let mut stmt = tx.prepare(&format!(
                    "{SELECT} WHERE {field}=?1 AND sequence<=?2 ORDER BY sequence DESC LIMIT ?3"
                ))?;
                let mut rows = stmt
                    .query_map(
                        params![subject, window.upper - window.last, i64::from(limit) + 1],
                        read,
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                let more = rows.len() > usize::from(limit);
                rows.truncate(usize::from(limit));
                let items = rows
                    .into_iter()
                    .map(|(seq, item)| {
                        window.last = window.upper - seq + 1;
                        item
                    })
                    .collect();
                let next_cursor = if more {
                    Some(window.encode(&sealer, scope)?)
                } else {
                    None
                };
                let as_of = window.as_of()?;
                drop(stmt);
                Ok(ActivityListResponse {
                    request_id: id,
                    items,
                    next_cursor,
                    as_of,
                })
            })
            .await
    }
    pub(crate) async fn activity(
        &self,
        token: &str,
        network: NetworkId,
        activity: ActivityId,
        id: RequestId,
    ) -> Result<ActivityResponse, Error> {
        let digest = token_digest(token)?;
        self.handle
            .read(move |tx| {
                let user = current_session(tx, &digest, chrono::Utc::now().timestamp_millis())?
                    .user
                    .id;
                authorize_manager(tx, network, user)?;
                let (_, activity) = tx
                    .query_row(
                        &format!("{SELECT} WHERE network_id=?1 AND id=?2"),
                        params![
                            network.as_uuid().as_bytes().as_slice(),
                            activity.as_uuid().as_bytes().as_slice()
                        ],
                        read,
                    )
                    .optional()?
                    .ok_or(Error::NotFound)?;
                Ok(ActivityResponse {
                    request_id: id,
                    activity,
                })
            })
            .await
    }
    pub(crate) async fn revisions(
        &self,
        token: &str,
        network: NetworkId,
        limit: u16,
        cursor: Option<String>,
        id: RequestId,
    ) -> Result<ConfigurationRevisionList, Error> {
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidRequest);
        }
        let digest = token_digest(token)?;
        let sealer = Arc::clone(&self.sealer);
        self.handle.read(move |tx| {
            let now = chrono::Utc::now().timestamp_millis();
            let user = current_session(tx, &digest, now)?.user.id;
            authorize_manager(tx, network, user)?;
            let current_revision = revision(tx, network)?;
            let scope = CursorScope::Revisions { user, network };
            let mut window = match cursor {
                Some(cursor) => Window::decode(&sealer, scope, &cursor, now)?,
                None => Window::new(current_revision.get() as i64, 0, now)?,
            };
            let mut stmt = tx.prepare("SELECT revision,created_at,created_by,cause,rollback_from,created_by_email FROM configuration_versions WHERE network_id=?1 AND revision<=?2 ORDER BY revision DESC LIMIT ?3")?;
            let mut rows = stmt.query_map(
                params![network.as_uuid().as_bytes().as_slice(), window.upper-window.last, i64::from(limit)+1],
                |row| {
                    let revision = revision_value(row.get::<_, i64>(0)?)?;
                    Ok(ConfigurationRevisionSummary {
                        revision,
                        created_at: chrono::DateTime::from_timestamp_millis(row.get(1)?).ok_or(rusqlite::Error::InvalidQuery)?,
                        created_by: ActivityActor::User { user_id: uuid(row, 2, UserId::new)?, email: row.get(5)? },
                        cause: ConfigurationChangeCause::try_from(row.get::<_, String>(3)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
                        current: revision == current_revision,
                        rollback_from: row.get::<_, Option<i64>>(4)?.map(revision_value).transpose()?,
                    })
                },
            )?.collect::<Result<Vec<_>, _>>()?;
            let more = rows.len() > usize::from(limit);
            rows.truncate(usize::from(limit));
            if let Some(last) = rows.last() { window.last = window.upper - last.revision.get() as i64 + 1; }
            let next_cursor = if more { Some(window.encode(&sealer, scope)?) } else { None };
            let as_of = window.as_of()?;
            drop(stmt);
            Ok(ConfigurationRevisionList { request_id: id, current_revision, items: rows, next_cursor, as_of })
        }).await
    }
}
fn revision(tx: &Transaction<'_>, network: NetworkId) -> Result<Revision, Error> {
    revision_value(tx.query_row(
        "SELECT current_revision FROM networks WHERE id=?1",
        [network.as_uuid().as_bytes().as_slice()],
        |r| r.get::<_, i64>(0),
    )?)
    .map_err(|_| Error::decode("networks.current_revision"))
}
fn uuid<T>(
    r: &rusqlite::Row<'_>,
    i: usize,
    make: impl FnOnce(uuid::Uuid) -> Result<T, meshway_model::InvalidId>,
) -> rusqlite::Result<T> {
    let bytes = r.get::<_, Vec<u8>>(i)?;
    make(uuid::Uuid::from_slice(&bytes).map_err(|_| rusqlite::Error::InvalidQuery)?)
        .map_err(|_| rusqlite::Error::InvalidQuery)
}
fn revision_value(value: i64) -> rusqlite::Result<Revision> {
    Revision::new(u64::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery)?)
        .map_err(|_| rusqlite::Error::InvalidQuery)
}
const SELECT: &str = "SELECT sequence,id,at,actor_kind,actor_user_id,action,target_user_id,target_session_id,network_id,target_membership_id,target_join_link_id,before_revision,after_revision,request_id,actor_email,previous_email FROM activity_events";
fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, ActivityEvent)> {
    let actor = match r.get::<_, String>(3)?.as_str() {
        "user" => ActivityActor::User {
            user_id: uuid(r, 4, UserId::new)?,
            email: r.get(14)?,
        },
        "system" => ActivityActor::System {},
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let before_revision = r
        .get::<_, Option<i64>>(11)?
        .map(revision_value)
        .transpose()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let after_revision = r
        .get::<_, Option<i64>>(12)?
        .map(revision_value)
        .transpose()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let target = if r.get::<_, Option<Vec<u8>>>(7)?.is_some() {
        ActivityTarget::Session {
            session_id: uuid(r, 7, meshway_model::SessionId::new)?,
        }
    } else if r.get::<_, Option<Vec<u8>>>(6)?.is_some() {
        ActivityTarget::Account {
            user_id: uuid(r, 6, UserId::new)?,
            previous_email: r.get(15)?,
        }
    } else if r.get::<_, Option<Vec<u8>>>(9)?.is_some() {
        ActivityTarget::Membership {
            membership_id: uuid(r, 9, meshway_model::MembershipId::new)?,
        }
    } else if r.get::<_, Option<Vec<u8>>>(10)?.is_some() {
        ActivityTarget::JoinLink {
            link_id: uuid(r, 10, meshway_model::JoinLinkId::new)?,
        }
    } else if let Some(revision) = after_revision {
        ActivityTarget::Configuration { revision }
    } else {
        ActivityTarget::Network {
            network_id: uuid(r, 8, NetworkId::new)?,
        }
    };
    Ok((
        r.get(0)?,
        ActivityEvent {
            id: uuid(r, 1, ActivityId::new)?,
            at: chrono::DateTime::from_timestamp_millis(r.get(2)?)
                .ok_or(rusqlite::Error::InvalidQuery)?,
            actor,
            action: ActivityAction::try_from(r.get::<_, String>(5)?)
                .map_err(|_| rusqlite::Error::InvalidQuery)?,
            target,
            before_revision,
            after_revision,
            request_id: uuid(r, 13, RequestId::new)?,
        },
    ))
}
