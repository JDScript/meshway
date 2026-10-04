//! Production B→C authorization projection. The root StateProvider composes
//! these transaction-checked facts with A's configuration/entity projections.
use super::{Execution, NodeError};
use crate::{
    state::{
        ConfigurationAdmission, CurrentNode, ReportAdmission, ResourceAdmission, StateError,
        VisibilityScope, service::ReadIdentity,
    },
    store::{authorization::current_session, execution as store},
};
use chrono::{DateTime, Utc};
use meshway_configuration::{
    NodeAddresses,
    execution::{CompiledNode, DialParameters},
};
use meshway_model::state::{ResourceRef, SocketNetwork};
use meshway_model::*;
use meshway_transport_spec::{Endpoint, Listener};
use rusqlite::{OptionalExtension, Transaction, params};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
};

impl Execution {
    /// Reauthorize an accepted diagnostic subscription against its current
    /// exact service permit. Only prepare_observation may create a lifetime.
    pub fn observation_admission_in_transaction(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &meshway_protocol::observations::CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<crate::observations::TaskAdmission, StateError> {
        self.diagnostic_admission(tx, identity, request, now)
    }
    /// Authenticate the real Node session and current instance in the same SQL
    /// transaction C uses to accept a report. No synthetic principal or bearer.
    pub fn report_admission_in_transaction(
        &self,
        tx: &Transaction<'_>,
        token: &str,
        now: DateTime<Utc>,
    ) -> Result<ReportAdmission, StateError> {
        let principal = store::authorize_node(
            tx,
            &self.sealer,
            self.boot,
            super::proof::token_hash(token).map_err(state_error)?,
            now.timestamp_millis(),
        )
        .map_err(state_error)?;
        let mut report = self.node_admission(tx, principal.network, principal.node, now)?;
        report.lease_expires_at = report.lease_expires_at.min(time(principal.expires_at_ms)?);
        Ok(report)
    }

    /// Visibility is a server projection of current Config identity and compiled
    /// execution dependencies. Only self/admin gets full local socket details.
    pub fn state_scope_in_transaction(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<VisibilityScope, StateError> {
        let (network, full, expires_at, actor) = match identity {
            ReadIdentity::Node { authorization } => {
                let report = self.report_admission_in_transaction(tx, authorization, now)?;
                (
                    report.network_id,
                    BTreeSet::from([report.node_id]),
                    report.lease_expires_at,
                    super::proof::request_digest(&("node", report.node_id)).map_err(state_error)?,
                )
            }
            ReadIdentity::UserSession { digest, network } => {
                let session = current_session(tx, digest, now.timestamp_millis())
                    .map_err(|_| StateError::Unauthorized)?;
                let (membership, role): (Vec<u8>, String) = tx.query_row(
                    "SELECT m.id,m.role FROM memberships m JOIN networks n ON n.id=m.network_id WHERE m.network_id=?1 AND m.user_id=?2 AND m.revoked_at IS NULL AND n.deleted_at IS NULL",
                    params![network.as_uuid().as_bytes().as_slice(), session.user.id.as_uuid().as_bytes().as_slice()],
                    |r| Ok((r.get(0)?, r.get(1)?))).optional()?.ok_or(StateError::NotFound)?;
                let (_, cfg, bindings) = store::current_configuration(tx, &self.sealer, *network)
                    .map_err(state_error)?;
                let manager = matches!(role.as_str(), "owner" | "admin");
                let full = cfg
                    .nodes
                    .keys()
                    .filter(|node| {
                        manager
                            || store::user_binding(&cfg, &bindings, **node)
                                .is_some_and(|binding| binding.user_id == session.user.id)
                    })
                    .copied()
                    .collect();
                (
                    *network,
                    full,
                    session.expires_at,
                    super::proof::request_digest(&("user", session.user.id, membership, role))
                        .map_err(state_error)?,
                )
            }
        };
        let (revision, cfg, _) =
            store::current_configuration(tx, &self.sealer, network).map_err(state_error)?;
        let mut visible = full.clone();
        let mut observation_paths = BTreeSet::new();
        for node in &full {
            if let Some((publication, target)) =
                store::target_for_node(tx, &self.sealer, network, *node).map_err(state_error)?
            {
                match store::check_publication(
                    tx,
                    &self.sealer,
                    network,
                    *node,
                    publication,
                    now.timestamp_millis(),
                ) {
                    Ok(()) => {}
                    Err(NodeError::Conflict) => continue,
                    Err(error) => return Err(state_error(error)),
                }
                for permit in target
                    .configuration
                    .permits
                    .values()
                    .filter(|p| p.expires_at_ms > now.timestamp_millis() as u64)
                {
                    for path in &permit.paths {
                        visible.extend(path.nodes.iter().copied());
                        observation_paths.insert(path.nodes.clone());
                    }
                }
                for permit in target
                    .configuration
                    .diagnostics
                    .values()
                    .filter(|p| p.expires_at_ms > now.timestamp_millis() as u64)
                {
                    for path in &permit.paths {
                        visible.extend(path.nodes.iter().copied());
                        observation_paths.insert(path.nodes.clone());
                    }
                }
            }
        }
        visible.retain(|node| cfg.nodes.contains_key(node));
        let mut current = BTreeMap::new();
        let mut bindings = BTreeMap::new();
        for node in &visible {
            let revoked: bool = tx.query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM nodes n JOIN node_keys c ON c.network_id=n.network_id AND c.node_id=n.id AND c.purpose='control' AND c.retired_at IS NULL AND c.revoked_at IS NULL JOIN node_keys d ON d.network_id=n.network_id AND d.node_id=n.id AND d.purpose='dataplane' AND d.retired_at IS NULL AND d.revoked_at IS NULL WHERE n.network_id=?1 AND n.id=?2 AND n.credentials_revoked_at IS NULL)",
                params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice()], |r| r.get(0))?;
            let live: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM node_execution WHERE network_id=?1 AND node_id=?2 AND boot_id=?3 AND instance_expires_at>?4)",
                params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice(), self.boot.as_uuid().as_bytes().as_slice(), now.timestamp_millis()], |r| r.get(0))?;
            let lease = if live && !revoked {
                Some(store::lease_for_node(tx, &self.sealer, network, *node).map_err(state_error)?)
            } else {
                None
            };
            let configurations: Vec<_> = lease
                .as_ref()
                .and_then(|l| l.target.clone())
                .into_iter()
                .collect();
            let execution = match &lease {
                Some(lease)
                    if lease
                        .execution_expires_at_ms
                        .is_some_and(|expiry| expiry > now.timestamp_millis() as u64) =>
                {
                    if let Some(target) = &lease.target {
                        match store::check_publication(
                            tx,
                            &self.sealer,
                            network,
                            *node,
                            target.execution_publication,
                            now.timestamp_millis(),
                        ) {
                            Ok(()) => Some(target.clone()),
                            Err(NodeError::Conflict) => None,
                            Err(error) => return Err(state_error(error)),
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };
            bindings.insert(
                *node,
                (
                    revoked,
                    lease
                        .as_ref()
                        .map(|l| (l.instance_id, l.generation, l.target.clone())),
                ),
            );
            current.insert(
                *node,
                CurrentNode {
                    instance_id: lease.as_ref().map(|l| l.instance_id),
                    execution,
                    configurations,
                    revoked,
                    lease_expires_at: lease.as_ref().map(|l| time(l.expires_at_ms)).transpose()?,
                },
            );
        }
        let authorization_digest = super::proof::request_digest(&(
            network,
            revision,
            actor,
            &visible,
            &full,
            &bindings,
            &observation_paths,
        ))
        .map_err(state_error)?;
        Ok(VisibilityScope {
            network_id: network,
            configuration_revision: revision,
            authorization_digest,
            observation_authorization_digest: super::proof::request_digest(&(
                network, revision, actor, &full,
            ))
            .map_err(state_error)?,
            visible,
            full,
            observation_paths,
            expires_at,
            current,
            mesh_addresses: cfg
                .nodes
                .values()
                .flat_map(|n| addresses(&n.addresses))
                .collect(),
        })
    }

    pub(super) fn node_admission(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
        now: DateTime<Utc>,
    ) -> Result<ReportAdmission, StateError> {
        let has_instance: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM node_execution WHERE network_id=?1 AND node_id=?2 AND boot_id=?3 AND instance_expires_at>?4)",
            params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice(), self.boot.as_uuid().as_bytes().as_slice(), now.timestamp_millis()], |r| r.get(0))?;
        if !has_instance {
            return Err(StateError::Conflict);
        }
        let lease = store::lease_for_node(tx, &self.sealer, network, node).map_err(state_error)?;
        let (_, cfg, bindings) =
            store::current_configuration(tx, &self.sealer, network).map_err(state_error)?;
        store::identity_eligible(tx, &cfg, &bindings, network, node).map_err(state_error)?;
        let mut report = ReportAdmission {
            network_id: network,
            node_id: node,
            instance_id: lease.instance_id,
            lease_expires_at: time(lease.expires_at_ms)?,
            execution: None,
            configurations: lease.target.clone().into_iter().collect(),
            contexts: vec![],
            mesh_addresses: cfg
                .nodes
                .values()
                .flat_map(|n| addresses(&n.addresses))
                .collect(),
        };
        if let Some((publication, target)) =
            store::target_for_node(tx, &self.sealer, network, node).map_err(state_error)?
        {
            let eligible = match store::check_publication(
                tx,
                &self.sealer,
                network,
                node,
                publication,
                now.timestamp_millis(),
            ) {
                Ok(()) => true,
                Err(NodeError::Conflict) => false,
                Err(error) => return Err(state_error(error)),
            };
            if eligible
                && let Some(expiry) = lease
                    .execution_expires_at_ms
                    .filter(|expiry| *expiry > now.timestamp_millis() as u64)
            {
                report.execution = lease.target.clone();
                report.lease_expires_at = report.lease_expires_at.min(time(expiry)?);
            }
            report
                .contexts
                .push(configuration_admission(&target, eligible));
        }
        for target in store::receipt_history_targets(
            tx,
            &self.sealer,
            network,
            node,
            lease.instance_id,
            now.timestamp_millis(),
        )
        .map_err(state_error)?
        {
            let context = configuration_admission(&target, false);
            if !report.configurations.contains(&context.configuration) {
                report.configurations.push(context.configuration.clone());
                report.contexts.push(context);
            }
        }
        Ok(report)
    }
}

fn configuration_admission(
    target: &CompiledNode,
    installation_allowed: bool,
) -> ConfigurationAdmission {
    let cfg = &target.configuration;
    let mut report = ConfigurationAdmission {
        path_deadlines: BTreeMap::new(),
        configuration: ConfigurationRef {
            revision: cfg.revision,
            execution_publication: cfg.publication,
            digest: target.digest,
        },
        installation_allowed,
        resources: BTreeMap::new(),
        peers: BTreeSet::new(),
        paths: vec![],
        profiles: BTreeSet::new(),
        neighbor_transports: cfg.neighbor_transports(),
    };
    report.peers = cfg.peers.keys().copied().collect();
    report.paths = cfg
        .permits
        .values()
        .flat_map(|p| p.paths.iter().map(|p| p.nodes.clone()))
        .chain(
            cfg.diagnostics
                .values()
                .flat_map(|p| p.paths.iter().map(|p| p.nodes.clone())),
        )
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    report.profiles.insert(cfg.dns.profile.clone());
    for (paths, expires) in cfg
        .permits
        .values()
        .map(|p| (&p.paths, p.expires_at_ms))
        .chain(
            cfg.diagnostics
                .values()
                .map(|p| (&p.paths, p.expires_at_ms)),
        )
    {
        if let Ok(expiry) = time(expires) {
            for path in paths {
                report
                    .path_deadlines
                    .entry(path.nodes.clone())
                    .and_modify(|old| *old = (*old).max(expiry))
                    .or_insert(expiry);
            }
        }
    }
    for (key, listener) in &cfg.listeners {
        let (enabled, network, transport, endpoints) = match listener.declaration() {
            Listener::NativeUdp(l) => (l.enabled, SocketNetwork::Udp, "native_udp", &l.endpoints),
            Listener::NativeTcp(l) => (l.enabled, SocketNetwork::Tcp, "native_tcp", &l.endpoints),
            Listener::Vless(l) => (l.enabled, SocketNetwork::Tcp, "vless", &l.endpoints),
            Listener::Trojan(l) => (l.enabled, SocketNetwork::Tcp, "trojan", &l.endpoints),
        };
        if !enabled {
            continue;
        }
        let key_path = key.replace('~', "~0").replace('/', "~1");
        report.resources.insert(
            ResourceRef::Listener { key: key.clone() },
            ResourceAdmission {
                network,
                transport: transport.into(),
                configured_endpoints: endpoints
                    .iter()
                    .enumerate()
                    .map(|(i, endpoint)| {
                        (
                            format!("/nodes/{}/listeners/{key_path}/endpoints/{i}", cfg.node_id),
                            endpoint_address(endpoint),
                        )
                    })
                    .collect(),
            },
        );
    }
    for (key, dial) in &cfg.dialers {
        let (network, transport) = match dial.parameters {
            DialParameters::NativeUdp { .. } => (SocketNetwork::Udp, "native_udp"),
            DialParameters::NativeTcp { .. } => (SocketNetwork::Tcp, "native_tcp"),
            DialParameters::Vless { .. } => (SocketNetwork::Tcp, "vless"),
            DialParameters::Trojan { .. } => (SocketNetwork::Tcp, "trojan"),
        };
        // DialCandidate.endpoints are remote destinations, never claims about
        // the local dial socket's externally reachable address.
        report.resources.insert(
            ResourceRef::Dial { key: key.clone() },
            ResourceAdmission {
                network,
                transport: transport.into(),
                configured_endpoints: BTreeMap::new(),
            },
        );
    }
    report
}
fn endpoint_address(endpoint: &Endpoint) -> String {
    match endpoint.host.parse::<IpAddr>() {
        Ok(ip) => SocketAddr::new(ip, endpoint.port.get()).to_string(),
        Err(_) => format!("{}:{}", endpoint.host, endpoint.port.get()),
    }
}
fn addresses(addresses: &NodeAddresses) -> impl Iterator<Item = IpAddr> {
    [
        addresses.ipv4.map(IpAddr::V4),
        addresses.ipv6.map(IpAddr::V6),
    ]
    .into_iter()
    .flatten()
}
fn time(ms: u64) -> Result<DateTime<Utc>, StateError> {
    i64::try_from(ms)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .ok_or(StateError::Storage)
}
fn state_error(error: NodeError) -> StateError {
    match error {
        NodeError::Unauthorized => StateError::Unauthorized,
        NodeError::Forbidden => StateError::Forbidden,
        NodeError::Invalid | NodeError::InvalidSettings(_) => StateError::Invalid,
        NodeError::Conflict => StateError::Conflict,
        NodeError::NotFound => StateError::NotFound,
        NodeError::Busy => StateError::Budget,
        NodeError::Storage => StateError::Storage,
    }
}

impl crate::observations::service::ObservationProvider for Execution {
    fn prepare_observation(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &meshway_protocol::observations::CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<crate::observations::TaskAdmission, StateError> {
        self.prepare_diagnostic(tx, identity, request, now)
    }
    fn synchronize(
        &self,
        tx: &Transaction<'_>,
        subscriptions: &[crate::observations::service::SubscriptionLease],
        now: DateTime<Utc>,
    ) -> Result<(), StateError> {
        self.synchronize_diagnostics(tx, subscriptions, now)
    }
    fn scope(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<VisibilityScope, StateError> {
        self.state_scope_in_transaction(tx, identity, now)
    }
    fn report(
        &self,
        tx: &Transaction<'_>,
        authorization: &str,
        now: DateTime<Utc>,
    ) -> Result<ReportAdmission, StateError> {
        self.report_admission_in_transaction(tx, authorization, now)
    }
    fn observation(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &meshway_protocol::observations::CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<crate::observations::TaskAdmission, StateError> {
        self.observation_admission_in_transaction(tx, identity, request, now)
    }
    fn observation_session(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<SessionId, StateError> {
        let ReadIdentity::UserSession { digest, .. } = identity else {
            return Err(StateError::Forbidden);
        };
        self.state_scope_in_transaction(tx, identity, now)?;
        current_session(tx, digest, now.timestamp_millis())
            .map(|session| session.session_id)
            .map_err(|_| StateError::Unauthorized)
    }
}
