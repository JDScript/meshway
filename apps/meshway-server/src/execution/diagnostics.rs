//! Observation-owned, short-lived execution dependencies. The subscription
//! rows are the only renewable authority; Node instance renewal cannot renew them.
use super::Execution;
use crate::error::Error;
use crate::{observations::TaskAdmission, state::service::ReadIdentity, store::execution as store};
use chrono::{DateTime, Utc};
use meshway_configuration::execution::DiagnosticDemand;
use meshway_model::*;
use meshway_protocol::observations::{CreateObservation, ObservationPath};
use rusqlite::Transaction;
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
    store::diagnostics::collect(tx, network, now)
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
            MembershipId,
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
        let membership = store::diagnostics::membership(tx, *network, user.user.id)?;
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
        let existing = store::diagnostics::subscription_matches(
            tx,
            request.subscription_id,
            user.session_id,
            digest,
        )?;
        if let Some(matches) = existing {
            if !matches {
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
        let existing =
            store::diagnostics::reusable_task(tx, scope.network_id, key, now.timestamp_millis())
                .map_err(node_dependency_error)?;
        let new_task = existing.is_none();
        let task = match existing {
            Some(id) => id,
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
        let (count, total) =
            store::diagnostics::subscription_counts(tx, user.session_id, now.timestamp_millis())?;
        if count >= crate::observations::MAX_TASKS_PER_SESSION as i64
            || total >= crate::observations::MAX_SUBSCRIPTIONS as i64
        {
            return Err(Error::ObservationLimit);
        }
        if new_task {
            store::diagnostics::insert_task(
                tx,
                scope.network_id,
                self.boot,
                scope.configuration_revision,
                key,
                &demand,
            )
            .map_err(node_dependency_error)?;
        }
        store::diagnostics::insert_subscription(
            tx,
            request.subscription_id,
            task,
            user.session_id,
            membership,
            digest,
            expires.timestamp_millis(),
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
        let id = store::diagnostics::admitted_task(
            tx,
            request.subscription_id,
            user.session_id,
            now.timestamp_millis(),
            scope.network_id,
            scope.configuration_revision,
            self.boot,
        )
        .map_err(node_dependency_error)?;
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
        store::diagnostics::mark_published(tx, network)?;
        Ok(())
    }

    pub(super) fn synchronize_diagnostics(
        &self,
        tx: &Transaction<'_>,
        subscriptions: &[crate::observations::service::SubscriptionLease],
        now: DateTime<Utc>,
    ) -> Result<(), Error> {
        let rows = store::diagnostics::active_subscriptions(tx).map_err(node_dependency_error)?;
        for (id, old_expiry) in rows {
            let lease = subscriptions
                .iter()
                .find(|s| s.id == id && s.expires_at > now);
            match lease {
                Some(lease) if lease.expires_at.timestamp_millis() != old_expiry => {
                    store::diagnostics::update_subscription_expiry(
                        tx,
                        id,
                        lease.expires_at.timestamp_millis(),
                    )?;
                }
                None => {
                    store::diagnostics::cancel_subscription(tx, id)?;
                }
                _ => {}
            }
        }
        let networks = store::diagnostics::dirty_networks(tx).map_err(node_dependency_error)?;
        for network in networks {
            self.publish_diagnostics(tx, network, now)?;
        }
        store::diagnostics::prune(tx, now.timestamp_millis() - 60_000)?;
        Ok(())
    }
}
