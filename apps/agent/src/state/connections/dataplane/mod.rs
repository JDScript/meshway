//! Current execution journal and normal-run orchestration. Inventory is not
//! authorization; only a fresh, coherent execution snapshot can start packets.
use super::{Agent, AgentError, convert, local};
use crate::managed_connections::Manager;
use meshway_protocol::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
use tokio::time::Instant;

pub(crate) struct State {
    cache: Option<Cache>,
}

pub(crate) struct Synced {
    pub receipt: super::ControlSyncReceipts,
    pub snapshot: MeshExecutionSnapshot,
    pub fresh_at: Instant,
}

impl State {
    pub fn load(agent: &mut Agent) -> Result<Self, AgentError> {
        agent.load_connections()?;
        let mut obsolete = agent.path.as_os_str().to_owned();
        obsolete.push(".dataplane.json");
        match std::fs::symlink_metadata(PathBuf::from(obsolete)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
            Ok(_) => {
                return Err(AgentError::Invalid(
                    "unsupported old dataplane cache; explicit cleanup required",
                ));
            }
        }
        Ok(Self {
            cache: load(agent)?,
        })
    }

    pub fn has_cache(&self) -> bool {
        self.cache.is_some()
    }

    fn check(&self, agent: &Agent) -> Result<(), AgentError> {
        agent
            .connections
            .as_ref()
            .expect("loaded connections")
            .check(&agent.path, &agent.identity)?;
        if load(agent)?.as_ref().map(|c| &c.snapshot.digest)
            != self.cache.as_ref().map(|c| &c.snapshot.digest)
        {
            return Err(AgentError::Invalid("execution journal changed during sync"));
        }
        Ok(())
    }

    fn persist(
        &mut self,
        agent: &Agent,
        snapshot: MeshExecutionSnapshot,
    ) -> Result<(), AgentError> {
        self.check(agent)?;
        let cache = Cache {
            format_version: 1,
            controller: agent.identity.controller.clone(),
            node_id: snapshot.payload.node_id.clone(),
            credential_hash: local::digest(&agent.identity.node_credential),
            snapshot,
        };
        let bytes = zeroize::Zeroizing::new(
            serde_json::to_vec(&cache)
                .map_err(|_| AgentError::Invalid("cannot encode Mesh journal"))?,
        );
        let result = local::atomic_write(&path(agent), &bytes, self.cache.is_some());
        if result.is_ok() || result.as_ref().is_err_and(|error| error.committed()) {
            self.cache = Some(cache);
        }
        result?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    format_version: u32,
    controller: String,
    node_id: String,
    credential_hash: Vec<u8>,
    snapshot: MeshExecutionSnapshot,
}

fn path(agent: &Agent) -> PathBuf {
    let mut path = agent.path.as_os_str().to_owned();
    path.push(".mesh-execution.json");
    PathBuf::from(path)
}

fn load(agent: &Agent) -> Result<Option<Cache>, AgentError> {
    let path = path(agent);
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
        Ok(_) => {}
    }
    let encoded = zeroize::Zeroizing::new(local::read_private_bounded(
        &path,
        MAX_DATAPLANE_BYTES + 64 * 1024,
    )?);
    let cache: Cache = serde_json::from_str(&encoded)
        .map_err(|_| AgentError::Invalid("invalid Mesh execution journal"))?;
    if cache.format_version != 1
        || cache.controller != agent.identity.controller
        || Some(&cache.node_id) != agent.identity.node_id.as_ref()
        || cache.credential_hash != local::digest(&agent.identity.node_credential)
    {
        return Err(AgentError::Invalid(
            "Mesh execution journal identity mismatch",
        ));
    }
    // A journal is historical evidence, not an incoming publication. Startup
    // inventory may already be newer; its revision must not invalidate this
    // file. Its intrinsic integrity and immutable bindings still must hold.
    validate_snapshot(agent, &cache.snapshot)?;
    Ok(Some(cache))
}

#[cfg(target_os = "linux")]
pub(super) fn cached_network(agent: &Agent) -> Result<Option<String>, AgentError> {
    Ok(load(agent)?.map(|cache| cache.snapshot.payload.network_id))
}

fn validate(
    agent: &Agent,
    snapshot: &MeshExecutionSnapshot,
    old: Option<&MeshExecutionSnapshot>,
) -> Result<(), AgentError> {
    validate_snapshot(agent, snapshot)?;
    let p = &snapshot.payload;
    let connections = AgentConnectionsSnapshot {
        digest: connection_digest(&p.connections)
            .map_err(|_| AgentError::Invalid("cannot hash Mesh adjacency"))?,
        payload: p.connections.clone(),
    };
    super::validation::validate(
        &connections,
        &agent.identity,
        agent
            .connections
            .as_ref()
            .and_then(|s| s.keys.public_key())
            .ok_or(AgentError::Invalid("missing Mesh execution private key"))?,
        agent
            .connections
            .as_ref()
            .and_then(|s| s.cache.as_ref().map(|c| &c.snapshot)),
    )?;
    if let Some(cache) = &agent.cache {
        let inventory = cache.public_inventory();
        if p.revision < inventory.revision
            || (p.revision == inventory.revision
                && (p.node != inventory.node || p.peers != inventory.peers))
        {
            return Err(AgentError::Invalid(
                "Mesh execution predates or mutates saved inventory",
            ));
        }
    }
    if let Some(old) = old {
        let o = &old.payload;
        if p.revision < o.revision
            || (p.revision == o.revision && snapshot.digest != old.digest)
            || p.network_id != o.network_id
            || p.network != o.network
            || p.node.mesh_ipv4 != o.node.mesh_ipv4
            || p.node.mesh_ipv6 != o.node.mesh_ipv6
            || p.identities.iter().any(|n| {
                o.identities.iter().any(|old| {
                    (n.node_id == old.node_id && n.public_key != old.public_key)
                        || (n.public_key == old.public_key && n.node_id != old.node_id)
                })
            })
        {
            return Err(AgentError::Invalid(
                "Mesh execution rollback or immutable binding changed",
            ));
        }
    }
    Ok(())
}

fn validate_snapshot(agent: &Agent, snapshot: &MeshExecutionSnapshot) -> Result<(), AgentError> {
    let p = &snapshot.payload;
    if p.schema_version != MESH_EXECUTION_VERSION
        || p.required_capability != MeshExecutionCapability::Ipv4ExactPathV1
        || p.revision == 0
        || p.network_id.len() != 32
        || !p
            .network_id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || Some(&p.node_id) != agent.identity.node_id.as_ref()
        || p.node.id != p.node_id
        || p.node.name != agent.identity.name
        || p.connections.node_id != p.node_id
        || p.connections.revision != p.revision
        || p.paths.len() > MAX_MESH_EXECUTION_PATHS
        || p.identities.len() > MAX_DATAPLANE_IDENTITIES
        || p.peers.len() >= 1024
        || p.materials.len() > MAX_MESH_EXECUTION_MATERIALS
        || (!p.valid && !p.paths.is_empty())
        || mesh_execution_digest(p)
            .map_err(|_| AgentError::Invalid("cannot hash Mesh execution"))?
            != snapshot.digest
        || serde_json::to_vec(snapshot)
            .map_err(|_| AgentError::Invalid("cannot encode Mesh execution"))?
            .len()
            > MAX_DATAPLANE_BYTES
    {
        return Err(AgentError::Invalid(
            "invalid Mesh execution version, binding, digest or budget",
        ));
    }
    if let Some(cache) = &agent.cache {
        let inventory = cache.public_inventory();
        if p.network != inventory.network
            || p.node.mesh_ipv4 != inventory.node.mesh_ipv4
            || p.node.mesh_ipv6 != inventory.node.mesh_ipv6
        {
            return Err(AgentError::Invalid(
                "Mesh journal immutable address binding changed",
            ));
        }
    }
    if let Some(cache) = agent.connections.as_ref().and_then(|s| s.cache.as_ref())
        && p.identities.iter().any(|peer| {
            cache.snapshot.payload.peers.iter().any(|other| {
                (peer.node_id == other.node_id && peer.public_key != other.public_key)
                    || (peer.public_key == other.public_key && peer.node_id != other.node_id)
            })
        })
    {
        return Err(AgentError::Invalid(
            "Mesh journal immutable key binding changed",
        ));
    }
    let prefix: ipnet::Ipv4Net = p
        .network
        .ipv4_prefix
        .parse()
        .map_err(|_| AgentError::Invalid("invalid Mesh IPv4 pool"))?;
    let v6: ipnet::Ipv6Net = p
        .network
        .ipv6_prefix
        .parse()
        .map_err(|_| AgentError::Invalid("invalid inventory IPv6 pool"))?;
    if prefix.prefix_len() > 30
        || prefix.addr() != prefix.network()
        || prefix.to_string() != p.network.ipv4_prefix
        || v6.prefix_len() != 64
        || v6.addr() != v6.network()
        || v6.to_string() != p.network.ipv6_prefix
        || u128::from(v6.addr()) >> 120 != 0xfd
        || p.peers.windows(2).any(|n| n[0].id >= n[1].id)
    {
        return Err(AgentError::Invalid(
            "invalid Mesh inventory network or order",
        ));
    }
    let mut nodes = BTreeMap::new();
    let mut ips = BTreeSet::new();
    for n in std::iter::once(&p.node).chain(&p.peers) {
        let ip: std::net::Ipv4Addr = n
            .mesh_ipv4
            .parse()
            .map_err(|_| AgentError::Invalid("invalid Mesh IPv4 binding"))?;
        let ip6: std::net::Ipv6Addr = n
            .mesh_ipv6
            .parse()
            .map_err(|_| AgentError::Invalid("invalid inventory IPv6 binding"))?;
        if !local::valid_credential(&n.id)
            || !super::super::valid_name(&n.name)
            || n.country
                .as_ref()
                .is_some_and(|c| c.len() != 2 || !c.bytes().all(|b| b.is_ascii_uppercase()))
            || !prefix.contains(&ip)
            || ip == prefix.network()
            || ip == prefix.broadcast()
            || ip.to_string() != n.mesh_ipv4
            || ip6.to_string() != n.mesh_ipv6
            || u128::from(ip6)
                != u128::from(v6.network())
                    + u128::from(u32::from(ip) - u32::from(prefix.network()))
            || !ips.insert(ip)
            || nodes.insert(n.id.as_str(), ip).is_some()
        {
            return Err(AgentError::Invalid(
                "invalid or duplicate Mesh inventory address/identity",
            ));
        }
    }
    let own_key = agent
        .connections
        .as_ref()
        .and_then(|s| s.keys.public_key())
        .ok_or(AgentError::Invalid("missing Mesh execution private key"))?;
    let connections = AgentConnectionsSnapshot {
        digest: connection_digest(&p.connections)
            .map_err(|_| AgentError::Invalid("cannot hash Mesh adjacency"))?,
        payload: p.connections.clone(),
    };
    super::validation::validate(&connections, &agent.identity, own_key, None)?;
    let mut unique_paths = BTreeSet::new();
    let mut outgoing = BTreeSet::new();
    for path in &p.paths {
        let path = &path.nodes;
        if !(2..=8).contains(&path.len())
            || !path.contains(&p.node_id)
            || path.iter().collect::<BTreeSet<_>>().len() != path.len()
            || path.iter().any(|id| !nodes.contains_key(id.as_str()))
            || !unique_paths.insert(path)
            || (path.first() == Some(&p.node_id) && !outgoing.insert(path.last()))
        {
            return Err(AgentError::Invalid(
                "invalid, ambiguous or nonlocal exact Mesh path",
            ));
        }
        let i = path
            .iter()
            .position(|id| id == &p.node_id)
            .expect("checked local");
        for neighbor in i
            .checked_sub(1)
            .into_iter()
            .chain((i + 1 < path.len()).then_some(i + 1))
        {
            if !p
                .connections
                .peers
                .iter()
                .any(|peer| peer.node_id == path[neighbor])
            {
                return Err(AgentError::Invalid(
                    "Mesh path local hop missing physical candidate",
                ));
            }
        }
    }
    let required: BTreeSet<_> = p
        .connections
        .peers
        .iter()
        .map(|n| n.node_id.as_str())
        .chain(
            p.paths
                .iter()
                .flat_map(|p| p.nodes.iter().map(String::as_str)),
        )
        .filter(|id| *id != p.node_id)
        .collect();
    let mut keys = BTreeSet::from([own_key]);
    let mut identities = BTreeMap::new();
    for n in &p.identities {
        if !nodes.contains_key(n.node_id.as_str())
            || n.node_id == p.node_id
            || !valid_connection_public_key(&n.public_key)
            || !keys.insert(n.public_key.as_str())
            || identities
                .insert(n.node_id.as_str(), n.public_key.as_str())
                .is_some()
        {
            return Err(AgentError::Invalid("invalid Mesh execution identities"));
        }
    }
    if p.identities
        .windows(2)
        .any(|n| n[0].node_id >= n[1].node_id)
        || identities.keys().copied().collect::<BTreeSet<_>>() != required
        || p.connections
            .peers
            .iter()
            .any(|n| identities.get(n.node_id.as_str()).copied() != Some(n.public_key.as_str()))
    {
        return Err(AgentError::Invalid(
            "Mesh identities differ from exact authorization scope",
        ));
    }
    Ok(())
}

impl State {
    pub(crate) async fn sync(
        &mut self,
        agent: &mut Agent,
        manager: &mut Manager,
        runtime: &MeshExecutionRequest,
    ) -> Result<Synced, AgentError> {
        let publication = agent.publish_connection_identity().await?;
        manager
            .invalidate_dataplane_revision(publication.revision)
            .await;
        let client = agent
            .controller_client()
            .with_api_key(&agent.identity.node_credential)
            .with_max_response_body_bytes(MAX_DATAPLANE_BYTES);
        for _ in 0..3 {
            let response = client.negotiate_mesh_execution(convert(runtime)?).await;
            let response: MeshExecutionResponse = match response {
                Err(error) if error.api().is_some_and(|error| error.status == 409) => {
                    // Negotiation has no source CAS: conflict means the epoch /
                    // instance no longer owns execution. Never reacquire it.
                    manager
                        .withdraw(ConnectionRuntimeReason::LeaseSuperseded)
                        .await;
                    return Err(AgentError::Invalid(
                        "execution runtime lease superseded; refusing to reacquire",
                    ));
                }
                Err(error) => return Err(error.into()),
                Ok(response) => convert(response)?,
            };
            manager
                .invalidate_dataplane_revision(response.revision)
                .await;
            let snapshot = response.snapshot;
            if response.revision != snapshot.payload.revision {
                return Err(AgentError::Invalid("Mesh execution revision mismatch"));
            }
            let fresh_at = Instant::now();
            self.check(agent)?;
            validate(agent, &snapshot, self.cache.as_ref().map(|c| &c.snapshot))?;
            if snapshot.payload.revision
                < agent
                    .connections
                    .as_ref()
                    .expect("published connections")
                    .published_revision
                || snapshot.payload.connections.capabilities
                    != super::compiled_connection_capabilities()
            {
                return Err(AgentError::Invalid(
                    "Mesh execution predates key publication or differs from compiled capabilities",
                ));
            }
            if let Some(guard) = &agent.dns_guard {
                guard.observe_revision(snapshot.payload.revision);
            }
            manager.invalidate_dataplane(&snapshot).await;
            manager
                .validate_materials(&snapshot.payload.connections, &snapshot.payload.materials)?;
            let p = &snapshot.payload;
            let journal_digest = managed_digest(&p.network_id, &snapshot, &p.materials)
                .map_err(|_| AgentError::Invalid("cannot hash Mesh material journal"))?;
            super::materials::persist(
                agent,
                &p.network_id,
                p.revision,
                &snapshot,
                &p.materials,
                &journal_digest,
            )?;
            manager.install_materials(p.materials.clone());
            let connections = AgentConnectionsSnapshot {
                digest: connection_digest(&p.connections)
                    .map_err(|_| AgentError::Invalid("cannot hash Mesh adjacency"))?,
                payload: p.connections.clone(),
            };
            self.persist(agent, snapshot.clone())?;
            agent
                .connections
                .as_mut()
                .expect("published connections")
                .persist(&agent.path, &agent.identity, connections.clone())?;
            local::ensure_durable(&path(agent))?;
            local::ensure_durable(&super::cache_path(&agent.path))?;
            local::ensure_durable(&super::super::connection_key::path(&agent.path))?;
            manager.prepare_dataplane_sink(&snapshot).await?;
            // This independently fetched inventory must agree at the same revision.
            // It supplies DNS/status, never packet paths or authorization.
            let inventory: ConfigurationSnapshot =
                convert(client.get_agent_configuration().await?)?;
            super::super::configuration::validate(
                &inventory,
                &agent.identity,
                agent.cache.as_ref(),
            )?;
            manager
                .invalidate_dataplane_revision(inventory.payload.revision)
                .await;
            if let Some(guard) = &agent.dns_guard {
                guard.observe_revision(inventory.payload.revision);
            }
            if inventory.payload.revision != p.revision {
                manager
                    .withdraw(ConnectionRuntimeReason::InvalidConfiguration)
                    .await;
                continue;
            }
            if inventory.payload.network != p.network
                || inventory.payload.node != p.node
                || inventory.payload.peers != p.peers
            {
                return Err(AgentError::Invalid(
                    "Mesh execution differs from same-revision inventory",
                ));
            }
            agent.persist_configuration(inventory.clone())?;
            local::ensure_durable(&super::super::configuration::cache_path(&agent.path))?;
            let ack = client
                .acknowledge_configuration(convert(AcknowledgeConfigurationRequest {
                    revision: p.revision,
                    digest: inventory.digest.clone(),
                })?)
                .await;
            let ack: AcknowledgeConfigurationResponse = match ack {
                Err(e) if e.api().is_some_and(|e| e.status == 409) => {
                    if let Some(guard) = &agent.dns_guard {
                        guard.conflict(p.revision);
                    }
                    manager
                        .withdraw(ConnectionRuntimeReason::InvalidConfiguration)
                        .await;
                    continue;
                }
                Err(e) => return Err(e.into()),
                Ok(value) => convert(value)?,
            };
            if ack.revision != p.revision || ack.digest != inventory.digest {
                return Err(AgentError::Invalid("Mesh inventory ACK mismatch"));
            }
            let configuration = super::SyncReceipt {
                controller: agent.identity.controller.clone(),
                node_id: p.node_id.clone(),
                stored_revision: p.revision,
                digest: inventory.digest,
                mesh_ipv4: p.node.mesh_ipv4.clone(),
                mesh_ipv6: p.node.mesh_ipv6.clone(),
            };
            #[cfg(unix)]
            manager.set_probe_inventory(
                agent
                    .cache
                    .as_ref()
                    .expect("saved inventory")
                    .public_inventory(),
            );
            let ack = client
                .acknowledge_mesh_execution(convert(AcknowledgeMeshExecutionRequest {
                    runtime: runtime.clone(),
                    revision: p.revision,
                    digest: snapshot.digest.clone(),
                })?)
                .await;
            let ack: AcknowledgeMeshExecutionResponse = match ack {
                Err(e) if e.api().is_some_and(|e| e.status == 409) => {
                    if let Some(guard) = &agent.dns_guard {
                        guard.conflict(p.revision);
                    }
                    manager
                        .withdraw(ConnectionRuntimeReason::InvalidConfiguration)
                        .await;
                    continue;
                }
                Err(e) => return Err(e.into()),
                Ok(value) => convert(value)?,
            };
            if ack.revision != p.revision || ack.digest != snapshot.digest || !ack.saved {
                return Err(AgentError::Invalid("Mesh saved ACK mismatch"));
            }
            let ack = client
                .acknowledge_agent_connections(convert(
                    meshway_protocol::AcknowledgeAgentConnectionsRequest {
                        revision: p.revision,
                        digest: connections.digest.clone(),
                    },
                )?)
                .await;
            let ack: AcknowledgeAgentConnectionsResponse = match ack {
                Err(e) if e.api().is_some_and(|e| e.status == 409) => {
                    if let Some(guard) = &agent.dns_guard {
                        guard.conflict(p.revision);
                    }
                    manager
                        .withdraw(ConnectionRuntimeReason::InvalidConfiguration)
                        .await;
                    continue;
                }
                Err(e) => return Err(e.into()),
                Ok(value) => convert(value)?,
            };
            if ack.revision != p.revision
                || ack.digest != connections.digest
                || ack.status != ConnectionReceiptStatus::Saved
            {
                return Err(AgentError::Invalid("Mesh adjacency ACK mismatch"));
            }
            self.check(agent)?;
            return Ok(Synced {
                receipt: super::ControlSyncReceipts {
                    configuration,
                    connections: super::ConnectionSyncReceipt {
                        controller: agent.identity.controller.clone(),
                        node_id: p.node_id.clone(),
                        stored_revision: p.revision,
                        digest: connections.digest,
                        configuration_valid: connections.payload.valid,
                        status: ConnectionReceiptStatus::Saved,
                        applied: false,
                    },
                },
                snapshot,
                fresh_at,
            });
        }
        Err(AgentError::Http {
            status: 409,
            retryable: true,
        })
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod execution_tests {
    use super::*;

    fn fixture(directory: &std::path::Path) -> (Agent, MeshExecutionSnapshot) {
        let (payloads, _) = crate::managed_dataplane::tests::fixtures(2);
        let old = &payloads[0];
        let mut agent = Agent::prepare_enrollment(
            directory.join("agent.json"),
            "http://127.0.0.1",
            &"a".repeat(64),
            &old.node.name,
        )
        .unwrap();
        agent.identity.node_id = Some(old.node_id.clone());
        crate::state::write_identity(&agent.path, &agent.identity, true).unwrap();
        agent.load_connections().unwrap();
        let key = agent
            .connections
            .as_mut()
            .unwrap()
            .keys
            .prepare(&agent.path, &agent.identity)
            .unwrap()
            .to_owned();
        let mut connections = old.connections.clone();
        connections.public_key = Some(key);
        let payload = NodeMeshExecution {
            schema_version: MESH_EXECUTION_VERSION,
            network_id: "0".repeat(32),
            revision: old.revision,
            node_id: old.node_id.clone(),
            required_capability: MeshExecutionCapability::Ipv4ExactPathV1,
            network: old.network.clone(),
            node: old.node.clone(),
            peers: old.peers.clone(),
            valid: true,
            paths: old.paths.clone(),
            identities: old.identities.clone(),
            connections,
            materials: vec![],
        };
        let digest = mesh_execution_digest(&payload).unwrap();
        (agent, MeshExecutionSnapshot { digest, payload })
    }

    #[test]
    fn historical_journal_survives_newer_inventory_but_incoming_floors_remain_strict() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let (mut agent, historical) = fixture(dir.path());
        let cache = Cache {
            format_version: 1,
            controller: agent.identity.controller.clone(),
            node_id: historical.payload.node_id.clone(),
            credential_hash: local::digest(&agent.identity.node_credential),
            snapshot: historical.clone(),
        };
        local::atomic_write(&path(&agent), &serde_json::to_vec(&cache).unwrap(), false).unwrap();
        let mut fresh = historical.clone();
        fresh.payload.revision += 1;
        fresh.payload.connections.revision += 1;
        fresh.digest = mesh_execution_digest(&fresh.payload).unwrap();
        let connections = AgentConnectionsSnapshot {
            digest: connection_digest(&fresh.payload.connections).unwrap(),
            payload: fresh.payload.connections.clone(),
        };
        agent
            .connections
            .as_mut()
            .unwrap()
            .persist(&agent.path, &agent.identity, connections)
            .unwrap();
        let loaded = load(&agent).unwrap().unwrap();
        assert_eq!(loaded.snapshot.digest, historical.digest);
        validate(&agent, &fresh, Some(&loaded.snapshot)).unwrap();
        validate(&agent, &fresh, Some(&fresh)).unwrap(); // Same revision restart.
        assert!(validate(&agent, &historical, Some(&historical)).is_err()); // Current connections floor.
        let mut changed = fresh.clone();
        changed.payload.paths.clear();
        changed.digest = mesh_execution_digest(&changed.payload).unwrap();
        assert!(validate(&agent, &changed, Some(&fresh)).is_err());
        // A history reader cannot skip the immutable binding check merely
        // because an inventory floor is newer than the journal.
        let config = NodeConfiguration {
            schema_version: CONFIGURATION_SCHEMA_VERSION,
            revision: fresh.payload.revision + 1,
            network: fresh.payload.network.clone(),
            node: fresh.payload.node.clone(),
            peers: fresh.payload.peers.clone(),
        };
        agent
            .persist_configuration(ConfigurationSnapshot {
                digest: configuration_digest(&config).unwrap(),
                payload: config,
            })
            .unwrap();
        assert!(load(&agent).is_ok());
        assert!(validate(&agent, &fresh, Some(&historical)).is_err()); // Inventory-only higher floor.
        let mut corrupt = cache;
        corrupt.snapshot.payload.node.mesh_ipv4 = "100.64.0.20".into();
        corrupt.snapshot.digest = mesh_execution_digest(&corrupt.snapshot.payload).unwrap();
        local::atomic_write(&path(&agent), &serde_json::to_vec(&corrupt).unwrap(), true).unwrap();
        assert!(load(&agent).is_err());
    }

    #[test]
    fn defensive_admission_rejects_forged_scope_version_budget_and_rollback() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let (agent, snapshot) = fixture(dir.path());
        validate(&agent, &snapshot, None).unwrap();
        crate::managed_dataplane::PacketPlan::compile(&snapshot.payload, 1000).unwrap();
        for mode in 0..10 {
            let mut bad = snapshot.clone();
            match mode {
                0 => bad.payload.schema_version = 2,
                1 => bad.payload.revision += 1,
                2 => bad.payload.node_id = "f".repeat(64),
                3 => {
                    bad.payload.identities.clear();
                }
                4 => bad.payload.valid = false,
                5 => bad.payload.paths[0].nodes.push(bad.payload.node_id.clone()),
                6 => bad.payload.paths.push(bad.payload.paths[0].clone()),
                7 => bad.payload.node.mesh_ipv4 = bad.payload.peers[0].mesh_ipv4.clone(),
                8 => bad.payload.connections.peers.clear(),
                9 => {
                    bad.payload.paths =
                        vec![bad.payload.paths[0].clone(); MAX_MESH_EXECUTION_PATHS + 1]
                }
                _ => unreachable!(),
            }
            bad.digest = mesh_execution_digest(&bad.payload).unwrap();
            assert!(validate(&agent, &bad, None).is_err(), "forgery {mode}");
        }
        let mut bad = snapshot.clone();
        bad.payload.revision -= 1;
        bad.payload.connections.revision -= 1;
        bad.digest = mesh_execution_digest(&bad.payload).unwrap();
        assert!(validate(&agent, &bad, Some(&snapshot)).is_err());
        let mut bad = snapshot.clone();
        bad.payload.paths.clear();
        bad.digest = mesh_execution_digest(&bad.payload).unwrap();
        assert!(validate(&agent, &bad, Some(&snapshot)).is_err());
    }

    #[test]
    fn real_generated_contract_requires_snapshot_rejects_null_and_unknowns() {
        use meshway_control_client::types;
        for json in [
            serde_json::json!({"revision": 1}),
            serde_json::json!({"revision": 1, "snapshot": null}),
        ] {
            assert!(serde_json::from_value::<types::MeshExecutionResponse>(json.clone()).is_err());
            assert!(serde_json::from_value::<MeshExecutionResponse>(json).is_err());
        }
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let (_, snapshot) = fixture(dir.path());
        let mut json = serde_json::to_value(MeshExecutionResponse {
            revision: snapshot.payload.revision,
            snapshot,
        })
        .unwrap();
        let generated: types::MeshExecutionResponse = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(generated).unwrap(), json);
        json["snapshot"]["payload"]["paths"][0]["injected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<types::MeshExecutionResponse>(json).is_err());
    }
}
