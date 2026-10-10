//! Observation authority: creation, renewal and reauthorization
//! decide on the source's current netmap with the source's own compiler, and
//! never persist a diagnostic demand, compile or publish.
use super::*;

/// Diagnostic decision on the source's current netmap: the
/// result of the same local compiler the source runs before probing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetmapDiagnostic {
    /// Observable candidate paths; every allowed path when none is observable.
    pub paths: Vec<Vec<NodeId>>,
    /// Whether every Node of `paths` declares path observation.
    pub observable: bool,
}

/// Denied (or the service is outside the decision) is `PermissionDenied`;
/// an allowed pair without an observable path is `observable: false`.
/// Nothing is compiled into a netmap, persisted or published.
pub fn netmap_diagnostic(
    source: &meshway_configuration::netmap::NodeNetmap,
    target: NodeId,
    service: &meshway_configuration::execution::ObservationServiceScope,
    selection: &ObservationPath,
) -> Result<NetmapDiagnostic, Error> {
    let explicit = match selection {
        ObservationPath::Auto => None,
        ObservationPath::Explicit { path } => Some(path.as_slice()),
    };
    let decided =
        meshway_policy::compile::netmap::observation_paths(source, target, service, explicit)
            .map_err(|_| Error::PermissionDenied)?;
    Ok(NetmapDiagnostic {
        paths: decided.paths,
        observable: decided.observable,
    })
}

/// The observation provider. Creation, renewal and
/// reauthorization only read the source's current netmap; there is no
/// diagnostic demand, compilation mirror or publication. A subscription's
/// renewal changes only the subscription itself.
pub struct NetmapDiagnostics {
    execution: Execution,
    netmaps: std::sync::Arc<dyn crate::state::facts::NetmapReader>,
}

impl NetmapDiagnostics {
    pub(crate) fn new(
        execution: Execution,
        netmaps: std::sync::Arc<dyn crate::state::facts::NetmapReader>,
    ) -> Self {
        Self { execution, netmaps }
    }
    fn admission(
        &self,
        tx: &Transaction<'_>,
        principal: SubscriberPrincipal,
        authorization: (Digest, DateTime<Utc>),
        request: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<TaskAdmission, Error> {
        let execution = &self.execution;
        let network = match &principal {
            SubscriberPrincipal::UserSession {
                identity: ReadIdentity::UserSession { network, .. },
                ..
            } => *network,
            SubscriberPrincipal::NodeInstance { network, .. } => *network,
            SubscriberPrincipal::UserSession { .. } => return Err(Error::PermissionDenied),
        };
        let source = self
            .netmaps
            .current(tx, network, request.source_node_id)
            .map_err(node_dependency_error)?
            .ok_or(Error::StateConflict)?;
        let decision = netmap_diagnostic(
            &source.netmap,
            request.target_node_id,
            &request.service,
            &request.selection,
        )?;
        let report = execution
            .current_node_admission(tx, network, request.source_node_id, now)
            .map_err(node_dependency_error)?;
        let netmap = source.reference();
        if report.execution != Some(netmap) {
            return Err(Error::StateConflict);
        }
        let lease = store::lease_for_node(tx, network, request.source_node_id)
            .map_err(node_dependency_error)?;
        Ok(TaskAdmission {
            observation_id: None,
            unsupported_metrics: request
                .metrics
                .iter()
                .filter(|metric| {
                    !decision.observable
                        || **metric == meshway_model::observations::MetricKind::Traffic
                })
                .copied()
                .collect(),
            netmap,
            paths: decision.paths,
            node_expires_at: DateTime::from_timestamp_millis(lease.expires_at_ms as i64)
                .ok_or(Error::decode("node_execution.instance_expires_at"))?,
            report,
            principal,
            authorization_digest: authorization.0,
            authorization_expires_at: authorization.1,
        })
    }

    fn user(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<TaskAdmission, Error> {
        let ReadIdentity::UserSession { digest, .. } = identity else {
            return Err(Error::PermissionDenied);
        };
        let user =
            crate::store::authorization::current_session(tx, digest, now.timestamp_millis())?;
        let scope = self
            .execution
            .state_scope_in_transaction(tx, identity, now)?;
        if !scope.full.contains(&request.source_node_id)
            || request.source_node_id == request.target_node_id
        {
            return Err(Error::PermissionDenied);
        }
        self.admission(
            tx,
            SubscriberPrincipal::UserSession {
                session: user.session_id,
                identity: identity.clone(),
            },
            (scope.observation_authorization_digest, user.expires_at),
            request,
            now,
        )
    }

    fn node(
        &self,
        tx: &Transaction<'_>,
        principal: &SubscriberPrincipal,
        request: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<NodeObservationAdmission, Error> {
        let subject = self.execution.diagnostic_node_subject(tx, principal, now)?;
        let SubscriberPrincipal::NodeInstance { node, .. } = principal else {
            return Err(Error::PermissionDenied);
        };
        if *node != request.source_node_id {
            return Err(Error::PermissionDenied);
        }
        let admission = self.admission(
            tx,
            principal.clone(),
            (subject.observation_authorization_digest, subject.expires_at),
            request,
            now,
        )?;
        let source = self
            .netmaps
            .current(tx, subject.network_id, *node)?
            .ok_or(Error::StateConflict)?;
        let settings = source
            .netmap
            .projection
            .configuration
            .nodes
            .get(&request.target_node_id)
            .ok_or(Error::NotFound)?;
        Ok(NodeObservationAdmission {
            admission,
            request: request.clone(),
            target: NodeObservationTargetView {
                node_id: request.target_node_id,
                hostname: settings.hostname.clone(),
                addresses: settings.addresses.clone(),
            },
        })
    }
}

/// Name or Mesh address resolution on the source's own netmap.
fn netmap_target(
    source: &meshway_configuration::netmap::NodeNetmap,
    target: &NodeObservationTarget,
) -> Result<NodeId, Error> {
    let nodes = &source.projection.configuration.nodes;
    match target {
        NodeObservationTarget::Name { name } => {
            let name = meshway_configuration::normalize_dns_name(name)
                .map_err(|_| Error::InvalidRequest)?;
            let suffix = format!(".{}", source.projection.configuration.dns.domain);
            let hostname = name.strip_suffix(&suffix).unwrap_or(&name);
            if !meshway_configuration::valid_node_hostname(hostname) {
                return Err(Error::InvalidRequest);
            }
            nodes
                .iter()
                .find(|(_, node)| node.hostname == hostname)
                .map(|(id, _)| *id)
        }
        NodeObservationTarget::MeshIp { address } => {
            let address: std::net::IpAddr = address.parse().map_err(|_| Error::InvalidRequest)?;
            nodes
                .iter()
                .find(|(_, node)| match address {
                    std::net::IpAddr::V4(ip) => node.addresses.ipv4 == Some(ip),
                    std::net::IpAddr::V6(ip) => node.addresses.ipv6 == Some(ip),
                })
                .map(|(id, _)| *id)
        }
    }
    .ok_or(Error::NotFound)
}

impl crate::observations::service::ObservationProvider for NetmapDiagnostics {
    fn event_driven_maintenance(&self) -> bool {
        true
    }
    fn node_observation_identity(
        &self,
        tx: &Transaction<'_>,
        token: &str,
        instance: &InstanceRequest,
        now: DateTime<Utc>,
    ) -> Result<NodeObservationIdentity, Error> {
        self.execution
            .diagnostic_node_identity(tx, token, instance, now)
    }
    fn prepare_node_observation(
        &self,
        tx: &Transaction<'_>,
        token: &str,
        request: &CreateNodeObservation,
        now: DateTime<Utc>,
    ) -> Result<NodeObservationAdmission, Error> {
        let owner = self
            .execution
            .diagnostic_node_identity(tx, token, &request.instance, now)?;
        let SubscriberPrincipal::NodeInstance { network, node, .. } = &owner.principal else {
            return Err(Error::PermissionDenied);
        };
        let source = self
            .netmaps
            .current(tx, *network, *node)?
            .ok_or(Error::StateConflict)?;
        let target = netmap_target(&source.netmap, &request.target)?;
        let resolved = CreateObservation {
            subscription_id: request.subscription_id,
            source_node_id: *node,
            target_node_id: target,
            service: request.service.clone(),
            selection: request.selection.clone(),
            metrics: request.metrics.clone(),
            interval_ms: request.interval_ms,
        };
        let mut admitted = self.node(tx, &owner.principal, &resolved, now)?;
        admitted.admission.authorization_expires_at = admitted
            .admission
            .authorization_expires_at
            .min(owner.expires_at);
        Ok(admitted)
    }
    fn node_observation(
        &self,
        tx: &Transaction<'_>,
        _: ObservationSubscriptionId,
        stored: &crate::observations::service::StoredSubscription,
        now: DateTime<Utc>,
    ) -> Result<NodeObservationAdmission, Error> {
        let mut current = self.node(tx, &stored.principal, &stored.request, now)?;
        current.admission.observation_id = Some(stored.observation);
        Ok(current)
    }
    fn synchronize(
        &self,
        _: &Transaction<'_>,
        _: &[crate::observations::service::SubscriptionLease],
        _: DateTime<Utc>,
    ) -> Result<bool, Error> {
        Ok(false)
    }
    fn scope(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<crate::state::VisibilityScope, Error> {
        self.execution.state_scope_in_transaction(tx, identity, now)
    }
    fn report(
        &self,
        tx: &Transaction<'_>,
        authorization: &str,
        now: DateTime<Utc>,
    ) -> Result<crate::state::ReportAdmission, Error> {
        self.execution
            .report_with_context(tx, authorization, now, false)
    }
    fn prepare_observation(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<TaskAdmission, Error> {
        if request.metrics.is_empty()
            || request.metrics.len() > 3
            || !(1000..=30_000).contains(&request.interval_ms)
        {
            return Err(Error::InvalidRequest);
        }
        self.user(tx, identity, request, now)
    }
    fn observation(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<TaskAdmission, Error> {
        self.user(tx, identity, request, now)
    }
    fn observation_session(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<SessionId, Error> {
        self.execution.observation_session(tx, identity, now)
    }
}
