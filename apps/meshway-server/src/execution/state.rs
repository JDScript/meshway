//! Execution-side authorization projection for State. The root StateProvider
//! composes these transaction-checked facts with configuration/entity projections.
use super::Execution;
use crate::error::Error;
use crate::{
    state::{
        ConfigurationAdmission, CurrentNode, ReportAdmission, ResourceAdmission, VisibilityScope,
        service::ReadIdentity,
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
use rusqlite::Transaction;
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
};

#[cfg(test)]
thread_local! {
    static STATE_WORK: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

impl Execution {
    /// Test accounting lives on the same isolated SQLite executor thread.
    #[cfg(test)]
    pub(crate) fn take_state_work() -> (usize, usize) {
        STATE_WORK.with(|counts| counts.replace((0, 0)))
    }
    /// Reauthorize an accepted diagnostic subscription against its current
    /// exact service permit. Only prepare_observation may create a lifetime.
    pub fn observation_admission_in_transaction(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &meshway_protocol::observations::CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<crate::observations::TaskAdmission, Error> {
        self.diagnostic_admission(tx, identity, request, now)
    }
    /// Authenticate the real Node session and current instance in the same SQL
    /// transaction C uses to accept a report. No synthetic principal or bearer.
    pub fn report_admission_in_transaction(
        &self,
        tx: &Transaction<'_>,
        token: &str,
        now: DateTime<Utc>,
    ) -> Result<ReportAdmission, Error> {
        self.report_with_context(tx, token, now, true)
    }

    /// Task polling/results cannot use retained publications. State/history
    /// callers explicitly retain those contexts through the public entrypoint.
    fn report_with_context(
        &self,
        tx: &Transaction<'_>,
        token: &str,
        now: DateTime<Utc>,
        retain_history: bool,
    ) -> Result<ReportAdmission, Error> {
        let principal = store::authorize_node(
            tx,
            &self.sealer,
            self.boot,
            super::proof::token_hash(token)?,
            now.timestamp_millis(),
        )?;
        let mut report = if retain_history {
            self.node_admission(tx, principal.network, principal.node, now)?
        } else {
            self.current_node_admission(tx, principal.network, principal.node, now)?
        };
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
    ) -> Result<VisibilityScope, Error> {
        let (network, full, mut expires_at, actor) = match identity {
            ReadIdentity::Node { authorization } => {
                #[cfg(test)]
                STATE_WORK.with(|counts| {
                    let (checks, history) = counts.get();
                    counts.set((checks + 1, history));
                });
                // A visibility scope uses only the current subject, instance
                // and deadline. Retained report/history contexts grant no
                // visibility here and must not be decrypted on every SSE frame.
                let report = self.report_with_context(tx, authorization, now, false)?;
                (
                    report.network_id,
                    BTreeSet::from([report.node_id]),
                    report.lease_expires_at,
                    super::proof::request_digest(&("node", report.node_id))?,
                )
            }
            ReadIdentity::UserSession { digest, network } => {
                let session = current_session(tx, digest, now.timestamp_millis())?;
                let (membership, role) =
                    store::admission::membership_role(tx, *network, session.user.id)?;
                let (_, cfg, bindings) = store::current_configuration(tx, &self.sealer, *network)?;
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
                    super::proof::request_digest(&(
                        "user",
                        session.user.id,
                        membership.as_uuid().as_bytes().to_vec(),
                        role,
                    ))?,
                )
            }
        };
        let (revision, cfg, _) = store::current_configuration(tx, &self.sealer, network)?;
        let mut visible = full.clone();
        let mut observation_paths = BTreeSet::new();
        for node in &full {
            if let Some((publication, target)) =
                store::target_for_node(tx, &self.sealer, network, *node)?
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
                    Err(Error::BindingConflict) => continue,
                    Err(error) => return Err(error),
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
            let revoked = store::admission::credentials_revoked(tx, network, *node)?;
            let live = store::admission::has_live_instance(
                tx,
                network,
                *node,
                self.boot,
                now.timestamp_millis(),
            )?;
            let lease = if live && !revoked {
                Some(store::lease_for_node(tx, &self.sealer, network, *node)?)
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
                            Err(Error::BindingConflict) => None,
                            Err(error) => return Err(error),
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
                    instance_generation: lease.as_ref().map(|l| l.generation),
                    execution,
                    configurations,
                    revoked,
                    lease_expires_at: lease.as_ref().map(|l| time(l.expires_at_ms)).transpose()?,
                },
            );
        }
        let endpoint_resources = match identity {
            ReadIdentity::Node { .. } => {
                let mut resources = BTreeMap::new();
                for local in &full {
                    let disclosure =
                        self.endpoint_disclosure(tx, network, *local, &visible, now)?;
                    resources.extend(disclosure.resources);
                    if let Some(deadline) = disclosure.expires_at {
                        expires_at = expires_at.min(deadline);
                    }
                }
                Some(resources)
            }
            ReadIdentity::UserSession { .. } => None,
        };
        let authorization_digest = super::proof::request_digest(&(
            network,
            revision,
            actor,
            &visible,
            &full,
            &bindings,
            &observation_paths,
            &endpoint_resources,
        ))?;
        Ok(VisibilityScope {
            network_id: network,
            configuration_revision: revision,
            authorization_digest,
            observation_authorization_digest: super::proof::request_digest(&(
                network, revision, actor, &full,
            ))?,
            hostnames: visible
                .iter()
                .filter_map(|node| cfg.nodes.get(node).map(|n| (*node, n.hostname.clone())))
                .collect(),
            visible,
            full,
            endpoint_resources,
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

    /// Current transactional identity/instance/publication admission. Acquisition
    /// and endpoint disclosure never use retained receipt/history contexts.
    pub(super) fn current_node_admission(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
        now: DateTime<Utc>,
    ) -> Result<ReportAdmission, Error> {
        self.current_node_context(tx, network, node, now)
            .map(|(report, _)| report)
    }

    /// Return the immutable target already read for current admission so a
    /// transaction-local consumer does not decrypt the publication again.
    pub(super) fn current_node_context(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
        now: DateTime<Utc>,
    ) -> Result<(ReportAdmission, Option<CompiledNode>), Error> {
        let has_instance = store::admission::has_live_instance(
            tx,
            network,
            node,
            self.boot,
            now.timestamp_millis(),
        )?;
        if !has_instance {
            return Err(Error::StateConflict);
        }
        let lease = store::lease_for_node(tx, &self.sealer, network, node)?;
        let (_, cfg, bindings) = store::current_configuration(tx, &self.sealer, network)?;
        store::identity_eligible(tx, &cfg, &bindings, network, node)?;
        let mut report = ReportAdmission {
            network_id: network,
            node_id: node,
            instance_id: lease.instance_id,
            instance_generation: lease.generation,
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
        let target = store::target_for_node(tx, &self.sealer, network, node)?;
        if let Some((publication, target)) = &target {
            let eligible = match store::check_publication(
                tx,
                &self.sealer,
                network,
                node,
                *publication,
                now.timestamp_millis(),
            ) {
                Ok(()) => true,
                Err(Error::BindingConflict) => false,
                Err(error) => return Err(error),
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
                .push(configuration_admission(target, eligible));
        }
        Ok((report, target.map(|(_, target)| target)))
    }

    pub(super) fn node_admission(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
        now: DateTime<Utc>,
    ) -> Result<ReportAdmission, Error> {
        #[cfg(test)]
        STATE_WORK.with(|counts| {
            let (checks, history) = counts.get();
            counts.set((checks, history + 1));
        });
        let mut report = self.current_node_admission(tx, network, node, now)?;
        let history = store::receipt_history_targets(
            tx,
            &self.sealer,
            network,
            node,
            report.instance_id,
            now.timestamp_millis(),
        )?
        .into_iter()
        .chain(store::recent_targets_for_node(
            tx,
            &self.sealer,
            network,
            node,
            now.timestamp_millis(),
        )?);
        for target in history {
            if !report.configurations.contains(&target.reference) {
                report.configurations.push(target.reference);
                if let Some(current) = target.current {
                    report
                        .contexts
                        .push(configuration_admission(&current, false));
                }
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
        path_transports: BTreeMap::new(),
        diagnostic_transports: BTreeMap::new(),
        mesh_address_pools: cfg.mesh_address_pools.clone(),
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
    use meshway_configuration::execution::{Carrier, CircuitAuthorization};
    for (authorization, paths) in cfg
        .permits
        .iter()
        .map(|(id, p)| (CircuitAuthorization::Business(*id), &p.paths))
        .chain(
            cfg.diagnostics
                .iter()
                .map(|(id, p)| (CircuitAuthorization::Diagnostic(*id), &p.paths)),
        )
    {
        for path in paths
            .iter()
            .filter(|path| path.nodes.first() == Some(&cfg.node_id))
        {
            let Some(peer) = path.nodes.get(1) else {
                continue;
            };
            let outgoing = cfg
                .dialers
                .values()
                .filter(|dial| dial.peer == *peer)
                .map(|dial| Carrier {
                    dialer: cfg.node_id,
                    listener: *peer,
                    listener_key: dial.listener_key.clone(),
                    protocol: dial.parameters.transport(),
                });
            let incoming = cfg.listeners.iter().map(|(key, listener)| Carrier {
                dialer: *peer,
                listener: cfg.node_id,
                listener_key: key.clone(),
                protocol: match listener.declaration() {
                    Listener::NativeUdp(_) => meshway_transport_spec::TransportProtocol::NativeUdp,
                    Listener::NativeTcp(_) => meshway_transport_spec::TransportProtocol::NativeTcp,
                    Listener::Vless(_) => meshway_transport_spec::TransportProtocol::Vless,
                    Listener::Trojan(_) => meshway_transport_spec::TransportProtocol::Trojan,
                },
            });
            for carrier in outgoing.chain(incoming) {
                if meshway_policy::admission::admit_circuit_carrier(
                    cfg,
                    authorization,
                    &path.nodes,
                    *peer,
                    &carrier,
                )
                .is_ok()
                {
                    report
                        .path_transports
                        .entry(path.nodes.clone())
                        .or_default()
                        .insert(carrier.protocol.as_str().into());
                    if let CircuitAuthorization::Diagnostic(id) = authorization {
                        report
                            .diagnostic_transports
                            .entry(id)
                            .or_default()
                            .entry(path.nodes.clone())
                            .or_default()
                            .insert(carrier.protocol.as_str().into());
                    }
                }
            }
        }
    }
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
fn time(ms: u64) -> Result<DateTime<Utc>, Error> {
    i64::try_from(ms)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .ok_or(Error::decode("execution expiry"))
}

impl crate::observations::service::ObservationProvider for Execution {
    fn node_observation_identity(
        &self,
        tx: &Transaction<'_>,
        token: &str,
        instance: &meshway_protocol::InstanceRequest,
        now: DateTime<Utc>,
    ) -> Result<crate::observations::service::NodeObservationIdentity, Error> {
        self.diagnostic_node_identity(tx, token, instance, now)
    }
    fn prepare_node_observation(
        &self,
        tx: &Transaction<'_>,
        token: &str,
        request: &meshway_protocol::node_observations::CreateNodeObservation,
        now: DateTime<Utc>,
    ) -> Result<crate::observations::service::NodeObservationAdmission, Error> {
        self.prepare_node_diagnostic(tx, token, request, now)
    }
    fn node_observation(
        &self,
        tx: &Transaction<'_>,
        subscription: ObservationSubscriptionId,
        now: DateTime<Utc>,
    ) -> Result<crate::observations::service::NodeObservationAdmission, Error> {
        self.node_diagnostic_admission(tx, subscription, now)
    }
    fn prepare_observation(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &meshway_protocol::observations::CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<crate::observations::TaskAdmission, Error> {
        self.prepare_diagnostic(tx, identity, request, now)
    }
    fn synchronize(
        &self,
        tx: &Transaction<'_>,
        subscriptions: &[crate::observations::service::SubscriptionLease],
        now: DateTime<Utc>,
    ) -> Result<(), Error> {
        self.synchronize_diagnostics(tx, subscriptions, now)
    }
    fn scope(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<VisibilityScope, Error> {
        self.state_scope_in_transaction(tx, identity, now)
    }
    fn report(
        &self,
        tx: &Transaction<'_>,
        authorization: &str,
        now: DateTime<Utc>,
    ) -> Result<ReportAdmission, Error> {
        self.report_with_context(tx, authorization, now, false)
    }
    fn observation(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &meshway_protocol::observations::CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<crate::observations::TaskAdmission, Error> {
        self.observation_admission_in_transaction(tx, identity, request, now)
    }
    fn observation_session(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<SessionId, Error> {
        let ReadIdentity::UserSession { digest, network } = identity else {
            return Err(Error::PermissionDenied);
        };
        let session = current_session(tx, digest, now.timestamp_millis())?;
        store::admission::membership_role(tx, *network, session.user.id)?;
        Ok(session.session_id)
    }
}
