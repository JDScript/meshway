//! One coherent, generated-HTTP bundle is the sole packet authorization source.
//! Disk caches provide a rollback/binding floor only; they never start a runtime.

use meshway_protocol::{
    AcknowledgeAgentConnectionsResponse, AgentConnectionsSnapshot, AgentDataplaneSnapshot,
    ConfigurationSnapshot, ConnectionReceiptStatus, ConnectionRuntimeReason,
    DATAPLANE_SCHEMA_VERSION, MAX_DATAPLANE_BYTES, MAX_DATAPLANE_IDENTITIES, configuration_digest,
    connection_digest, dataplane_digest, valid_connection_public_key,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use tokio::time::Instant;

use super::super::{configuration, connection_key};
use super::{
    Agent, AgentError, ConnectionSyncReceipt, ControlSyncReceipts, Identity, convert, local,
};
use crate::{
    generated::{
        client::HttpClient,
        types::{AcknowledgeAgentConnectionsRequest, AcknowledgeConfigurationRequest},
    },
    managed_connections::Manager,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    format_version: u32,
    controller: String,
    node_id: String,
    credential_hash: Vec<u8>,
    snapshot: AgentDataplaneSnapshot,
}

pub(super) struct State {
    cache: Option<Cache>,
}

pub(super) struct Synced {
    pub receipt: ControlSyncReceipts,
    pub snapshot: AgentDataplaneSnapshot,
    pub fresh_at: Instant,
}

fn path(state: &Path) -> PathBuf {
    let mut path = state.as_os_str().to_owned();
    path.push(".dataplane.json");
    PathBuf::from(path)
}

fn projections(
    snapshot: &AgentDataplaneSnapshot,
) -> Result<(ConfigurationSnapshot, AgentConnectionsSnapshot), AgentError> {
    Ok((
        ConfigurationSnapshot {
            digest: configuration_digest(&snapshot.payload.configuration)
                .map_err(|_| AgentError::Invalid("cannot hash bundled configuration"))?,
            payload: snapshot.payload.configuration.clone(),
        },
        AgentConnectionsSnapshot {
            digest: connection_digest(&snapshot.payload.connections)
                .map_err(|_| AgentError::Invalid("cannot hash bundled connections"))?,
            payload: snapshot.payload.connections.clone(),
        },
    ))
}

fn validate(
    snapshot: &AgentDataplaneSnapshot,
    identity: &Identity,
    own_key: &str,
    previous: Option<&AgentDataplaneSnapshot>,
) -> Result<(), AgentError> {
    let payload = &snapshot.payload;
    if serde_json::to_vec(snapshot)
        .map_err(|_| AgentError::Invalid("cannot encode dataplane snapshot"))?
        .len()
        > MAX_DATAPLANE_BYTES
        || payload.schema_version != DATAPLANE_SCHEMA_VERSION
        || payload.revision == 0
        || Some(&payload.node_id) != identity.node_id.as_ref()
        || payload.configuration.revision != payload.revision
        || payload.connections.revision != payload.revision
        || dataplane_digest(payload)
            .map_err(|_| AgentError::Invalid("cannot hash dataplane snapshot"))?
            != snapshot.digest
        || payload.identities.len() > MAX_DATAPLANE_IDENTITIES
        || payload
            .identities
            .windows(2)
            .any(|peers| peers[0].node_id >= peers[1].node_id)
        || (!payload.access_valid && !payload.access_policy.grants.is_empty())
    {
        return Err(AgentError::Invalid(
            "invalid coherent dataplane schema, digest, revision or binding",
        ));
    }
    let (configuration, connections) = projections(snapshot)?;
    configuration::validate(&configuration, identity, None)?;
    let old_connections = previous
        .map(projections)
        .transpose()?
        .map(|(_, connections)| connections);
    super::validation::validate(&connections, identity, own_key, old_connections.as_ref())?;
    if let Some(previous) = previous {
        let old = &previous.payload;
        if payload.revision < old.revision
            || (payload.revision == old.revision && snapshot.digest != previous.digest)
            || payload.configuration.network != old.configuration.network
            || payload.configuration.node.mesh_ipv4 != old.configuration.node.mesh_ipv4
            || payload.configuration.node.mesh_ipv6 != old.configuration.node.mesh_ipv6
        {
            return Err(AgentError::Invalid(
                "dataplane rollback, same-revision mutation or identity renumbering",
            ));
        }
        for peer in &payload.identities {
            if old.identities.iter().any(|old| {
                (old.node_id == peer.node_id && old.public_key != peer.public_key)
                    || (old.public_key == peer.public_key && old.node_id != peer.node_id)
            }) {
                return Err(AgentError::Invalid(
                    "dataplane peer identity binding changed",
                ));
            }
        }
    }
    let nodes: Vec<_> = std::iter::once(payload.configuration.node.clone())
        .chain(payload.configuration.peers.iter().cloned())
        .collect();
    if !meshway_policy::access::validate_mesh_path_authorization(&payload.access_policy, &nodes)
        .is_empty()
        || payload
            .access_policy
            .grants
            .iter()
            .any(|grant| !grant.path.contains(&payload.node_id))
    {
        return Err(AgentError::Invalid(
            "invalid or nonlocal mesh access projection",
        ));
    }
    let inventory: BTreeSet<_> = nodes.iter().map(|node| node.id.as_str()).collect();
    let required: BTreeSet<_> = payload
        .connections
        .peers
        .iter()
        .map(|peer| peer.node_id.as_str())
        .chain(
            payload
                .access_policy
                .grants
                .iter()
                .flat_map(|grant| grant.path.iter().map(String::as_str)),
        )
        .filter(|id| *id != payload.node_id)
        .collect();
    let mut keys = BTreeSet::from([own_key]);
    let mut identities = BTreeMap::new();
    for peer in &payload.identities {
        if peer.node_id == payload.node_id
            || !inventory.contains(peer.node_id.as_str())
            || !local::valid_credential(&peer.node_id)
            || !valid_connection_public_key(&peer.public_key)
            || !keys.insert(peer.public_key.as_str())
            || identities
                .insert(peer.node_id.as_str(), peer.public_key.as_str())
                .is_some()
        {
            return Err(AgentError::Invalid("invalid dataplane identity inventory"));
        }
    }
    if identities.keys().copied().collect::<BTreeSet<_>>() != required
        || payload.connections.peers.iter().any(|peer| {
            identities.get(peer.node_id.as_str()).copied() != Some(peer.public_key.as_str())
        })
    {
        return Err(AgentError::Invalid(
            "dataplane identity projection differs from full grant and adjacency scope",
        ));
    }
    Ok(())
}

fn load(agent: &Agent) -> Result<Option<Cache>, AgentError> {
    let path = path(&agent.path);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let cache: Cache = serde_json::from_str(&local::read_private_bounded(
        &path,
        MAX_DATAPLANE_BYTES + 64 * 1024,
    )?)
    .map_err(|_| AgentError::Invalid("invalid dataplane cache; refusing to overwrite"))?;
    if cache.format_version != 1
        || cache.controller != agent.identity.controller
        || Some(&cache.node_id) != agent.identity.node_id.as_ref()
        || cache.credential_hash != local::digest(&agent.identity.node_credential)
    {
        return Err(AgentError::Invalid(
            "dataplane cache belongs to another controller or credential",
        ));
    }
    let key = agent
        .connections
        .as_ref()
        .and_then(|state| state.keys.public_key())
        .ok_or(AgentError::Invalid(
            "dataplane cache is missing its existing connection key",
        ))?;
    validate(&cache.snapshot, &agent.identity, key, None)?;
    Ok(Some(cache))
}

impl State {
    pub fn load(agent: &mut Agent) -> Result<Self, AgentError> {
        agent.load_connections()?;
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
        if load(agent)?.as_ref().map(|cache| &cache.snapshot.digest)
            != self.cache.as_ref().map(|cache| &cache.snapshot.digest)
        {
            return Err(AgentError::Invalid("dataplane cache changed during sync"));
        }
        Ok(())
    }

    fn persist(
        &mut self,
        agent: &Agent,
        snapshot: AgentDataplaneSnapshot,
    ) -> Result<(), AgentError> {
        self.check(agent)?;
        if self
            .cache
            .as_ref()
            .is_some_and(|cache| cache.snapshot.digest == snapshot.digest)
        {
            return Ok(());
        }
        let cache = Cache {
            format_version: 1,
            controller: agent.identity.controller.clone(),
            node_id: snapshot.payload.node_id.clone(),
            credential_hash: local::digest(&agent.identity.node_credential),
            snapshot,
        };
        let bytes = serde_json::to_vec(&cache)
            .map_err(|_| AgentError::Invalid("cannot encode dataplane cache"))?;
        let result = local::atomic_write(&path(&agent.path), &bytes, self.cache.is_some());
        if result.is_ok() || result.as_ref().is_err_and(|error| error.committed()) {
            self.cache = Some(cache);
        }
        result?;
        Ok(())
    }

    pub async fn sync(
        &mut self,
        agent: &mut Agent,
        manager: &mut Manager,
    ) -> Result<Synced, AgentError> {
        let publication = agent.publish_connection_identity().await?;
        manager
            .invalidate_dataplane_revision(publication.revision)
            .await;
        let client = HttpClient::new()
            .with_base_url(&agent.identity.controller)
            .with_api_key(&agent.identity.node_credential)
            .with_max_response_body_bytes(MAX_DATAPLANE_BYTES);
        for _ in 0..3 {
            // Generated SDK operation only; no handwritten endpoint or wire DTO.
            let snapshot: AgentDataplaneSnapshot = convert(client.get_agent_dataplane().await?)?;
            let state = agent.connections.as_ref().expect("published connections");
            validate(
                &snapshot,
                &agent.identity,
                state.keys.public_key().expect("published key"),
                self.cache.as_ref().map(|cache| &cache.snapshot),
            )?;
            let (configuration, connections) = projections(&snapshot)?;
            configuration::validate(&configuration, &agent.identity, agent.cache.as_ref())?;
            super::validation::validate(
                &connections,
                &agent.identity,
                state.keys.public_key().expect("published key"),
                state.cache.as_ref().map(|cache| &cache.snapshot),
            )?;
            if snapshot.payload.revision < state.published_revision
                || snapshot.payload.connections.capabilities
                    != super::compiled_connection_capabilities()
            {
                return Err(AgentError::Invalid(
                    "dataplane snapshot predates connection identity publication",
                ));
            }
            let fresh_at = Instant::now();
            // This MUST stay before disk persistence and either saved ACK. A
            // changed routing rule invalidates even an unchanged adjacency plan.
            manager.invalidate_dataplane(&snapshot).await;
            self.persist(agent, snapshot.clone())?;
            agent.persist_configuration(configuration.clone())?;
            agent
                .connections
                .as_mut()
                .expect("published connections")
                .persist(&agent.path, &agent.identity, connections.clone())?;
            self.check(agent)?;
            for file in [
                path(&agent.path),
                configuration::cache_path(&agent.path),
                super::cache_path(&agent.path),
                connection_key::path(&agent.path),
            ] {
                local::ensure_durable(&file)?;
            }
            manager.prepare_dataplane_sink(&snapshot).await?;
            #[cfg(unix)]
            manager.set_probe_inventory(&snapshot.payload.configuration);
            let ack = client
                .acknowledge_configuration(AcknowledgeConfigurationRequest {
                    revision: snapshot.payload.revision.into(),
                    digest: configuration.digest.clone(),
                })
                .await;
            let ack = match ack {
                Err(error) if error.api().is_some_and(|error| error.status == 409) => {
                    // Even an identical fetched bundle is now known stale. Do
                    // not keep its grants alive while the retry GET can stall.
                    manager
                        .withdraw(ConnectionRuntimeReason::InvalidConfiguration)
                        .await;
                    continue;
                }
                Err(error) => return Err(error.into()),
                Ok(value) => value,
            };
            if ack.revision != i64::from(snapshot.payload.revision)
                || ack.digest != configuration.digest
            {
                return Err(AgentError::Invalid("coherent configuration ACK mismatch"));
            }
            let ack = client
                .acknowledge_agent_connections(AcknowledgeAgentConnectionsRequest {
                    revision: snapshot.payload.revision.into(),
                    digest: connections.digest.clone(),
                })
                .await;
            let ack: AcknowledgeAgentConnectionsResponse = match ack {
                Err(error) if error.api().is_some_and(|error| error.status == 409) => {
                    manager
                        .withdraw(ConnectionRuntimeReason::InvalidConfiguration)
                        .await;
                    continue;
                }
                Err(error) => return Err(error.into()),
                Ok(value) => convert(value)?,
            };
            if ack.revision != snapshot.payload.revision
                || ack.digest != connections.digest
                || ack.status != ConnectionReceiptStatus::Saved
            {
                return Err(AgentError::Invalid("coherent connections ACK mismatch"));
            }
            self.check(agent)?;
            let receipt = ControlSyncReceipts {
                configuration: super::SyncReceipt {
                    controller: agent.identity.controller.clone(),
                    node_id: snapshot.payload.node_id.clone(),
                    stored_revision: snapshot.payload.revision,
                    digest: configuration.digest,
                    mesh_ipv4: configuration.payload.node.mesh_ipv4,
                    mesh_ipv6: configuration.payload.node.mesh_ipv6,
                    policy_valid: configuration.payload.policy_valid,
                },
                connections: ConnectionSyncReceipt {
                    controller: agent.identity.controller.clone(),
                    node_id: snapshot.payload.node_id.clone(),
                    stored_revision: snapshot.payload.revision,
                    digest: connections.digest,
                    configuration_valid: connections.payload.valid,
                    status: ConnectionReceiptStatus::Saved,
                    applied: false,
                },
            };
            return Ok(Synced {
                receipt,
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
