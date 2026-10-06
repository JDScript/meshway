//! SQL behind diagnostic (observation) demand collection.
use crate::error::Error;
use crate::execution::diagnostics::{DiagnosticCompilation, DiagnosticDecision};
use crate::observations::SubscriberPrincipal;
use meshway_configuration::execution::DiagnosticDemand;
use meshway_model::{
    Digest, InstanceId, MembershipId, NetworkId, ObservationId, ObservationSubscriptionId,
    Revision, UserId,
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
       LEFT JOIN user_sessions u ON u.id=s.session_id
       LEFT JOIN users a ON a.id=u.user_id
       LEFT JOIN memberships m ON m.id=s.membership_id AND m.network_id=t.network_id AND m.user_id=u.user_id
      JOIN networks n ON n.id=t.network_id AND n.current_revision=t.revision
      JOIN node_execution e ON e.network_id=t.network_id AND e.node_id=t.node_id AND e.instance_id=t.instance_id AND e.boot_id=t.boot_id
      JOIN nodes node ON node.network_id=t.network_id AND node.id=t.node_id
       WHERE t.network_id=?1 AND s.cancelled=0 AND s.expires_at>?2
        AND ((s.principal_kind='user_session' AND u.revoked_at IS NULL AND u.expires_at>?2
        AND a.disabled_at IS NULL AND a.credential_version=u.credential_version AND m.id IS NOT NULL AND m.revoked_at IS NULL)
        OR (s.principal_kind='node_instance' AND s.node_network_id=t.network_id AND s.node_id=t.node_id
        AND s.instance_id=e.instance_id AND s.instance_generation=e.generation AND s.boot_id=e.boot_id
        AND EXISTS(SELECT 1 FROM node_keys k WHERE k.id=s.control_key_id AND k.network_id=t.network_id AND k.node_id=t.node_id AND k.purpose='control' AND k.revoked_at IS NULL AND k.retired_at IS NULL)
        AND EXISTS(SELECT 1 FROM node_sessions ns WHERE ns.network_id=t.network_id AND ns.node_id=t.node_id AND ns.key_id=s.control_key_id AND ns.boot_id=e.boot_id AND ns.expires_at>?2)))
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
    principal: &SubscriberPrincipal,
    digest: Digest,
) -> Result<Option<bool>, Error> {
    struct Existing {
        session: Option<Vec<u8>>,
        key: Option<Vec<u8>>,
        digest: Vec<u8>,
    }
    let existing = tx
        .query_row(
            "SELECT session_id,control_key_id,request_digest FROM diagnostic_subscriptions WHERE id=?1",
            [subscription.as_uuid().as_bytes().as_slice()],
            |r| Ok(Existing { session: r.get(0)?, key: r.get(1)?, digest: r.get(2)? }),
        )
        .optional()?;
    Ok(existing.map(|previous| {
        let owner = match principal {
            SubscriberPrincipal::UserSession { session, .. } => {
                previous.session.as_deref() == Some(session.as_uuid().as_bytes().as_slice())
            }
            SubscriberPrincipal::NodeInstance { key, .. } => {
                previous.key.as_deref() == Some(key.as_uuid().as_bytes().as_slice())
            }
        };
        owner && previous.digest == digest.as_bytes()
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
    principal: &SubscriberPrincipal,
    now: i64,
) -> Result<(i64, i64), Error> {
    let (column, id) = match principal {
        SubscriberPrincipal::UserSession { session, .. } => ("session_id", session.as_uuid()),
        SubscriberPrincipal::NodeInstance { node, .. } => ("node_id", node.as_uuid()),
    };
    let count = tx.query_row(&format!("SELECT count(*) FROM diagnostic_subscriptions WHERE {column}=?1 AND cancelled=0 AND expires_at>?2"),params![id.as_bytes().as_slice(),now],|r|r.get(0))?;
    let total = tx.query_row(
        "SELECT count(*) FROM diagnostic_subscriptions WHERE cancelled=0",
        [],
        |r| r.get(0),
    )?;
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

pub(crate) struct SubscriptionInsert<'a> {
    pub subscription: ObservationSubscriptionId,
    pub task: ObservationId,
    pub principal: &'a SubscriberPrincipal,
    pub membership: Option<MembershipId>,
    pub digest: Digest,
    pub expires: i64,
    pub request: Option<&'a meshway_protocol::node_observations::CreateNodeObservation>,
}
pub(crate) fn insert_subscription(
    tx: &Transaction<'_>,
    input: SubscriptionInsert<'_>,
) -> Result<(), Error> {
    let SubscriptionInsert {
        subscription,
        task,
        principal,
        membership,
        digest,
        expires,
        request,
    } = input;
    match principal {
        SubscriberPrincipal::UserSession { session, .. } => {
            let membership = membership.ok_or(Error::PermissionDenied)?;
            tx.execute("INSERT INTO diagnostic_subscriptions(id,task_id,session_id,membership_id,request_digest,expires_at) VALUES(?1,?2,?3,?4,?5,?6)",params![subscription.as_uuid().as_bytes().as_slice(),task.as_uuid().as_bytes().as_slice(),session.as_uuid().as_bytes().as_slice(),membership.as_uuid().as_bytes().as_slice(),digest.as_bytes().as_slice(),expires])?;
        }
        SubscriberPrincipal::NodeInstance {
            network,
            node,
            key,
            instance,
            generation,
            boot,
        } => {
            tx.execute("INSERT INTO diagnostic_subscriptions(id,task_id,principal_kind,node_network_id,node_id,control_key_id,instance_id,instance_generation,boot_id,request_payload,request_digest,expires_at) VALUES(?1,?2,'node_instance',?3,?4,?5,?6,?7,?8,?9,?10,?11)", params![subscription.as_uuid().as_bytes().as_slice(),task.as_uuid().as_bytes().as_slice(),network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice(),key.as_uuid().as_bytes().as_slice(),instance.as_uuid().as_bytes().as_slice(),generation.get() as i64,boot.as_uuid().as_bytes().as_slice(),super::bytes(request.ok_or(Error::InvalidRequest)?)?,digest.as_bytes().as_slice(),expires])?;
        }
    }
    tx.execute(
        "UPDATE diagnostic_tasks SET dirty=1 WHERE id=?1",
        [task.as_uuid().as_bytes().as_slice()],
    )?;
    Ok(())
}

pub(crate) fn admitted_task(
    tx: &Transaction<'_>,
    subscription: ObservationSubscriptionId,
    principal: &SubscriberPrincipal,
    now: i64,
    network: NetworkId,
    revision: Revision,
    boot: InstanceId,
) -> Result<ObservationId, Error> {
    let (column, owner) = match principal {
        SubscriberPrincipal::UserSession { session, .. } => ("s.session_id", session.as_uuid()),
        SubscriberPrincipal::NodeInstance { key, .. } => ("s.control_key_id", key.as_uuid()),
    };
    let row = tx.query_row(&format!("SELECT t.id,s.cancelled,t.compilation FROM diagnostic_subscriptions s JOIN diagnostic_tasks t ON t.id=s.task_id WHERE s.id=?1 AND {column}=?2 AND s.expires_at>?3 AND t.network_id=?4 AND t.revision=?5 AND t.boot_id=?6"),params![subscription.as_uuid().as_bytes().as_slice(),owner.as_bytes().as_slice(),now,network.as_uuid().as_bytes().as_slice(),revision.get() as i64,boot.as_uuid().as_bytes().as_slice()],|r|Ok((r.get::<_,Vec<u8>>(0)?, r.get::<_,bool>(1)?, r.get::<_,Option<Vec<u8>>>(2)?))).optional()?.ok_or(Error::NotFound)?;
    // Unsupported is a bounded terminal capability result. Reading it cannot
    // restore a cancelled subscription to collect() or create executable work.
    if row.1
        && !row
            .2
            .as_deref()
            .map(super::json::<DiagnosticCompilation>)
            .transpose()?
            .is_some_and(|compilation| {
                matches!(compilation.decision, DiagnosticDecision::Unsupported { .. })
            })
    {
        return Err(Error::NotFound);
    }
    super::id(row.0)
}

pub(crate) fn compilation(
    tx: &Transaction<'_>,
    id: ObservationId,
) -> Result<(DiagnosticDemand, DiagnosticCompilation), Error> {
    let (demand, decision) = tx.query_row(
        "SELECT payload,compilation FROM diagnostic_tasks WHERE id=?1",
        [id.as_uuid().as_bytes().as_slice()],
        |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Option<Vec<u8>>>(1)?)),
    )?;
    Ok((
        super::json(&demand)?,
        super::json(&decision.ok_or(Error::StateConflict)?)?,
    ))
}

pub(crate) fn record_compilation(
    tx: &Transaction<'_>,
    id: ObservationId,
    decision: &DiagnosticCompilation,
) -> Result<(), Error> {
    tx.execute(
        "UPDATE diagnostic_tasks SET compilation=?2 WHERE id=?1",
        params![id.as_uuid().as_bytes().as_slice(), super::bytes(decision)?],
    )?;
    Ok(())
}

pub(crate) struct NodeSubscription {
    pub principal: SubscriberPrincipal,
    pub request: meshway_protocol::node_observations::CreateNodeObservation,
    pub demand: DiagnosticDemand,
}
pub(crate) fn node_subscription(
    tx: &Transaction<'_>,
    id: ObservationSubscriptionId,
) -> Result<NodeSubscription, Error> {
    let row = tx.query_row("SELECT s.node_network_id,s.node_id,s.control_key_id,s.instance_id,s.instance_generation,s.boot_id,s.request_payload,t.payload FROM diagnostic_subscriptions s JOIN diagnostic_tasks t ON t.id=s.task_id WHERE s.id=?1 AND s.principal_kind='node_instance'", [id.as_uuid().as_bytes().as_slice()], |r| Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,Vec<u8>>(3)?,r.get::<_,i64>(4)?,r.get::<_,Vec<u8>>(5)?,r.get::<_,Vec<u8>>(6)?,r.get::<_,Vec<u8>>(7)?))).optional()?.ok_or(Error::NotFound)?;
    Ok(NodeSubscription {
        principal: SubscriberPrincipal::NodeInstance {
            network: super::id(row.0)?,
            node: super::id(row.1)?,
            key: super::id(row.2)?,
            instance: super::id(row.3)?,
            generation: meshway_model::Version::new(row.4 as u64)
                .map_err(|_| Error::decode("diagnostic generation"))?,
            boot: super::id(row.5)?,
        },
        request: super::json(&row.6)?,
        demand: super::json(&row.7)?,
    })
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
    // Node IDs retain their immutable selector binding even after terminal RAM
    // retention. Tombstones grant nothing and do not consume the live quota.
    tx.execute(
        "DELETE FROM diagnostic_subscriptions WHERE principal_kind='user_session' AND cancelled=1 AND expires_at<?1",
        [before],
    )?;
    tx.execute("DELETE FROM diagnostic_tasks WHERE NOT EXISTS(SELECT 1 FROM diagnostic_subscriptions WHERE task_id=diagnostic_tasks.id)",[])?;
    Ok(())
}
