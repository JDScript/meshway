//! Observation-owned, short-lived execution dependencies. The subscription
//! rows are the only renewable authority; Node instance renewal cannot renew them.
use super::Execution;
use crate::error::Error;
use crate::observations::{
    SubscriberPrincipal,
    service::{NodeObservationAdmission, NodeObservationIdentity},
};
use crate::{observations::TaskAdmission, state::service::ReadIdentity, store::execution as store};
use chrono::{DateTime, Utc};
use meshway_configuration::execution::DiagnosticDemand;
use meshway_model::*;
use meshway_protocol::observations::{CreateObservation, ObservationPath};
use meshway_protocol::{InstanceRequest, node_observations::*};
use rusqlite::Transaction;
use std::collections::BTreeMap;

struct DiagnosticSubject {
    principal: SubscriberPrincipal,
    network_id: NetworkId,
    configuration_revision: Revision,
    observation_authorization_digest: Digest,
    expires_at: DateTime<Utc>,
    membership: Option<MembershipId>,
}

/// Private compiler evidence, not an execution permit. Only the publication
/// owner writes this alongside the safe final target in the same transaction.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum DiagnosticDecision {
    Denied,
    Unsupported { paths: Vec<Vec<NodeId>> },
    Executable,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiagnosticCompilation {
    pub configuration: ConfigurationRef,
    pub participants: BTreeMap<NodeId, meshway_configuration::execution::TrustedNode>,
    pub decision: DiagnosticDecision,
}

pub(crate) fn supports_diagnostics(
    capabilities: &BTreeMap<String, meshway_model::state::Capability>,
) -> bool {
    matches!(
        capabilities.get("feature:path-observation"),
        Some(meshway_model::state::Capability::Feature { version: 1 })
    )
}

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
    pub(super) fn diagnostic_node_identity(
        &self,
        tx: &Transaction<'_>,
        token: &str,
        instance: &InstanceRequest,
        now: DateTime<Utc>,
    ) -> Result<NodeObservationIdentity, Error> {
        let principal = self.traversal_principal(tx, token, now)?;
        let report = self.current_node_admission(tx, principal.network, principal.node, now)?;
        if report.instance_id != instance.instance_id
            || report.instance_generation != instance.generation
        {
            return Err(Error::InvalidSession);
        }
        let lease = store::lease_for_node(tx, &self.sealer, principal.network, principal.node)?;
        Ok(NodeObservationIdentity {
            principal: SubscriberPrincipal::NodeInstance {
                network: principal.network,
                node: principal.node,
                key: principal.key,
                instance: report.instance_id,
                generation: report.instance_generation,
                boot: self.boot,
            },
            expires_at: DateTime::from_timestamp_millis(lease.expires_at_ms as i64)
                .ok_or(Error::InvalidSession)?
                .min(
                    DateTime::from_timestamp_millis(principal.expires_at_ms as i64)
                        .ok_or(Error::InvalidSession)?,
                ),
        })
    }

    fn diagnostic_node_subject(
        &self,
        tx: &Transaction<'_>,
        principal: &SubscriberPrincipal,
        now: DateTime<Utc>,
    ) -> Result<DiagnosticSubject, Error> {
        let SubscriberPrincipal::NodeInstance {
            network,
            node,
            key,
            instance,
            generation,
            boot,
        } = principal
        else {
            return Err(Error::PermissionDenied);
        };
        if *boot != self.boot {
            return Err(Error::InvalidSession);
        }
        let report = self.current_node_admission(tx, *network, *node, now)?;
        if report.instance_id != *instance || report.instance_generation != *generation {
            return Err(Error::InvalidSession);
        }
        let deadline = store::admission::current_session_deadline(
            tx,
            *network,
            *node,
            self.boot,
            *key,
            now.timestamp_millis(),
        )?
        .ok_or(Error::InvalidSession)?;
        let (revision, _, _) = store::current_configuration(tx, &self.sealer, *network)?;
        let lease = store::lease_for_node(tx, &self.sealer, *network, *node)?;
        Ok(DiagnosticSubject {
            principal: principal.clone(),
            network_id: *network,
            configuration_revision: revision,
            observation_authorization_digest: super::proof::request_digest(&(
                "node-observation",
                network,
                revision,
                node,
                key,
                instance,
                generation,
                boot,
            ))?,
            expires_at: DateTime::from_timestamp_millis(lease.expires_at_ms as i64)
                .ok_or(Error::InvalidSession)?
                .min(
                    DateTime::from_timestamp_millis(deadline as i64)
                        .ok_or(Error::InvalidSession)?,
                ),
            membership: None,
        })
    }

    pub(super) fn prepare_node_diagnostic(
        &self,
        tx: &Transaction<'_>,
        token: &str,
        request: &CreateNodeObservation,
        now: DateTime<Utc>,
    ) -> Result<NodeObservationAdmission, Error> {
        let owner = self.diagnostic_node_identity(tx, token, &request.instance, now)?;
        let subject = self.diagnostic_node_subject(tx, &owner.principal, now)?;
        let digest = super::proof::request_digest(request)?;
        if let Some(matches) = store::diagnostics::subscription_matches(
            tx,
            request.subscription_id,
            &owner.principal,
            digest,
        )? {
            if !matches {
                return Err(Error::StateConflict);
            }
            let existing = self.node_diagnostic_admission(tx, request.subscription_id, now)?;
            if !owner.principal.same_owner(&existing.admission.principal) {
                return Err(Error::PermissionDenied);
            }
            return Ok(existing);
        }
        let (_, cfg, _) = store::current_configuration(tx, &self.sealer, subject.network_id)?;
        let target = match &request.target {
            NodeObservationTarget::Name { name } => {
                let name = meshway_configuration::normalize_dns_name(name)
                    .map_err(|_| Error::InvalidRequest)?;
                let suffix = format!(".{}", cfg.dns.domain);
                let hostname = name.strip_suffix(&suffix).unwrap_or(&name);
                if !meshway_configuration::valid_node_hostname(hostname) {
                    return Err(Error::InvalidRequest);
                }
                cfg.nodes
                    .iter()
                    .find(|(_, node)| node.hostname == hostname)
                    .map(|(id, _)| *id)
            }
            NodeObservationTarget::MeshIp { address } => {
                let address: std::net::IpAddr =
                    address.parse().map_err(|_| Error::InvalidRequest)?;
                cfg.nodes
                    .iter()
                    .find(|(_, node)| match address {
                        std::net::IpAddr::V4(ip) => node.addresses.ipv4 == Some(ip),
                        std::net::IpAddr::V6(ip) => node.addresses.ipv6 == Some(ip),
                    })
                    .map(|(id, _)| *id)
            }
        }
        .ok_or(Error::NotFound)?;
        let SubscriberPrincipal::NodeInstance { node: source, .. } = owner.principal else {
            return Err(Error::PermissionDenied);
        };
        if source == target {
            return Err(Error::PermissionDenied);
        }
        let resolved = CreateObservation {
            subscription_id: request.subscription_id,
            source_node_id: source,
            target_node_id: target,
            service: request.service.clone(),
            selection: request.selection.clone(),
            metrics: request.metrics.clone(),
            interval_ms: request.interval_ms,
        };
        let mut subject = subject;
        subject.expires_at = subject.expires_at.min(owner.expires_at);
        let admission =
            self.prepare_diagnostic_subject(tx, &subject, &resolved, Some(request), now)?;
        let node = cfg.nodes.get(&target).ok_or(Error::NotFound)?;
        Ok(NodeObservationAdmission {
            admission,
            request: resolved,
            target: NodeObservationTargetView {
                node_id: target,
                hostname: node.hostname.clone(),
                addresses: node.addresses.clone(),
            },
        })
    }

    pub(super) fn node_diagnostic_admission(
        &self,
        tx: &Transaction<'_>,
        subscription: ObservationSubscriptionId,
        now: DateTime<Utc>,
    ) -> Result<NodeObservationAdmission, Error> {
        let stored = store::diagnostics::node_subscription(tx, subscription)?;
        let subject = self.diagnostic_node_subject(tx, &stored.principal, now)?;
        let request = CreateObservation {
            subscription_id: subscription,
            source_node_id: stored.demand.source,
            target_node_id: stored.demand.target,
            service: stored.request.service,
            selection: stored.request.selection,
            metrics: stored.request.metrics,
            interval_ms: stored.request.interval_ms,
        };
        let admission = self.diagnostic_subject_admission(tx, &subject, &request, now)?;
        let (_, cfg, _) = store::current_configuration(tx, &self.sealer, subject.network_id)?;
        let node = cfg
            .nodes
            .get(&request.target_node_id)
            .ok_or(Error::NotFound)?;
        // The bound NodeId survives retries; the selector is never re-resolved.
        let target = NodeObservationTargetView {
            node_id: request.target_node_id,
            hostname: node.hostname.clone(),
            addresses: node.addresses.clone(),
        };
        Ok(NodeObservationAdmission {
            admission,
            request,
            target,
        })
    }

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
        let subject = DiagnosticSubject {
            principal: SubscriberPrincipal::UserSession {
                session: user.session_id,
                identity: identity.clone(),
            },
            network_id: scope.network_id,
            configuration_revision: scope.configuration_revision,
            observation_authorization_digest: scope.observation_authorization_digest,
            expires_at: user.expires_at,
            membership: Some(membership),
        };
        self.prepare_diagnostic_subject(tx, &subject, request, None, now)
    }

    fn prepare_diagnostic_subject(
        &self,
        tx: &Transaction<'_>,
        scope: &DiagnosticSubject,
        request: &CreateObservation,
        node_request: Option<&CreateNodeObservation>,
        now: DateTime<Utc>,
    ) -> Result<TaskAdmission, Error> {
        if request.metrics.is_empty()
            || request.metrics.len() > 3
            || !(1000..=30_000).contains(&request.interval_ms)
        {
            return Err(Error::InvalidRequest);
        }
        let digest = match node_request {
            Some(request) => super::proof::request_digest(request),
            None => super::proof::request_digest(request),
        }
        .map_err(node_dependency_error)?;
        let existing = store::diagnostics::subscription_matches(
            tx,
            request.subscription_id,
            &scope.principal,
            digest,
        )?;
        if let Some(matches) = existing {
            if !matches {
                return Err(Error::StateConflict);
            }
            return self.diagnostic_subject_admission(tx, scope, request, now);
        }
        let report = self
            .current_node_admission(tx, scope.network_id, request.source_node_id, now)
            .map_err(node_dependency_error)?;
        let instance_lease =
            store::lease_for_node(tx, &self.sealer, scope.network_id, request.source_node_id)
                .map_err(node_dependency_error)?;
        let key = super::proof::request_digest(&(
            scope.network_id,
            self.boot,
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
        .min(scope.expires_at)
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
            store::diagnostics::subscription_counts(tx, &scope.principal, now.timestamp_millis())?;
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
            store::diagnostics::SubscriptionInsert {
                subscription: request.subscription_id,
                task,
                principal: &scope.principal,
                membership: scope.membership,
                digest,
                expires: expires.timestamp_millis(),
                request: node_request,
            },
        )?;
        self.publish_diagnostics(tx, scope.network_id, now)?;
        self.diagnostic_subject_admission(tx, scope, request, now)
    }

    pub(super) fn diagnostic_admission(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<TaskAdmission, Error> {
        let (user, scope, _) = self.diagnostic_user(tx, identity, request, now)?;
        let subject = DiagnosticSubject {
            principal: SubscriberPrincipal::UserSession {
                session: user.session_id,
                identity: identity.clone(),
            },
            network_id: scope.network_id,
            configuration_revision: scope.configuration_revision,
            observation_authorization_digest: scope.observation_authorization_digest,
            expires_at: user.expires_at,
            membership: None,
        };
        self.diagnostic_subject_admission(tx, &subject, request, now)
    }

    fn diagnostic_subject_admission(
        &self,
        tx: &Transaction<'_>,
        scope: &DiagnosticSubject,
        request: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<TaskAdmission, Error> {
        let id = store::diagnostics::admitted_task(
            tx,
            request.subscription_id,
            &scope.principal,
            now.timestamp_millis(),
            scope.network_id,
            scope.configuration_revision,
            self.boot,
        )
        .map_err(node_dependency_error)?;
        let report = self
            .current_node_admission(tx, scope.network_id, request.source_node_id, now)
            .map_err(node_dependency_error)?;
        let (_, target) =
            store::target_for_node(tx, &self.sealer, scope.network_id, request.source_node_id)
                .map_err(node_dependency_error)?
                .ok_or(Error::StateConflict)?;
        let (demand, compilation) = store::diagnostics::compilation(tx, id)?;
        if demand.id != id
            || demand.source != request.source_node_id
            || demand.target != request.target_node_id
            || demand.service != request.service
            || demand.instance_id != report.instance_id
        {
            return Err(Error::StateConflict);
        }
        let reference = ConfigurationRef {
            revision: target.configuration.revision,
            execution_publication: target.configuration.publication,
            digest: target.digest,
        };
        if compilation.configuration != reference {
            return Err(Error::StateConflict);
        }
        store::check_diagnostic_compilation(
            tx,
            &self.sealer,
            scope.network_id,
            &compilation.participants,
        )?;
        let (paths, supported) = match compilation.decision {
            DiagnosticDecision::Denied => return Err(Error::PermissionDenied),
            DiagnosticDecision::Unsupported { paths } => (paths, false),
            DiagnosticDecision::Executable => {
                let permit = target
                    .configuration
                    .diagnostics
                    .get(&id)
                    .ok_or(Error::StateConflict)?;
                if permit.source != request.source_node_id
                    || permit.target != request.target_node_id
                    || permit.service != request.service
                    || permit.instance_id != report.instance_id
                    || permit.expires_at_ms <= now.timestamp_millis() as u64
                {
                    return Err(Error::StateConflict);
                }
                (permit.paths.iter().map(|p| p.nodes.clone()).collect(), true)
            }
        };
        if paths.is_empty()
            || paths.iter().any(|path| {
                path.first() != Some(&request.source_node_id)
                    || path.last() != Some(&request.target_node_id)
            })
        {
            return Err(Error::StateConflict);
        }
        if let ObservationPath::Explicit { path } = &request.selection
            && !paths.contains(path)
        {
            return Err(Error::PermissionDenied);
        }
        let lease =
            store::lease_for_node(tx, &self.sealer, scope.network_id, request.source_node_id)
                .map_err(node_dependency_error)?;
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
            paths,
            node_expires_at: DateTime::from_timestamp_millis(lease.expires_at_ms as i64)
                .ok_or(Error::decode("node_execution.instance_expires_at"))?,
            report,
            principal: scope.principal.clone(),
            authorization_digest: scope.observation_authorization_digest,
            authorization_expires_at: scope.expires_at,
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
    ) -> Result<bool, Error> {
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
        let changed = !networks.is_empty();
        for network in networks {
            self.publish_diagnostics(tx, network, now)?;
        }
        store::diagnostics::prune(tx, now.timestamp_millis() - 60_000)?;
        Ok(changed)
    }
}
