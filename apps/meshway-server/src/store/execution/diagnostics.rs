//! SQL behind diagnostic (observation) demand collection.
use crate::error::Error;
use meshway_configuration::execution::DiagnosticDemand;
use meshway_model::{
    Digest, InstanceId, MembershipId, NetworkId, ObservationId, ObservationSubscriptionId,
    Revision, SessionId, UserId,
};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::BTreeMap;

pub(crate) fn collect(
    tx: &Transaction<'_>,
    network: NetworkId,
    now: i64,
) -> Result<BTreeMap<ObservationId, DiagnosticDemand>, Error> {
    let mut statement = tx.prepare("SELECT t.payload,max(s.expires_at) FROM diagnostic_tasks t
      JOIN diagnostic_subscriptions s ON s.task_id=t.id
      JOIN user_sessions u ON u.id=s.session_id
      JOIN users a ON a.id=u.user_id
      JOIN memberships m ON m.id=s.membership_id AND m.network_id=t.network_id AND m.user_id=u.user_id
      JOIN networks n ON n.id=t.network_id AND n.current_revision=t.revision
      JOIN node_execution e ON e.network_id=t.network_id AND e.node_id=t.node_id AND e.instance_id=t.instance_id AND e.boot_id=t.boot_id
      JOIN nodes node ON node.network_id=t.network_id AND node.id=t.node_id
      WHERE t.network_id=?1 AND s.cancelled=0 AND s.expires_at>?2 AND u.revoked_at IS NULL AND u.expires_at>?2
       AND a.disabled_at IS NULL AND a.credential_version=u.credential_version AND m.revoked_at IS NULL
       AND n.deleted_at IS NULL AND node.credentials_revoked_at IS NULL AND e.instance_expires_at>?2
      GROUP BY t.id ORDER BY t.id")?;
    let rows = statement.query_map(params![network.as_uuid().as_bytes().as_slice(), now], |r| {
        Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?))
    })?;
    rows.map(|row| {
        let (payload, expiry) = row?;
        let mut demand: DiagnosticDemand = super::json(&payload)?;
        demand.expires_at_ms = expiry as u64;
        Ok((demand.id, demand))
    })
    .collect()
}

pub(crate) fn membership(
    tx: &Transaction<'_>,
    network: NetworkId,
    user: UserId,
) -> Result<MembershipId, Error> {
    let membership = tx.query_row(
        "SELECT id FROM memberships WHERE network_id=?1 AND user_id=?2 AND revoked_at IS NULL",
        params![
            network.as_uuid().as_bytes().as_slice(),
            user.as_uuid().as_bytes().as_slice()
        ],
        |r| r.get::<_, Vec<u8>>(0),
    )?;
    super::id(membership)
}

pub(crate) fn subscription_matches(
    tx: &Transaction<'_>,
    subscription: ObservationSubscriptionId,
    session: SessionId,
    digest: Digest,
) -> Result<Option<bool>, Error> {
    let existing: Option<(Vec<u8>, Vec<u8>)> = tx
        .query_row(
            "SELECT session_id,request_digest FROM diagnostic_subscriptions WHERE id=?1",
            [subscription.as_uuid().as_bytes().as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(existing.map(|(previous_session, previous_digest)| {
        previous_session == session.as_uuid().as_bytes() && previous_digest == digest.as_bytes()
    }))
}

pub(crate) fn reusable_task(
    tx: &Transaction<'_>,
    network: NetworkId,
    key: Digest,
    now: i64,
) -> Result<Option<ObservationId>, Error> {
    let existing: Option<Vec<u8>> = tx.query_row("SELECT t.id FROM diagnostic_tasks t WHERE t.network_id=?1 AND t.reuse_key=?2 AND EXISTS(SELECT 1 FROM diagnostic_subscriptions s WHERE s.task_id=t.id AND s.cancelled=0 AND s.expires_at>?3) LIMIT 1",params![network.as_uuid().as_bytes().as_slice(),key.as_bytes().as_slice(),now],|r|r.get(0)).optional()?;
    existing.map(super::id).transpose()
}

pub(crate) fn subscription_counts(
    tx: &Transaction<'_>,
    session: SessionId,
    now: i64,
) -> Result<(i64, i64), Error> {
    let count = tx.query_row("SELECT count(*) FROM diagnostic_subscriptions WHERE session_id=?1 AND cancelled=0 AND expires_at>?2",params![session.as_uuid().as_bytes().as_slice(),now],|r|r.get(0))?;
    let total = tx.query_row("SELECT count(*) FROM diagnostic_subscriptions", [], |r| {
        r.get(0)
    })?;
    Ok((count, total))
}

pub(crate) fn insert_task(
    tx: &Transaction<'_>,
    network: NetworkId,
    boot: InstanceId,
    revision: Revision,
    key: Digest,
    demand: &DiagnosticDemand,
) -> Result<(), Error> {
    tx.execute("INSERT INTO diagnostic_tasks(id,network_id,node_id,instance_id,boot_id,revision,reuse_key,payload) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![demand.id.as_uuid().as_bytes().as_slice(),network.as_uuid().as_bytes().as_slice(),demand.source.as_uuid().as_bytes().as_slice(),demand.instance_id.as_uuid().as_bytes().as_slice(),boot.as_uuid().as_bytes().as_slice(),revision.get() as i64,key.as_bytes().as_slice(),super::bytes(demand)?])?;
    Ok(())
}

pub(crate) fn insert_subscription(
    tx: &Transaction<'_>,
    subscription: ObservationSubscriptionId,
    task: ObservationId,
    session: SessionId,
    membership: MembershipId,
    digest: Digest,
    expires: i64,
) -> Result<(), Error> {
    tx.execute("INSERT INTO diagnostic_subscriptions(id,task_id,session_id,membership_id,request_digest,expires_at) VALUES(?1,?2,?3,?4,?5,?6)",params![subscription.as_uuid().as_bytes().as_slice(),task.as_uuid().as_bytes().as_slice(),session.as_uuid().as_bytes().as_slice(),membership.as_uuid().as_bytes().as_slice(),digest.as_bytes().as_slice(),expires])?;
    tx.execute(
        "UPDATE diagnostic_tasks SET dirty=1 WHERE id=?1",
        [task.as_uuid().as_bytes().as_slice()],
    )?;
    Ok(())
}

pub(crate) fn admitted_task(
    tx: &Transaction<'_>,
    subscription: ObservationSubscriptionId,
    session: SessionId,
    now: i64,
    network: NetworkId,
    revision: Revision,
    boot: InstanceId,
) -> Result<ObservationId, Error> {
    let id: Vec<u8> = tx.query_row("SELECT t.id FROM diagnostic_subscriptions s JOIN diagnostic_tasks t ON t.id=s.task_id WHERE s.id=?1 AND s.session_id=?2 AND s.cancelled=0 AND s.expires_at>?3 AND t.network_id=?4 AND t.revision=?5 AND t.boot_id=?6",params![subscription.as_uuid().as_bytes().as_slice(),session.as_uuid().as_bytes().as_slice(),now,network.as_uuid().as_bytes().as_slice(),revision.get() as i64,boot.as_uuid().as_bytes().as_slice()],|r|r.get(0)).optional()?.ok_or(Error::NotFound)?;
    super::id(id)
}

pub(crate) fn mark_published(tx: &Transaction<'_>, network: NetworkId) -> Result<(), Error> {
    tx.execute(
        "UPDATE diagnostic_tasks SET dirty=0 WHERE network_id=?1",
        [network.as_uuid().as_bytes().as_slice()],
    )?;
    Ok(())
}

pub(crate) fn active_subscriptions(
    tx: &Transaction<'_>,
) -> Result<Vec<(ObservationSubscriptionId, i64)>, Error> {
    let mut statement =
        tx.prepare("SELECT id,expires_at FROM diagnostic_subscriptions WHERE cancelled=0")?;
    let rows = statement
        .query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(id, expires)| Ok((super::id(id)?, expires)))
        .collect()
}

pub(crate) fn update_subscription_expiry(
    tx: &Transaction<'_>,
    id: ObservationSubscriptionId,
    expires: i64,
) -> Result<(), Error> {
    tx.execute(
        "UPDATE diagnostic_subscriptions SET expires_at=?2 WHERE id=?1 AND cancelled=0",
        params![id.as_uuid().as_bytes().as_slice(), expires],
    )?;
    Ok(())
}

pub(crate) fn cancel_subscription(
    tx: &Transaction<'_>,
    id: ObservationSubscriptionId,
) -> Result<(), Error> {
    tx.execute(
        "UPDATE diagnostic_subscriptions SET cancelled=1 WHERE id=?1 AND cancelled=0",
        [id.as_uuid().as_bytes().as_slice()],
    )?;
    Ok(())
}

pub(crate) fn dirty_networks(tx: &Transaction<'_>) -> Result<Vec<NetworkId>, Error> {
    let mut stmt = tx.prepare("SELECT DISTINCT network_id FROM diagnostic_tasks WHERE dirty=1")?;
    let networks = stmt
        .query_map([], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    networks.into_iter().map(super::id).collect()
}

pub(crate) fn prune(tx: &Transaction<'_>, before: i64) -> Result<(), Error> {
    tx.execute(
        "DELETE FROM diagnostic_subscriptions WHERE cancelled=1 AND expires_at<?1",
        [before],
    )?;
    tx.execute("DELETE FROM diagnostic_tasks WHERE NOT EXISTS(SELECT 1 FROM diagnostic_subscriptions WHERE task_id=diagnostic_tasks.id)",[])?;
    Ok(())
}
