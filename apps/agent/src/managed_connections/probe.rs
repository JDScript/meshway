//! Read-only handles to the manager's current runtime. No new runtime or grant.
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use meshway_dataplane::Runtime;
use meshway_protocol::{NodeConfiguration, NodeConnections};
use tokio::{
    sync::watch,
    time::{self, Instant},
};

use super::{Manager, Observation, Phase, session};
use crate::diagnostics::{
    AuthenticatedChannel, Evidence, LiveState, LiveStatus, MAX_PEERS, ProbeError, ProbeObservation,
    ProbeRequest, PublicIdentity, SCHEMA_VERSION, WhoisObservation,
};

#[derive(Clone, PartialEq, Eq)]
struct Inventory {
    revision: u32,
    nodes: BTreeMap<String, PublicIdentity>,
}

#[derive(Clone)]
struct Generation {
    number: u64,
    installed_revision: u32,
    source_revision: u32,
    inventory: Option<Arc<Inventory>>,
    runtime: Weak<Runtime>,
    intent: NodeConnections,
    deadline: watch::Receiver<Option<Instant>>,
    observed: watch::Receiver<Observation>,
}

impl Generation {
    fn current(&self) -> bool {
        self.runtime.strong_count() > 0
            && self.deadline.has_changed().is_ok()
            && self.observed.has_changed().is_ok()
            && self
                .deadline
                .borrow()
                .is_some_and(|deadline| deadline > Instant::now())
            && self.observed.borrow().state == Phase::Applied
    }

    async fn invalidated(&self) {
        let mut deadlines = self.deadline.clone();
        let mut observed = self.observed.clone();
        loop {
            let deadline = *deadlines.borrow_and_update();
            let applied = observed.borrow_and_update().state == Phase::Applied;
            let Some(deadline) = deadline.filter(|_| applied) else {
                return;
            };
            tokio::select! {
                biased;
                _ = time::sleep_until(deadline) => return,
                changed = deadlines.changed() => if changed.is_err() { return; },
                changed = observed.changed() => if changed.is_err() { return; },
            }
        }
    }
}

pub(super) struct Publisher {
    current: watch::Sender<Option<Arc<Generation>>>,
    inventory: watch::Sender<Option<Arc<Inventory>>>,
    sequence: AtomicU64,
}

impl Publisher {
    pub fn new() -> Self {
        Self {
            current: watch::channel(None).0,
            inventory: watch::channel(None).0,
            sequence: AtomicU64::new(0),
        }
    }

    pub fn withdraw(&self) {
        self.current.send_replace(None);
    }

    /// An equivalent fresh source renews the existing runtime. Preserve its
    /// actual installed revision, while binding metadata to the accepted source.
    pub fn refresh_source_revision(&self, revision: u32) {
        let old = self.current.borrow().clone();
        if let Some(old) = old {
            let inventory = self
                .inventory
                .borrow()
                .clone()
                .filter(|inventory| inventory.revision == revision);
            if old.source_revision == revision && old.inventory == inventory {
                return;
            }
            self.current.send_replace(Some(Arc::new(Generation {
                number: self.sequence.fetch_add(1, Ordering::Relaxed) + 1,
                source_revision: revision,
                inventory,
                ..old.as_ref().clone()
            })));
        }
    }

    pub fn publish(
        &self,
        runtime: &Arc<Runtime>,
        intent: &NodeConnections,
        deadline: &watch::Sender<Option<Instant>>,
        observed: &watch::Receiver<Observation>,
        revision: u32,
    ) {
        let inventory = self
            .inventory
            .borrow()
            .clone()
            .filter(|inventory| inventory.revision == revision);
        self.current.send_replace(Some(Arc::new(Generation {
            number: self.sequence.fetch_add(1, Ordering::Relaxed) + 1,
            installed_revision: revision,
            source_revision: revision,
            inventory,
            runtime: Arc::downgrade(runtime),
            intent: intent.clone(),
            deadline: deadline.subscribe(),
            observed: observed.clone(),
        })));
    }
}

impl Manager {
    /// Captures no strong runtime reference; request lifetimes are lease-fenced.
    pub fn prober(&self) -> Prober {
        Prober {
            current: self.probes.current.subscribe(),
        }
    }

    pub fn set_probe_inventory(&self, configuration: &NodeConfiguration) {
        let inventory = Arc::new(Inventory {
            revision: configuration.revision,
            nodes: std::iter::once(&configuration.node)
                .chain(&configuration.peers)
                .map(|node| {
                    (
                        node.id.clone(),
                        PublicIdentity {
                            node_id: node.id.clone(),
                            name: node.name.clone(),
                            mesh_ipv4: node.mesh_ipv4.clone(),
                            mesh_ipv6: node.mesh_ipv6.clone(),
                        },
                    )
                })
                .collect(),
        });
        if self.probes.inventory.borrow().as_ref() == Some(&inventory) {
            return;
        }
        self.probes.inventory.send_replace(Some(inventory));
        // Updating trusted metadata replaces the publication too. Pending probes
        // cannot return identity from a previous inventory generation. A newer
        // inventory is staged until the matching connection source is accepted.
        let old = self.probes.current.borrow().clone();
        if let Some(old) = old {
            self.probes.refresh_source_revision(old.source_revision);
        }
    }
}

#[derive(Clone)]
pub(crate) struct Prober {
    current: watch::Receiver<Option<Arc<Generation>>>,
}

impl Prober {
    fn current(&self, generation: &Arc<Generation>) -> bool {
        self.current.has_changed().is_ok()
            && self
                .current
                .borrow()
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, generation))
            && generation.current()
    }

    fn resolve(
        &self,
        peer: &str,
        generation: &Generation,
    ) -> Result<(String, Option<String>), ProbeError> {
        let intent = &generation.intent;
        let inventory = generation.inventory.as_ref();
        let id = if meshway_protocol::local::valid_credential(peer) {
            peer.to_owned()
        } else {
            let inventory = inventory.ok_or(ProbeError::Unavailable)?;
            let mut matches = inventory.nodes.iter().filter(|(_, node)| node.name == peer);
            let (id, _) = matches.next().ok_or(ProbeError::UnconfiguredPeer)?;
            if matches.next().is_some() {
                return Err(ProbeError::AmbiguousPeer);
            }
            id.clone()
        };
        if id == intent.node_id
            || !intent.peers.iter().any(|p| p.node_id == id)
            || !(intent.outgoing.iter().any(|p| p.peer_node_id == id)
                || intent.incoming.iter().any(|p| p.peer_node_id == id))
        {
            return Err(ProbeError::UnconfiguredPeer);
        }
        let name = inventory
            .and_then(|inventory| inventory.nodes.get(&id))
            .map(|node| node.name.clone());
        Ok((id, name))
    }

    fn generation(&self) -> Result<Arc<Generation>, ProbeError> {
        // Drop the watch read guard before the fencing check borrows it again.
        // A concurrent waiting writer must not turn nested reads into deadlock.
        let generation = self.current.borrow().clone();
        generation
            .filter(|generation| self.current(generation))
            .ok_or(ProbeError::Unavailable)
    }

    /// Registry evidence is a point-in-time handshake observation, not a probe
    /// or a business reachability claim. Pages are explicit and never truncated.
    pub fn live_status(
        &self,
        peers: bool,
        offset: u32,
        limit: u32,
    ) -> Result<LiveStatus, ProbeError> {
        if !(1..=MAX_PEERS).contains(&limit) || (!peers && offset != 0) {
            return Err(ProbeError::InvalidRequest);
        }
        let generation = self.generation()?;
        let runtime = generation
            .runtime
            .upgrade()
            .ok_or(ProbeError::Unavailable)?;
        let mut channels = runtime.peer_connections();
        channels.sort_by(|a, b| a.peer_node_id.cmp(&b.peer_node_id));
        let count = u32::try_from(channels.len()).map_err(|_| ProbeError::InvalidRequest)?;
        if offset > count {
            return Err(ProbeError::InvalidRequest);
        }
        let page = if peers {
            Some(
                channels
                    .into_iter()
                    .skip(offset as usize)
                    .take(limit as usize)
                    .map(|connection| {
                        session::describe(&generation.intent, &connection)
                            .map(|description| AuthenticatedChannel {
                                peer_node_id: description.peer_node_id,
                                initiator_node_id: description.initiator_node_id,
                                transport: description.transport,
                            })
                            .ok_or(ProbeError::Unavailable)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            )
        } else {
            None
        };
        let next = page
            .as_ref()
            .map(|page| offset + page.len() as u32)
            .filter(|next| *next < count);
        let deadline = (*generation.deadline.borrow()).ok_or(ProbeError::Unavailable)?;
        let result = LiveStatus {
            schema_version: SCHEMA_VERSION,
            state: LiveState::Installed,
            generation: generation.number,
            installed_revision: generation.installed_revision,
            source_revision: generation.source_revision,
            lease_remaining_ms: deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(u64::MAX as u128) as u64,
            local_node_id: generation.intent.node_id.clone(),
            local_identity: generation
                .inventory
                .as_ref()
                .and_then(|inventory| inventory.nodes.get(&generation.intent.node_id))
                .cloned(),
            network_id: None,
            authenticated_channel_count: count,
            peers: page,
            peer_offset: offset,
            next_peer_offset: next,
            evidence: Evidence::CurrentAuthenticatedChannelRegistry,
        };
        if !self.current(&generation) {
            return Err(ProbeError::Unavailable);
        }
        Ok(result)
    }

    pub fn whois(&self, query: &str) -> Result<WhoisObservation, ProbeError> {
        if query.is_empty() || query.chars().count() > 128 {
            return Err(ProbeError::InvalidRequest);
        }
        let generation = self.generation()?;
        let result = (|| {
            let inventory = generation
                .inventory
                .as_ref()
                .ok_or(ProbeError::Unavailable)?;
            let identity = if meshway_protocol::local::valid_credential(query) {
                inventory
                    .nodes
                    .get(query)
                    .ok_or(ProbeError::UnknownIdentity)?
            } else {
                let ip = query.parse::<std::net::IpAddr>().ok();
                let mut matches = inventory.nodes.values().filter(|node| match ip {
                    Some(ip) => {
                        node.mesh_ipv4.parse::<std::net::IpAddr>().ok() == Some(ip)
                            || node.mesh_ipv6.parse::<std::net::IpAddr>().ok() == Some(ip)
                    }
                    None => node.name == query,
                });
                let identity = matches.next().ok_or(ProbeError::UnknownIdentity)?;
                if matches.next().is_some() {
                    return Err(ProbeError::AmbiguousPeer);
                }
                identity
            };
            Ok(WhoisObservation {
                schema_version: SCHEMA_VERSION,
                generation: generation.number,
                installed_revision: generation.installed_revision,
                source_revision: generation.source_revision,
                identity: identity.clone(),
                evidence: Evidence::CurrentTrustedInventory,
            })
        })();
        if !self.current(&generation) {
            return Err(ProbeError::Unavailable);
        }
        result
    }

    /// Probes only the manager's existing authenticated direct neighbor channel.
    /// Both success and failure are fenced before/after await against withdrawal,
    /// lease expiry and semantic/material generation replacement.
    pub async fn probe_neighbor_observed(
        &self,
        request: ProbeRequest,
    ) -> Result<ProbeObservation, ProbeError> {
        request.validate()?;
        let mut current = self.current.clone();
        let generation = current
            .borrow_and_update()
            .clone()
            .ok_or(ProbeError::Unavailable)?;
        if !self.current(&generation) {
            return Err(ProbeError::Unavailable);
        }
        let resolved = self.resolve(&request.peer, &generation);
        if !self.current(&generation) {
            return Err(ProbeError::Unavailable);
        }
        let (peer, name) = resolved?;
        let runtime = generation
            .runtime
            .upgrade()
            .ok_or(ProbeError::Unavailable)?;
        let deadline = Instant::now() + Duration::from_millis(request.timeout_ms);
        let probe = async {
            // A background probe can temporarily occupy the single channel slot.
            // At most 20 short retries; the total request deadline also bounds it.
            for attempt in 0..=20 {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(ProbeError::Timeout);
                }
                match runtime.probe_neighbor_observed(&peer, remaining).await {
                    Ok(proof) => {
                        let described = session::describe(&generation.intent, &proof.connection)
                            .ok_or(ProbeError::Unavailable)?;
                        return Ok(ProbeObservation {
                            generation: generation.number,
                            installed_revision: generation.installed_revision,
                            source_revision: generation.source_revision,
                            evidence: Evidence::DirectAuthenticatedNeighborRoundTrip,
                            peer_node_id: described.peer_node_id,
                            peer_name: name,
                            local_node_id: generation.intent.node_id.clone(),
                            initiator_node_id: described.initiator_node_id,
                            transport: described.transport,
                            // Preserve sub-microsecond precision without formatting secrets.
                            rtt_ns: proof.round_trip.as_nanos().min(u64::MAX as u128) as u64,
                        });
                    }
                    Err(meshway_dataplane::Error::Capacity) if attempt < 20 => {
                        time::sleep_until(deadline.min(Instant::now() + Duration::from_millis(25)))
                            .await;
                    }
                    Err(meshway_dataplane::Error::Capacity) => return Err(ProbeError::Busy),
                    Err(meshway_dataplane::Error::Timeout) => return Err(ProbeError::Timeout),
                    Err(meshway_dataplane::Error::Authentication) => {
                        return Err(ProbeError::AuthenticationFailed);
                    }
                    Err(_) => return Err(ProbeError::ChannelUnavailable),
                }
            }
            Err(ProbeError::Busy)
        };
        let result = tokio::select! {
            biased;
            _ = generation.invalidated() => Err(ProbeError::Unavailable),
            _ = current.changed() => Err(ProbeError::Unavailable),
            result = probe => result,
        };
        if !self.current(&generation) {
            return Err(ProbeError::Unavailable);
        }
        result
    }
}

#[cfg(test)]
mod live_tests {
    use super::super::tests::{fixture, session, snapshot};
    use super::*;
    use crate::diagnostics;
    use std::os::unix::fs::PermissionsExt;

    fn inventory(own: &str, peer: &str) -> NodeConfiguration {
        let node = |id: &str, name: &str, index| meshway_protocol::MeshNode {
            id: id.into(),
            name: name.into(),
            country: None,
            mesh_ipv4: format!("100.64.0.{index}"),
            mesh_ipv6: format!("fd00::{index}"),
        };
        NodeConfiguration {
            schema_version: 1,
            revision: 1,
            network: meshway_protocol::MeshNetwork {
                ipv4_prefix: "100.64.0.0/24".into(),
                ipv6_prefix: "fd00::/64".into(),
            },
            node: node(own, "LAX", 1),
            peers: vec![node(peer, "SHA", 2), node(&"c".repeat(64), "NYC", 3)],
            routing_policy: Default::default(),
            policy_valid: true,
            diagnostics: vec![],
        }
    }

    #[tokio::test]
    async fn live_uds_registry_inventory_ambiguity_revision_revocation_and_lease() {
        let (server, key, client, remote_key) =
            fixture(meshway_protocol::ConnectionTransport::NativeUdp);
        let mut manager = Manager::new(None, Duration::from_secs(30));
        let mut names = inventory(&server.node_id, &client.node_id);
        manager.set_probe_inventory(&names);
        manager
            .reconcile(snapshot(server.clone()), key.clone(), Instant::now())
            .await;
        let mut remote = session(&client, remote_key, None).await;
        let prober = manager.prober();
        time::timeout(Duration::from_secs(8), async {
            loop {
                if prober
                    .live_status(true, 0, 1)
                    .unwrap()
                    .authenticated_channel_count
                    == 1
                {
                    break;
                }
                time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let unchanged = prober.generation().unwrap();
        manager.set_probe_inventory(&names);
        manager
            .reconcile(snapshot(server.clone()), key.clone(), Instant::now())
            .await;
        assert!(
            prober.current(&unchanged),
            "an identical fresh sync renews the lease without cancelling probes"
        );
        let dir = tempfile::Builder::new()
            .prefix("mw")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let state = dir.path().join("a");
        let lock = meshway_protocol::local::lock(&dir.path().join("a.lock")).unwrap();
        let socket = diagnostics::Server::bind(&state, &lock).await.unwrap();
        let serving = prober.clone();
        let task = tokio::spawn(async move { socket.serve(serving).await });
        let live = diagnostics::live_status(&state, true, 0, 1).await.unwrap();
        assert_eq!(live.installed_revision, 1);
        assert_eq!(live.local_identity.unwrap().mesh_ipv4, "100.64.0.1");
        assert!(live.lease_remaining_ms > 0);
        let peer = &live.peers.unwrap()[0];
        assert_eq!(peer.initiator_node_id, client.node_id);
        assert_eq!(
            peer.transport,
            meshway_protocol::ConnectionTransport::NativeUdp
        );
        assert!(prober.live_status(false, 0, 64).unwrap().peers.is_none());
        assert_eq!(
            prober.live_status(true, 2, 64).unwrap_err(),
            ProbeError::InvalidRequest
        );
        assert_eq!(
            prober.live_status(true, 0, 65).unwrap_err(),
            ProbeError::InvalidRequest
        );
        for query in [&client.node_id, "SHA", "100.64.0.2", "fd00:0:0:0:0:0:0:2"] {
            let found = diagnostics::whois(&state, query).await.unwrap();
            assert_eq!(found.identity.node_id, client.node_id);
            assert_eq!(found.generation, live.generation);
        }
        assert_eq!(
            prober.whois("unknown").unwrap_err(),
            ProbeError::UnknownIdentity
        );
        assert_eq!(
            prober.whois("bbbb").unwrap_err(),
            ProbeError::UnknownIdentity
        );
        // Inventory identities are broader than the direct-neighbor probe grant.
        assert_eq!(
            prober.whois("NYC").unwrap().identity.node_id,
            "c".repeat(64)
        );
        assert_eq!(
            prober
                .probe_neighbor_observed(ProbeRequest {
                    peer: "NYC".into(),
                    timeout_ms: 1000
                })
                .await
                .unwrap_err(),
            ProbeError::UnconfiguredPeer
        );
        time::sleep(Duration::from_secs(1)).await; // bounded server request rate
        names.node.name = "SHA".into();
        let old = prober.generation().unwrap();
        manager.set_probe_inventory(&names);
        assert!(
            !prober.current(&old),
            "metadata replacement fences old reads"
        );
        assert_eq!(
            diagnostics::whois(&state, "SHA").await.unwrap_err(),
            ProbeError::AmbiguousPeer
        );
        assert_eq!(
            diagnostics::whois(&state, &client.node_id)
                .await
                .unwrap()
                .identity
                .node_id,
            client.node_id
        );

        // A newer trusted inventory cannot be attached to an older runtime.
        names.revision = 2;
        names.node.name = "LAX".into();
        names.peers.retain(|node| node.id != client.node_id);
        manager.set_probe_inventory(&names);
        assert!(
            prober
                .live_status(false, 0, 64)
                .unwrap()
                .local_identity
                .is_none()
        );
        assert_eq!(
            prober.whois(&client.node_id).unwrap_err(),
            ProbeError::Unavailable
        );
        let mut renewed = server.clone();
        renewed.revision = 2;
        manager
            .reconcile(snapshot(renewed), key, Instant::now())
            .await;
        let renewed = prober.live_status(false, 0, 64).unwrap();
        assert_eq!(
            renewed.installed_revision, 1,
            "identical material retains the original runtime installation"
        );
        assert_eq!(renewed.source_revision, 2);
        assert_eq!(
            prober.whois(&client.node_id).unwrap_err(),
            ProbeError::UnknownIdentity
        );
        assert_eq!(prober.whois("LAX").unwrap().source_revision, 2);
        manager
            .running
            .as_ref()
            .unwrap()
            .deadline
            .send(Some(Instant::now()))
            .unwrap();
        assert_eq!(prober.whois("LAX").unwrap_err(), ProbeError::Unavailable);
        assert_eq!(
            prober.live_status(true, 0, 64).unwrap_err(),
            ProbeError::Unavailable
        );
        manager.withdraw(super::super::Reason::ShuttingDown).await;
        assert_eq!(
            diagnostics::live_status(&state, false, 0, 64)
                .await
                .unwrap_err(),
            ProbeError::Unavailable
        );
        task.abort();
        let _ = task.await;
        remote.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_metadata_replacement_never_splices_revision_and_identity() {
        let (server, key, _, _) = fixture(meshway_protocol::ConnectionTransport::NativeUdp);
        let mut manager = Manager::new(None, Duration::from_secs(30));
        let mut names = inventory(&server.node_id, &server.peers[0].node_id);
        names.node.name = "revision-1".into();
        manager.set_probe_inventory(&names);
        manager
            .reconcile(snapshot(server.clone()), key.clone(), Instant::now())
            .await;
        let reader = manager.prober();
        let own_id = server.node_id.clone();
        let reads = tokio::spawn(async move {
            for _ in 0..10_000 {
                if let Ok(status) = reader.live_status(false, 0, 64)
                    && let Some(identity) = status.local_identity
                {
                    assert_eq!(
                        identity.name,
                        format!("revision-{}", status.source_revision)
                    );
                }
                if let Ok(found) = reader.whois(&own_id) {
                    assert_eq!(
                        found.identity.name,
                        format!("revision-{}", found.source_revision)
                    );
                }
                tokio::task::yield_now().await;
            }
        });
        for revision in 2..=20 {
            names.revision = revision;
            names.node.name = format!("revision-{revision}");
            manager.set_probe_inventory(&names);
            let mut renewed = server.clone();
            renewed.revision = revision;
            manager
                .reconcile(snapshot(renewed), key.clone(), Instant::now())
                .await;
        }
        reads.await.unwrap();
        manager.withdraw(super::super::Reason::ShuttingDown).await;
    }
}
