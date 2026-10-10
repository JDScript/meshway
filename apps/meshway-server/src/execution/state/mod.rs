//! Execution-side authorization projection for State. What a reader may see
//! and which resources a Node may report follow the current netmaps alone,
//! never business flows or measurements. The root StateProvider composes
//! these transaction-checked facts with configuration/entity projections.
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
    execution::DialParameters,
    netmap::{CompiledNetmap, NodeNetmap},
};
use meshway_model::state::{ResourceRef, SocketNetwork};
use meshway_model::*;
use meshway_transport_spec::{Endpoint, Listener};
use rusqlite::Transaction;
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

impl Execution {
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

    /// Task polling/results use the current netmap only; State/history
    /// callers also accept the delta base the Node may still report.
    pub(crate) fn report_with_context(
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

    /// What a reader may see is the union of the current netmap projections
    /// of its own Nodes, independent of which business flows or measurements
    /// exist. It changes only with a netmap serial. Endpoint resources reach
    /// a Node reader only for adjacent carriers.
    pub fn state_scope_in_transaction(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<VisibilityScope, Error> {
        let netmaps = &*self.netmaps;
        let ScopeSubject {
            network,
            full,
            expires_at,
            actor,
        } = self.scope_subject(tx, identity, now)?;
        let (revision, cfg, _) = store::current_configuration(tx, &self.sealer, network)?;
        let mut owned = BTreeMap::new();
        for node in &full {
            if let Some(netmap) = netmaps.current(tx, network, *node)? {
                owned.insert(*node, netmap);
            }
        }
        let mut visible = full.clone();
        for netmap in owned.values() {
            visible.extend(crate::state::facts::visible(&netmap.netmap));
        }
        visible.retain(|node| cfg.nodes.contains_key(node));
        let (current, bindings) = self.current_nodes(tx, network, &visible, now)?;
        let endpoint_resources = match identity {
            ReadIdentity::Node { .. } => Some(
                owned
                    .values()
                    .flat_map(|netmap| netmap_endpoint_resources(&netmap.netmap))
                    .collect(),
            ),
            ReadIdentity::UserSession { .. } => None,
        };
        let observation_paths = BTreeSet::new();
        let references: BTreeMap<_, _> = owned
            .iter()
            .map(|(node, n)| (*node, n.reference()))
            .collect();
        let authorization_digest = crate::execution::proof::request_digest(&(
            network,
            revision,
            actor,
            &visible,
            &full,
            &bindings,
            &references,
            &endpoint_resources,
        ))?;
        Ok(VisibilityScope {
            network_id: network,
            configuration_revision: revision,
            authorization_digest,
            observation_authorization_digest: crate::execution::proof::request_digest(&(
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

    /// The authenticated reader and the Nodes it fully owns.
    fn scope_subject(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<ScopeSubject, Error> {
        Ok(match identity {
            ReadIdentity::Node { authorization } => {
                // A visibility scope uses only the current subject, instance
                // and deadline. Retained report/history contexts grant no
                // visibility here and must not be decrypted on every SSE frame.
                let report = self.report_with_context(tx, authorization, now, false)?;
                ScopeSubject {
                    network: report.network_id,
                    full: BTreeSet::from([report.node_id]),
                    expires_at: report.lease_expires_at,
                    actor: super::proof::request_digest(&("node", report.node_id))?,
                }
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
                ScopeSubject {
                    network: *network,
                    full,
                    expires_at: session.expires_at,
                    actor: super::proof::request_digest(&(
                        "user",
                        session.user.id,
                        membership.as_uuid().as_bytes().to_vec(),
                        role,
                    ))?,
                }
            }
        })
    }

    /// Current instance, netmap and revocation of every visible Node.
    fn current_nodes(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        visible: &BTreeSet<NodeId>,
        now: DateTime<Utc>,
    ) -> Result<(BTreeMap<NodeId, CurrentNode>, NodeBindings), Error> {
        let mut current = BTreeMap::new();
        let mut bindings = BTreeMap::new();
        for node in visible {
            let revoked = store::admission::credentials_revoked(tx, network, *node)?;
            let live = store::admission::has_live_instance(
                tx,
                network,
                *node,
                self.boot,
                now.timestamp_millis(),
            )?;
            let lease = if live && !revoked {
                Some(store::lease_for_node(tx, network, *node)?)
            } else {
                None
            };
            let netmap = if lease.is_some() {
                crate::store::netmap::current(tx, network, *node)?.map(|c| c.current)
            } else {
                None
            };
            bindings.insert(
                *node,
                (
                    revoked,
                    lease
                        .as_ref()
                        .map(|l| (l.instance_id, l.generation, netmap)),
                ),
            );
            current.insert(
                *node,
                CurrentNode {
                    instance_id: lease.as_ref().map(|l| l.instance_id),
                    instance_generation: lease.as_ref().map(|l| l.generation),
                    execution: netmap,
                    configurations: netmap.into_iter().collect(),
                    revoked,
                    lease_expires_at: lease.as_ref().map(|l| time(l.expires_at_ms)).transpose()?,
                },
            );
        }
        Ok((current, bindings))
    }

    /// Current transactional identity/instance/netmap admission. Acquisition
    /// and endpoint disclosure never use the retained delta base.
    pub(crate) fn current_node_admission(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
        now: DateTime<Utc>,
    ) -> Result<ReportAdmission, Error> {
        self.current_node_context(tx, network, node, now)
            .map(|(report, _)| report)
    }

    /// Return the verified netmap already read for current admission so a
    /// transaction-local consumer does not read it again.
    pub(crate) fn current_node_context(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
        now: DateTime<Utc>,
    ) -> Result<(ReportAdmission, Option<Arc<CompiledNetmap>>), Error> {
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
        let lease = store::lease_for_node(tx, network, node)?;
        let (_, cfg, bindings) = store::current_configuration(tx, &self.sealer, network)?;
        store::identity_eligible(tx, &cfg, &bindings, network, node)?;
        let netmap = self.netmaps.current(tx, network, node)?;
        let mut report = ReportAdmission {
            network_id: network,
            node_id: node,
            instance_id: lease.instance_id,
            instance_generation: lease.generation,
            lease_expires_at: time(lease.expires_at_ms)?,
            execution: netmap.as_ref().map(|netmap| netmap.reference()),
            configurations: netmap.iter().map(|netmap| netmap.reference()).collect(),
            contexts: netmap
                .iter()
                .map(|netmap| configuration_admission(netmap, true))
                .collect(),
            mesh_addresses: cfg
                .nodes
                .values()
                .flat_map(|n| addresses(&n.addresses))
                .collect(),
        };
        if netmap.is_none() {
            report.execution = None;
        }
        Ok((report, netmap))
    }

    /// Current admission plus the delta base the Node may still name in late
    /// receipts and State: history only, never installation authority.
    pub(crate) fn node_admission(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
        now: DateTime<Utc>,
    ) -> Result<ReportAdmission, Error> {
        let mut report = self.current_node_admission(tx, network, node, now)?;
        if let Some(previous) =
            crate::store::netmap::current(tx, network, node)?.and_then(|c| c.previous)
            && !report.configurations.contains(&previous)
        {
            let netmap = crate::store::netmap::load(tx, &self.sealer, network, node, previous)?;
            report.configurations.push(previous);
            report
                .contexts
                .push(configuration_admission(&netmap, false));
        }
        Ok(report)
    }

    /// The current user session of an observation subscriber.
    pub(crate) fn observation_session(
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

struct ScopeSubject {
    network: NetworkId,
    full: BTreeSet<NodeId>,
    expires_at: DateTime<Utc>,
    actor: Digest,
}

type NodeBindings = BTreeMap<NodeId, (bool, Option<(InstanceId, Version, Option<NetmapRef>)>)>;

fn transport(listener: &Listener) -> (bool, SocketNetwork, &'static str) {
    match listener {
        Listener::NativeUdp(l) => (l.enabled, SocketNetwork::Udp, "native_udp"),
        Listener::NativeTcp(l) => (l.enabled, SocketNetwork::Tcp, "native_tcp"),
        Listener::Vless(l) => (l.enabled, SocketNetwork::Tcp, "vless"),
        Listener::Trojan(l) => (l.enabled, SocketNetwork::Tcp, "trojan"),
    }
}

/// The resources and carriers one netmap lets its Node report. A peer's
/// carrier transports are those of its dial candidates and of this Node's
/// enabled listeners; reports are plausibility-checked, never authorized.
fn configuration_admission(
    compiled: &CompiledNetmap,
    installation_allowed: bool,
) -> ConfigurationAdmission {
    let netmap: &NodeNetmap = &compiled.netmap;
    let incoming: BTreeSet<String> = netmap
        .self_
        .listeners
        .values()
        .map(|listener| transport(listener.declaration()))
        .filter(|(enabled, _, _)| *enabled)
        .map(|(_, _, name)| name.to_owned())
        .collect();
    let mut neighbor_transports: BTreeMap<NodeId, BTreeSet<String>> = BTreeMap::new();
    for peer in netmap.projection.inventory.keys() {
        if *peer != netmap.node_id {
            neighbor_transports.insert(*peer, incoming.clone());
        }
    }
    for dial in netmap.dialers.values() {
        neighbor_transports
            .entry(dial.peer)
            .or_default()
            .insert(dial.parameters.transport().as_str().into());
    }
    let mut report = ConfigurationAdmission {
        configuration: compiled.reference(),
        installation_allowed,
        resources: BTreeMap::new(),
        peers: netmap
            .projection
            .inventory
            .keys()
            .copied()
            .filter(|peer| *peer != netmap.node_id)
            .collect(),
        profiles: BTreeSet::from([netmap.dns.profile.clone()]),
        neighbor_transports,
        mesh_address_pools: netmap.self_.mesh_address_pools.clone(),
    };
    for (key, listener) in &netmap.self_.listeners {
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
                            format!(
                                "/nodes/{}/listeners/{key_path}/endpoints/{i}",
                                netmap.node_id
                            ),
                            endpoint_address(endpoint),
                        )
                    })
                    .collect(),
            },
        );
    }
    for (key, dial) in &netmap.dialers {
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

/// Peer listeners the owner may dial: the only endpoint details it may read,
/// the same rule as its peer facts.
fn netmap_endpoint_resources(netmap: &NodeNetmap) -> BTreeMap<NodeId, BTreeSet<ResourceRef>> {
    let mut resources: BTreeMap<NodeId, BTreeSet<ResourceRef>> = BTreeMap::new();
    for dial in netmap.dialers.values() {
        resources
            .entry(dial.peer)
            .or_default()
            .insert(ResourceRef::Listener {
                key: dial.listener_key.clone(),
            });
    }
    resources
}
