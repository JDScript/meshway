//! SQL behind configuration version history and activity listings.
use crate::error::Error;
use meshway_model::{ActivityId, NetworkId, NodeId, RequestId, Revision, UserId};
use meshway_protocol::history::{
    ActivityAction, ActivityActor, ActivityEvent, ActivityTarget, ConfigurationChangeCause,
    ConfigurationRevisionSummary,
};
use rusqlite::{OptionalExtension, Transaction, params};

pub(crate) fn activity_upper(tx: &Transaction<'_>) -> Result<i64, Error> {
    Ok(tx.query_row(
        "SELECT coalesce(max(sequence),0) FROM activity_events",
        [],
        |r| r.get::<_, i64>(0),
    )?)
}

pub(crate) fn activities(
    tx: &Transaction<'_>,
    network: Option<NetworkId>,
    user: UserId,
    upper: i64,
    limit: i64,
) -> Result<Vec<(i64, ActivityEvent)>, Error> {
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
    Ok(stmt
        .query_map(params![subject, upper, limit], read)?
        .collect::<Result<Vec<_>, _>>()?)
}

pub(crate) fn activity(
    tx: &Transaction<'_>,
    network: NetworkId,
    activity: ActivityId,
) -> Result<ActivityEvent, Error> {
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
    Ok(activity)
}

pub(crate) fn revisions(
    tx: &Transaction<'_>,
    network: NetworkId,
    current_revision: Revision,
    upper: i64,
    limit: i64,
) -> Result<Vec<ConfigurationRevisionSummary>, Error> {
    let mut stmt = tx.prepare("SELECT revision,created_at,created_by,cause,rollback_from,created_by_email FROM configuration_versions WHERE network_id=?1 AND revision<=?2 ORDER BY revision DESC LIMIT ?3")?;
    Ok(stmt
        .query_map(
            params![network.as_uuid().as_bytes().as_slice(), upper, limit],
            |row| {
                let revision = revision_value(row.get::<_, i64>(0)?)?;
                Ok(ConfigurationRevisionSummary {
                    revision,
                    created_at: chrono::DateTime::from_timestamp_millis(row.get(1)?)
                        .ok_or(rusqlite::Error::InvalidQuery)?,
                    created_by: ActivityActor::User {
                        user_id: uuid(row, 2, UserId::new)?,
                        email: row.get(5)?,
                    },
                    cause: ConfigurationChangeCause::try_from(row.get::<_, String>(3)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    current: revision == current_revision,
                    rollback_from: row
                        .get::<_, Option<i64>>(4)?
                        .map(revision_value)
                        .transpose()?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?)
}

pub(crate) fn revision(tx: &Transaction<'_>, network: NetworkId) -> Result<Revision, Error> {
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
const SELECT: &str = "SELECT sequence,id,at,actor_kind,actor_user_id,action,target_user_id,target_session_id,network_id,target_membership_id,target_join_link_id,before_revision,after_revision,request_id,actor_email,previous_email,actor_node_id,target_node_id,target_profile FROM activity_events";
fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, ActivityEvent)> {
    let actor = match r.get::<_, String>(3)?.as_str() {
        "user" => ActivityActor::User {
            user_id: uuid(r, 4, UserId::new)?,
            email: r.get(14)?,
        },
        "system" => ActivityActor::System {},
        "node" => ActivityActor::Node {
            node_id: uuid(r, 16, NodeId::new)?,
        },
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
    let target = if r.get::<_, Option<Vec<u8>>>(17)?.is_some() {
        ActivityTarget::RoutingSelection {
            node_id: uuid(r, 17, NodeId::new)?,
            profile: r.get(18)?,
        }
    } else if r.get::<_, Option<Vec<u8>>>(7)?.is_some() {
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
