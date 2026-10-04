//! Observation-owned, short-lived execution dependencies. The subscription
//! rows are the only renewable authority; Node instance renewal cannot renew them.
use super::Execution;
use crate::error::Error;
use crate::{observations::TaskAdmission, state::service::ReadIdentity, store::execution as store};
use chrono::{DateTime, Utc};
use meshway_configuration::execution::DiagnosticDemand;
use meshway_model::*;
use meshway_protocol::observations::{CreateObservation, ObservationPath};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::BTreeMap;

/// Node-store lookups made on behalf of an authenticated user. A Node credential
/// or dependency failure must never read as that user's session being invalid.
fn node_dependency_error(error: Error) -> Error {
    match error {
        Error::InvalidCredentials | Error::InvalidSession => Error::PermissionDenied,
        other => other,
    }
}

/// Used by the normal publication compiler. No business-demand rows are added,
/// and no ordinary packet permit is emitted for these inputs.
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
        let mut demand: DiagnosticDemand = store::json(&payload)?;
        demand.expires_at_ms = expiry as u64;
        Ok((demand.id, demand))
    })
    .collect()
}

impl Execution {
    fn diagnostic_user(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<
        (
            crate::accounts::AuthenticatedSession,
            crate::state::VisibilityScope,
            Vec<u8>,
        ),
        Error,
    > {
        let ReadIdentity::UserSession { digest, network } = identity else {
            return Err(Error::PermissionDenied);
        };
        let user =
            crate::store::authorization::current_session(tx, digest, now.timestamp_millis())?;
        let scope = self.state_scope_in_transaction(tx, identity, now)?;
        if !scope.full.contains(&request.source_node_id)
            || request.source_node_id == request.target_node_id
        {
            return Err(Error::PermissionDenied);
        }
        let membership = tx.query_row(
            "SELECT id FROM memberships WHERE network_id=?1 AND user_id=?2 AND revoked_at IS NULL",
            params![
                network.as_uuid().as_bytes().as_slice(),
                user.user.id.as_uuid().as_bytes().as_slice()
            ],
            |r| r.get::<_, Vec<u8>>(0),
        )?;
        Ok((user, scope, membership))
    }

    pub(super) fn prepare_diagnostic(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<TaskAdmission, Error> {
        let (user, scope, membership) = self.diagnostic_user(tx, identity, request, now)?;
        if request.metrics.is_empty()
            || request.metrics.len() > 3
            || !(1000..=30_000).contains(&request.interval_ms)
        {
            return Err(Error::InvalidRequest);
        }
        let digest = super::proof::request_digest(request).map_err(node_dependency_error)?;
        let existing: Option<(Vec<u8>, Vec<u8>)> = tx
            .query_row(
                "SELECT session_id,request_digest FROM diagnostic_subscriptions WHERE id=?1",
                [request.subscription_id.as_uuid().as_bytes().as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((session, previous)) = existing {
            if session != user.session_id.as_uuid().as_bytes() || previous != digest.as_bytes() {
                return Err(Error::StateConflict);
            }
            return self.diagnostic_admission(tx, identity, request, now);
        }
        let report = self
            .node_admission(tx, scope.network_id, request.source_node_id, now)
            .map_err(node_dependency_error)?;
        let instance_lease =
            store::lease_for_node(tx, &self.sealer, scope.network_id, request.source_node_id)
                .map_err(node_dependency_error)?;
        let key = super::proof::request_digest(&(
            scope.network_id,
            scope.configuration_revision,
            request.source_node_id,
            report.instance_id,
            request.target_node_id,
            &request.service,
            &request.selection,
            &request.metrics,
            request.interval_ms,
        ))
        .map_err(node_dependency_error)?;
        let existing:Option<Vec<u8>>=tx.query_row("SELECT t.id FROM diagnostic_tasks t WHERE t.network_id=?1 AND t.reuse_key=?2 AND EXISTS(SELECT 1 FROM diagnostic_subscriptions s WHERE s.task_id=t.id AND s.cancelled=0 AND s.expires_at>?3) LIMIT 1",params![scope.network_id.as_uuid().as_bytes().as_slice(),key.as_bytes().as_slice(),now.timestamp_millis()],|r|r.get(0)).optional()?;
        let new_task = existing.is_none();
        let task = match existing {
            Some(id) => store::id(id).map_err(node_dependency_error)?,
            None => ObservationId::new(store::uuid().map_err(node_dependency_error)?)
                .map_err(|_| Error::Internal("generated observation ID"))?,
        };
        let expires = (now
            + chrono::Duration::seconds(crate::observations::OBSERVATION_LEASE_SECONDS))
        .min(user.expires_at)
        .min(
            DateTime::from_timestamp_millis(instance_lease.expires_at_ms as i64)
                .ok_or(Error::decode("node_execution.instance_expires_at"))?,
        );
        let demand = DiagnosticDemand {
            id: task,
            source: request.source_node_id,
            instance_id: report.instance_id,
            target: request.target_node_id,
            service: request.service.clone(),
            path: match &request.selection {
                ObservationPath::Auto => None,
                ObservationPath::Explicit { path } => Some(path.clone()),
            },
            expires_at_ms: expires.timestamp_millis() as u64,
        };
        let count:i64=tx.query_row("SELECT count(*) FROM diagnostic_subscriptions WHERE session_id=?1 AND cancelled=0 AND expires_at>?2",params![user.session_id.as_uuid().as_bytes().as_slice(),now.timestamp_millis()],|r|r.get(0))?;
        let total: i64 =
            tx.query_row("SELECT count(*) FROM diagnostic_subscriptions", [], |r| {
                r.get(0)
            })?;
        if count >= crate::observations::MAX_TASKS_PER_SESSION as i64
            || total >= crate::observations::MAX_SUBSCRIPTIONS as i64
        {
            return Err(Error::ObservationLimit);
        }
        if new_task {
            tx.execute("INSERT INTO diagnostic_tasks(id,network_id,node_id,instance_id,boot_id,revision,reuse_key,payload) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![task.as_uuid().as_bytes().as_slice(),scope.network_id.as_uuid().as_bytes().as_slice(),request.source_node_id.as_uuid().as_bytes().as_slice(),report.instance_id.as_uuid().as_bytes().as_slice(),self.boot.as_uuid().as_bytes().as_slice(),scope.configuration_revision.get() as i64,key.as_bytes().as_slice(),store::bytes(&demand).map_err(node_dependency_error)?])?;
        }
        tx.execute("INSERT INTO diagnostic_subscriptions(id,task_id,session_id,membership_id,request_digest,expires_at) VALUES(?1,?2,?3,?4,?5,?6)",params![request.subscription_id.as_uuid().as_bytes().as_slice(),task.as_uuid().as_bytes().as_slice(),user.session_id.as_uuid().as_bytes().as_slice(),membership,digest.as_bytes().as_slice(),expires.timestamp_millis()])?;
        tx.execute(
            "UPDATE diagnostic_tasks SET dirty=1 WHERE id=?1",
            [task.as_uuid().as_bytes().as_slice()],
        )?;
        self.publish_diagnostics(tx, scope.network_id, now)?;
        self.diagnostic_admission(tx, identity, request, now)
    }

    pub(super) fn diagnostic_admission(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<TaskAdmission, Error> {
        let (user, scope, _) = self.diagnostic_user(tx, identity, request, now)?;
        let id:Vec<u8>=tx.query_row("SELECT t.id FROM diagnostic_subscriptions s JOIN diagnostic_tasks t ON t.id=s.task_id WHERE s.id=?1 AND s.session_id=?2 AND s.cancelled=0 AND s.expires_at>?3 AND t.network_id=?4 AND t.revision=?5 AND t.boot_id=?6",params![request.subscription_id.as_uuid().as_bytes().as_slice(),user.session_id.as_uuid().as_bytes().as_slice(),now.timestamp_millis(),scope.network_id.as_uuid().as_bytes().as_slice(),scope.configuration_revision.get() as i64,self.boot.as_uuid().as_bytes().as_slice()],|r|r.get(0)).optional()?.ok_or(Error::NotFound)?;
        let id: ObservationId = store::id(id).map_err(node_dependency_error)?;
        let report = self
            .node_admission(tx, scope.network_id, request.source_node_id, now)
            .map_err(node_dependency_error)?;
        let (_, target) =
            store::target_for_node(tx, &self.sealer, scope.network_id, request.source_node_id)
                .map_err(node_dependency_error)?
                .ok_or(Error::StateConflict)?;
        let permit = target
            .configuration
            .diagnostics
            .get(&id)
            .ok_or(Error::PermissionDenied)?;
        if let ObservationPath::Explicit { path } = &request.selection
            && !permit.paths.iter().any(|p| &p.nodes == path)
        {
            return Err(Error::PermissionDenied);
        }
        if permit.source != request.source_node_id
            || permit.target != request.target_node_id
            || permit.service != request.service
            || permit.instance_id != report.instance_id
            || permit.expires_at_ms <= now.timestamp_millis() as u64
        {
            return Err(Error::PermissionDenied);
        }
        let reference = ConfigurationRef {
            revision: target.configuration.revision,
            execution_publication: target.configuration.publication,
            digest: target.digest,
        };
        let lease =
            store::lease_for_node(tx, &self.sealer, scope.network_id, request.source_node_id)
                .map_err(node_dependency_error)?;
        let supported = matches!(
            target
                .configuration
                .self_keys
                .capabilities
                .get("feature:path-observation"),
            Some(meshway_model::state::Capability::Feature { version: 1 })
        );
        let unsupported_metrics = request
            .metrics
            .iter()
            .filter(|metric| {
                !supported || **metric == meshway_model::observations::MetricKind::Traffic
            })
            .copied()
            .collect();
        Ok(TaskAdmission {
            observation_id: Some(id),
            unsupported_metrics,
            configuration: reference,
            paths: permit.paths.iter().map(|p| p.nodes.clone()).collect(),
            node_expires_at: DateTime::from_timestamp_millis(lease.expires_at_ms as i64)
                .ok_or(Error::decode("node_execution.instance_expires_at"))?,
            report,
            session_id: user.session_id,
            authorization_digest: scope.observation_authorization_digest,
            session_expires_at: user.expires_at,
        })
    }

    fn publish_diagnostics(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        now: DateTime<Utc>,
    ) -> Result<(), Error> {
        store::publish(tx, &self.sealer, network, "refresh", now.timestamp_millis())
            .map_err(node_dependency_error)?;
        tx.execute(
            "UPDATE diagnostic_tasks SET dirty=0 WHERE network_id=?1",
            [network.as_uuid().as_bytes().as_slice()],
        )?;
        Ok(())
    }

    pub(super) fn synchronize_diagnostics(
        &self,
        tx: &Transaction<'_>,
        subscriptions: &[crate::observations::service::SubscriptionLease],
        now: DateTime<Utc>,
    ) -> Result<(), Error> {
        let mut statement =
            tx.prepare("SELECT id,expires_at FROM diagnostic_subscriptions WHERE cancelled=0")?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        for (id, old_expiry) in rows {
            let id: ObservationSubscriptionId = store::id(id).map_err(node_dependency_error)?;
            let lease = subscriptions
                .iter()
                .find(|s| s.id == id && s.expires_at > now);
            match lease {
                Some(lease) if lease.expires_at.timestamp_millis() != old_expiry => {
                    tx.execute("UPDATE diagnostic_subscriptions SET expires_at=?2 WHERE id=?1 AND cancelled=0",params![id.as_uuid().as_bytes().as_slice(),lease.expires_at.timestamp_millis()])?;
                }
                None => {
                    tx.execute("UPDATE diagnostic_subscriptions SET cancelled=1 WHERE id=?1 AND cancelled=0",[id.as_uuid().as_bytes().as_slice()])?;
                }
                _ => {}
            }
        }
        let mut stmt =
            tx.prepare("SELECT DISTINCT network_id FROM diagnostic_tasks WHERE dirty=1")?;
        let networks = stmt
            .query_map([], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        for network in networks {
            self.publish_diagnostics(tx, store::id(network).map_err(node_dependency_error)?, now)?;
        }
        tx.execute(
            "DELETE FROM diagnostic_subscriptions WHERE cancelled=1 AND expires_at<?1",
            [now.timestamp_millis() - 60_000],
        )?;
        tx.execute("DELETE FROM diagnostic_tasks WHERE NOT EXISTS(SELECT 1 FROM diagnostic_subscriptions WHERE task_id=diagnostic_tasks.id)",[])?;
        Ok(())
    }
}
