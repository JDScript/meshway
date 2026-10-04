//! Permission-checked, creation-bounded management history queries.
use crate::error::Error;
use crate::{
    accounts::token_digest,
    networks::pagination::Window,
    secrets::{CursorScope, Sealer},
    store::authorization::authorize_manager,
    store::{Handle, authorization::current_session, history as store},
};
use meshway_model::{ActivityId, NetworkId, RequestId};
use meshway_protocol::history::*;
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
                let upper = store::activity_upper(tx)?;
                let mut window = match cursor {
                    Some(c) => Window::decode(&sealer, scope, &c, now)?,
                    None => Window::new(upper, 0, now)?,
                };
                let mut rows = store::activities(
                    tx,
                    network,
                    user,
                    window.upper - window.last,
                    i64::from(limit) + 1,
                )?;
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
                let activity = store::activity(tx, network, activity)?;
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
        self.handle
            .read(move |tx| {
                let now = chrono::Utc::now().timestamp_millis();
                let user = current_session(tx, &digest, now)?.user.id;
                authorize_manager(tx, network, user)?;
                let current_revision = store::revision(tx, network)?;
                let scope = CursorScope::Revisions { user, network };
                let mut window = match cursor {
                    Some(cursor) => Window::decode(&sealer, scope, &cursor, now)?,
                    None => Window::new(current_revision.get() as i64, 0, now)?,
                };
                let mut rows = store::revisions(
                    tx,
                    network,
                    current_revision,
                    window.upper - window.last,
                    i64::from(limit) + 1,
                )?;
                let more = rows.len() > usize::from(limit);
                rows.truncate(usize::from(limit));
                if let Some(last) = rows.last() {
                    window.last = window.upper - last.revision.get() as i64 + 1;
                }
                let next_cursor = if more {
                    Some(window.encode(&sealer, scope)?)
                } else {
                    None
                };
                let as_of = window.as_of()?;
                Ok(ConfigurationRevisionList {
                    request_id: id,
                    current_revision,
                    items: rows,
                    next_cursor,
                    as_of,
                })
            })
            .await
    }
}
