//! Transactional traversal authority on the current netmaps of both ends:
//! endpoint disclosure follows dial candidates, coordination the two sides'
//! adjacency admission. Business flows never take part.
use super::{Execution, NodePrincipal};
use crate::{error::Error, state::ReportAdmission, store::execution as store};
use chrono::{DateTime, Utc};
use meshway_configuration::{
    execution::Carrier,
    netmap::{CompiledNetmap, NodeNetmap},
};
use meshway_model::{NetmapRef, NetworkId, NodeId, state::ResourceRef};
use meshway_protocol::{InstanceRequest, connectivity::UdpTraversalUnavailableReason as Reason};
use meshway_transport_spec::{Listener, TransportProtocol};
use rusqlite::Transaction;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) struct Scope {
    pub report: ReportAdmission,
    pub target: Arc<CompiledNetmap>,
    pub deadline_ms: u64,
}

/// Read memoization for one SQLite transaction and one clock snapshot only.
/// Each Node still undergoes the full current-key/instance/netmap check once.
pub(crate) struct ScopeRead<'a, 'connection> {
    execution: &'a Execution,
    tx: &'a Transaction<'connection>,
    now: DateTime<Utc>,
    scopes: BTreeMap<(NetworkId, NodeId), Result<Arc<Scope>>>,
}
impl<'a, 'connection> ScopeRead<'a, 'connection> {
    pub(crate) fn new(
        execution: &'a Execution,
        tx: &'a Transaction<'connection>,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            execution,
            tx,
            now,
            scopes: BTreeMap::new(),
        }
    }
    pub(crate) fn scope(&mut self, network: NetworkId, node: NodeId) -> Result<Arc<Scope>> {
        self.scopes
            .entry((network, node))
            .or_insert_with(|| {
                self.execution
                    .traversal_scope(self.tx, network, node, self.now)
                    .map(Arc::new)
            })
            .clone()
    }
    pub(crate) fn now(&self) -> DateTime<Utc> {
        self.now
    }
    pub(crate) fn reauthorize(&self, principal: &NodePrincipal) -> Result<u64> {
        self.execution
            .traversal_reauthorize(self.tx, principal, self.now)
    }
    pub(crate) fn caller(
        &mut self,
        principal: &NodePrincipal,
        instance: &InstanceRequest,
        netmap: &NetmapRef,
    ) -> Result<Arc<Scope>> {
        let scope = self.scope(principal.network, principal.node)?;
        if scope.report.instance_id != instance.instance_id
            || scope.report.instance_generation != instance.generation
            || scope.report.execution.as_ref() != Some(netmap)
            || scope.target.netmap.self_.self_keys.control_key.id != principal.key
        {
            return Err(Reason::ScopeChanged.into());
        }
        Ok(scope)
    }
}

#[derive(Clone, Debug)]
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
        let (report, target) = self
            .current_node_context(tx, network, node, now)
            .map_err(scope_error)?;
        let reference = report.execution.ok_or(Reason::ScopeChanged)?;
        let target = target.ok_or(Reason::ScopeChanged)?;
        if !target.netmap.self_.enabled || target.reference() != reference {
            return Err(Reason::ScopeChanged.into());
        }
        // This is a current-key session deadline, not a fabricated peer bearer.
        // Each receiver must subsequently accept using its own live credential.
        let session_expiry = store::admission::current_session_deadline(
            tx,
            network,
            node,
            self.boot,
            target.netmap.self_.self_keys.control_key.id,
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
}

/// UDP coordination authority between the carrier's two ends.
/// Both current netmaps must list each other with the same keys, and both must
/// allow this Native UDP carrier: the dialer holds it as a dial candidate and
/// the listener's own projection admits the dialer on that listener. Business
/// flows and complete paths play no part; the candidate CAS, the
/// two-sided acceptance and short attempt deadlines still apply to the attempt.
pub fn netmap_udp_coordination(
    a: &std::sync::Arc<NodeNetmap>,
    b: &std::sync::Arc<NodeNetmap>,
    carrier: &Carrier,
) -> std::result::Result<bool, Error> {
    if a.network_id != b.network_id
        || a.node_id == b.node_id
        || carrier.protocol != TransportProtocol::NativeUdp
        || !a.self_.enabled
        || !b.self_.enabled
    {
        return Ok(false);
    }
    let (dialer, listener) = match (carrier.dialer, carrier.listener) {
        (d, l) if d == a.node_id && l == b.node_id => (a, b),
        (d, l) if d == b.node_id && l == a.node_id => (b, a),
        _ => return Ok(false),
    };
    let bound = |viewer: &NodeNetmap, peer: &NodeNetmap| {
        viewer
            .projection
            .configuration
            .nodes
            .contains_key(&peer.node_id)
            && viewer
                .projection
                .inventory
                .get(&peer.node_id)
                .is_some_and(|keys| {
                    keys.control_key == peer.self_.self_keys.control_key
                        && keys.dataplane_key == peer.self_.self_keys.dataplane_key
                })
    };
    if !bound(a, b) || !bound(b, a) {
        return Ok(false);
    }
    let dialable = dialer.dialers.values().any(|dial| {
        dial.peer == listener.node_id
            && dial.listener_key == carrier.listener_key
            && dial.parameters.transport() == TransportProtocol::NativeUdp
    });
    let listening = matches!(
        listener.self_.listeners.get(&carrier.listener_key).map(|l| l.declaration()),
        Some(Listener::NativeUdp(declaration)) if declaration.enabled
    );
    if !dialable || !listening {
        return Ok(false);
    }
    let admitted = meshway_policy::compile::netmap::LocalCompiler::new(listener)
        .carriers(dialer.node_id, listener.node_id)
        .map_err(|_| Error::Internal("netmap carrier evaluation"))?;
    Ok(admitted.contains(&carrier.listener_key))
}

/// The resources of `netmap`'s own Node that a carrier may use.
pub(crate) fn carrier_resources(netmap: &NodeNetmap, carrier: &Carrier) -> BTreeSet<ResourceRef> {
    let mut resources = BTreeSet::new();
    if netmap.node_id == carrier.listener {
        resources.insert(ResourceRef::Listener {
            key: carrier.listener_key.clone(),
        });
    } else if netmap.node_id == carrier.dialer {
        for (key, dial) in &netmap.dialers {
            if dial.peer == carrier.listener
                && dial.listener_key == carrier.listener_key
                && dial.parameters.transport() == carrier.protocol
            {
                resources.insert(ResourceRef::Dial { key: key.clone() });
            }
        }
        if carrier.protocol == TransportProtocol::NativeUdp {
            for (key, listener) in &netmap.self_.listeners {
                if matches!(listener.declaration(), Listener::NativeUdp(value) if value.enabled) {
                    resources.insert(ResourceRef::Listener { key: key.clone() });
                }
            }
        }
    }
    resources
}
