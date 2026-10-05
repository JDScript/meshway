//! Transactional traversal authority, shared by endpoint disclosure and attempts.
use super::{Execution, NodePrincipal};
use crate::{error::Error, state::ReportAdmission, store::execution as store};
use chrono::{DateTime, Utc};
use meshway_configuration::execution::{Carrier, CompiledNode, NodeConfiguration};
use meshway_model::{ConfigurationRef, NetworkId, NodeId, RequestId, state::ResourceRef};
use meshway_policy::admission::{admit_carrier, admit_handshake};
use meshway_protocol::{InstanceRequest, connectivity::UdpTraversalUnavailableReason as Reason};
use meshway_transport_spec::{Listener, TransportProtocol};
use rusqlite::Transaction;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct Scope {
    pub report: ReportAdmission,
    pub target: CompiledNode,
    pub deadline_ms: u64,
}
pub(crate) struct EndpointDisclosure {
    pub resources: BTreeMap<NodeId, BTreeSet<ResourceRef>>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Witness {
    pub demand: RequestId,
    pub path: Vec<NodeId>,
}

#[derive(Debug)]
pub(crate) enum Failure {
    Unavailable(Reason),
    Error(Error),
}
impl From<Error> for Failure {
    fn from(value: Error) -> Self {
        Self::Error(value)
    }
}
impl From<Reason> for Failure {
    fn from(value: Reason) -> Self {
        Self::Unavailable(value)
    }
}
pub(crate) type Result<T> = std::result::Result<T, Failure>;

fn scope_error(error: Error) -> Failure {
    match error {
        Error::BindingConflict
        | Error::StateConflict
        | Error::InvalidCredentials
        | Error::NotFound
        | Error::PermissionDenied => Reason::ScopeChanged.into(),
        error => Failure::Error(error),
    }
}

impl Execution {
    pub(crate) fn traversal_principal(
        &self,
        tx: &Transaction<'_>,
        token: &str,
        now: DateTime<Utc>,
    ) -> std::result::Result<NodePrincipal, Error> {
        store::authorize_node(
            tx,
            &self.sealer,
            self.boot,
            super::proof::token_hash(token)?,
            now.timestamp_millis(),
        )
    }

    pub(crate) fn traversal_reauthorize(
        &self,
        tx: &Transaction<'_>,
        principal: &NodePrincipal,
        now: DateTime<Utc>,
    ) -> Result<u64> {
        store::reauthorize(
            tx,
            &self.sealer,
            self.boot,
            principal,
            now.timestamp_millis(),
        )
        .map(|principal| principal.expires_at_ms())
        .map_err(scope_error)
    }

    pub(crate) fn traversal_scope(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
        now: DateTime<Utc>,
    ) -> Result<Scope> {
        let report = self
            .current_node_admission(tx, network, node, now)
            .map_err(scope_error)?;
        let reference = report.execution.as_ref().ok_or(Reason::ScopeChanged)?;
        let (_, target) = store::target_for_node(tx, &self.sealer, network, node)
            .map_err(scope_error)?
            .ok_or(Reason::ScopeChanged)?;
        if !target.configuration.enabled
            || target.digest != reference.digest
            || target.configuration.publication != reference.execution_publication
            || target.configuration.revision != reference.revision
        {
            return Err(Reason::ScopeChanged.into());
        }
        // This is a current-key session deadline, not a fabricated peer bearer.
        // Each receiver must subsequently accept using its own live credential.
        let session_expiry = store::admission::current_session_deadline(
            tx,
            network,
            node,
            self.boot,
            target.configuration.self_keys.control_key.id,
            now.timestamp_millis(),
        )?;
        let lease_expiry = u64::try_from(report.lease_expires_at.timestamp_millis())
            .map_err(|_| Error::decode("traversal deadline"))?;
        let deadline_ms = session_expiry
            .ok_or(Reason::ScopeChanged)?
            .min(lease_expiry);
        Ok(Scope {
            report,
            target,
            deadline_ms,
        })
    }

    pub(crate) fn traversal_caller(
        &self,
        tx: &Transaction<'_>,
        principal: &NodePrincipal,
        instance: &InstanceRequest,
        configuration: &ConfigurationRef,
        now: DateTime<Utc>,
    ) -> Result<Scope> {
        let mut scope = self.traversal_scope(tx, principal.network, principal.node, now)?;
        if scope.report.instance_id != instance.instance_id
            || scope.report.instance_generation != instance.generation
            || scope.report.execution.as_ref() != Some(configuration)
            || scope.target.configuration.self_keys.control_key.id != principal.key
        {
            return Err(Reason::ScopeChanged.into());
        }
        scope.deadline_ms = scope.deadline_ms.min(principal.expires_at_ms);
        Ok(scope)
    }

    pub(crate) fn endpoint_disclosure(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        local: NodeId,
        visible: &BTreeSet<NodeId>,
        now: DateTime<Utc>,
    ) -> std::result::Result<EndpointDisclosure, Error> {
        let mut result = BTreeMap::new();
        let mut expires_at = None;
        let local = match self.traversal_scope(tx, network, local, now) {
            Ok(scope) => scope,
            Err(Failure::Unavailable(_)) => {
                return Ok(EndpointDisclosure {
                    resources: result,
                    expires_at,
                });
            }
            Err(Failure::Error(error)) => return Err(error),
        };
        for peer in visible.iter().filter(|node| **node != local.report.node_id) {
            let peer = match self.traversal_scope(tx, network, *peer, now) {
                Ok(scope) => scope,
                Err(Failure::Unavailable(_)) => continue,
                Err(Failure::Error(error)) => return Err(error),
            };
            // Preserve static stream carriers too; only traversal operations are UDP-only.
            for (dialer, listener) in [(&local, &peer), (&peer, &local)] {
                for dial in dialer
                    .target
                    .configuration
                    .dialers
                    .values()
                    .filter(|dial| dial.peer == listener.report.node_id)
                {
                    let carrier = Carrier {
                        dialer: dialer.report.node_id,
                        listener: listener.report.node_id,
                        listener_key: dial.listener_key.clone(),
                        protocol: dial.parameters.transport(),
                    };
                    if let Some((_, deadline)) = common_witness(
                        &local.target.configuration,
                        &peer.target.configuration,
                        &carrier,
                        now.timestamp_millis() as u64,
                        None,
                    ) {
                        let deadline = deadline.min(local.deadline_ms).min(peer.deadline_ms);
                        let deadline = i64::try_from(deadline)
                            .ok()
                            .and_then(DateTime::from_timestamp_millis)
                            .ok_or(Error::decode("endpoint disclosure deadline"))?;
                        expires_at = Some(
                            expires_at.map_or(deadline, |old: DateTime<Utc>| old.min(deadline)),
                        );
                        result
                            .entry(peer.report.node_id)
                            .or_insert_with(BTreeSet::new)
                            .extend(carrier_resources(&peer.target.configuration, &carrier));
                    }
                }
            }
        }
        Ok(EndpointDisclosure {
            resources: result,
            expires_at,
        })
    }
}

/// One exact, live business witness in both installed publications. A diagnostic
/// permit, topology edge or two unrelated one-sided permits cannot substitute.
pub(crate) fn common_witness(
    a: &NodeConfiguration,
    b: &NodeConfiguration,
    carrier: &Carrier,
    now: u64,
    selected: Option<&Witness>,
) -> Option<(Witness, u64)> {
    if a.network_id != b.network_id
        || a.node_id == b.node_id
        || a.peers.get(&b.node_id).is_none_or(|peer| {
            peer.control_key != b.self_keys.control_key
                || peer.dataplane_key != b.self_keys.dataplane_key
        })
        || b.peers.get(&a.node_id).is_none_or(|peer| {
            peer.control_key != a.self_keys.control_key
                || peer.dataplane_key != a.self_keys.dataplane_key
        })
    {
        return None;
    }
    for (id, left) in &a.permits {
        if selected.is_some_and(|witness| witness.demand != *id) {
            continue;
        }
        let Some(right) = b.permits.get(id) else {
            continue;
        };
        if left.source != right.source
            || left.target != right.target
            || left.protocol != right.protocol
            || left.port != right.port
            || left.profile != right.profile
        {
            continue;
        }
        let deadline = left.expires_at_ms.min(right.expires_at_ms);
        if deadline <= now {
            continue;
        }
        for path in &left.paths {
            if selected.is_some_and(|witness| witness.path != path.nodes) {
                continue;
            }
            let admitted = [(a, b.node_id), (b, a.node_id)]
                .into_iter()
                .all(|(cfg, peer)| {
                    let Some(position) = path.nodes.iter().position(|node| *node == cfg.node_id)
                    else {
                        return false;
                    };
                    let previous = position.checked_sub(1).map(|i| path.nodes[i]);
                    admit_handshake(cfg, *id, &path.nodes, left.source, previous, false, now)
                        .is_ok()
                        && admit_carrier(cfg, *id, &path.nodes, peer, carrier).is_ok()
                });
            if admitted {
                return Some((
                    Witness {
                        demand: *id,
                        path: path.nodes.clone(),
                    },
                    deadline,
                ));
            }
        }
    }
    None
}

pub(crate) fn carrier_resources(
    cfg: &NodeConfiguration,
    carrier: &Carrier,
) -> BTreeSet<ResourceRef> {
    let mut resources = BTreeSet::new();
    if cfg.node_id == carrier.listener {
        resources.insert(ResourceRef::Listener {
            key: carrier.listener_key.clone(),
        });
    } else if cfg.node_id == carrier.dialer {
        for (key, dial) in &cfg.dialers {
            if dial.peer == carrier.listener
                && dial.listener_key == carrier.listener_key
                && dial.parameters.transport() == carrier.protocol
            {
                resources.insert(ResourceRef::Dial { key: key.clone() });
            }
        }
        if carrier.protocol == TransportProtocol::NativeUdp {
            for (key, listener) in &cfg.listeners {
                if matches!(listener.declaration(), Listener::NativeUdp(value) if value.enabled) {
                    resources.insert(ResourceRef::Listener { key: key.clone() });
                }
            }
        }
    }
    resources
}
