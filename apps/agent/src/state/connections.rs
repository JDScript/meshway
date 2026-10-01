//! Node-scoped connection control intent. Nothing here starts a dataplane.

use std::path::{Path, PathBuf};

use meshway_protocol::{
    AgentConnectionsSnapshot, ConnectionCapability, ConnectionReceiptStatus, MAX_CONNECTIONS_BYTES,
    PublishConnectionIdentityResponse,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::{Agent, AgentError, Identity, SyncReceipt, connection_key::KeyStore, local};
use crate::generated::{client::HttpClient, types::AcknowledgeAgentConnectionsRequest};

#[cfg(feature = "managed-connections")]
mod dataplane;
#[cfg(feature = "managed-connections")]
mod managed;
#[cfg(test)]
mod tests;
mod validation;
#[cfg(test)]
mod validation_tests;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    format_version: u32,
    controller: String,
    node_id: String,
    snapshot: AgentConnectionsSnapshot,
}

pub(super) struct State {
    keys: KeyStore,
    cache: Option<Cache>,
    published_revision: u32,
}

/// Capabilities describe compiled carriers, not configured or running listeners.
pub fn compiled_connection_capabilities() -> Vec<ConnectionCapability> {
    #[cfg(feature = "transport-lab")]
    {
        use meshway_protocol::{ConnectionSemantics, ConnectionTransport};
        [
            (
                ConnectionTransport::NativeUdp,
                ConnectionSemantics::Datagram,
            ),
            (ConnectionTransport::NativeTcp, ConnectionSemantics::Stream),
            (ConnectionTransport::Vless, ConnectionSemantics::Stream),
        ]
        .into_iter()
        .map(|(transport, semantics)| ConnectionCapability {
            transport,
            semantics,
            can_dial: true,
            can_accept: true,
        })
        .collect()
    }
    #[cfg(not(feature = "transport-lab"))]
    Vec::new()
}

/// Safe local information. A saved revision is not an applied network state.
#[derive(Debug, Serialize)]
pub struct ConnectionStatus {
    pub controller: String,
    pub node_id: Option<String>,
    pub public_key: Option<String>,
    pub compiled_capabilities: Vec<ConnectionCapability>,
    pub stored_revision: u32,
    pub stored_digest: Option<String>,
    pub configuration_valid: Option<bool>,
    pub applied: bool,
}

/// The controller accepted a durable saved snapshot; no network apply is done.
#[derive(Debug, Serialize)]
pub struct ConnectionSyncReceipt {
    pub controller: String,
    pub node_id: String,
    pub stored_revision: u32,
    pub digest: String,
    pub configuration_valid: bool,
    pub status: ConnectionReceiptStatus,
    pub applied: bool,
}

/// Independent saved receipts; the two ACKs have different endpoint/digest scopes.
#[derive(Debug, Serialize)]
pub struct ControlSyncReceipts {
    pub configuration: SyncReceipt,
    pub connections: ConnectionSyncReceipt,
}

fn cache_path(state: &Path) -> PathBuf {
    let mut value = state.as_os_str().to_owned();
    value.push(".connections.json");
    PathBuf::from(value)
}

fn load_cache(
    state: &Path,
    identity: &Identity,
    keys: &KeyStore,
) -> Result<Option<Cache>, AgentError> {
    let path = cache_path(state);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let cache: Cache = serde_json::from_str(&local::read_private_bounded(
        &path,
        MAX_CONNECTIONS_BYTES + 64 * 1024,
    )?)
    .map_err(|_| AgentError::Invalid("invalid connections cache; refusing to overwrite"))?;
    if cache.format_version != 1
        || cache.controller != identity.controller
        || Some(&cache.node_id) != identity.node_id.as_ref()
    {
        return Err(AgentError::Invalid(
            "connections cache belongs to another controller or node",
        ));
    }
    let public_key = keys.public_key().ok_or(AgentError::Invalid(
        "connections cache has no private key sidecar; refusing to regenerate",
    ))?;
    validation::validate(&cache.snapshot, identity, public_key, None)?;
    Ok(Some(cache))
}

impl State {
    fn load(path: &Path, identity: &Identity) -> Result<Self, AgentError> {
        let keys = KeyStore::load(path, identity)?;
        let cache = load_cache(path, identity, &keys)?;
        Ok(Self {
            keys,
            cache,
            published_revision: 0,
        })
    }

    fn status(&self, identity: &Identity) -> ConnectionStatus {
        ConnectionStatus {
            controller: identity.controller.clone(),
            node_id: identity.node_id.clone(),
            public_key: self.keys.public_key().map(str::to_owned),
            compiled_capabilities: compiled_connection_capabilities(),
            stored_revision: self
                .cache
                .as_ref()
                .map_or(0, |cache| cache.snapshot.payload.revision),
            stored_digest: self
                .cache
                .as_ref()
                .map(|cache| cache.snapshot.digest.clone()),
            configuration_valid: self
                .cache
                .as_ref()
                .map(|cache| cache.snapshot.payload.valid),
            applied: false,
        }
    }

    fn check(&self, path: &Path, identity: &Identity) -> Result<(), AgentError> {
        self.keys.check(path, identity)?;
        let disk = load_cache(path, identity, &self.keys)?;
        if disk.as_ref().map(|cache| &cache.snapshot.digest)
            != self.cache.as_ref().map(|cache| &cache.snapshot.digest)
        {
            return Err(AgentError::Invalid("connections cache changed during sync"));
        }
        Ok(())
    }

    fn persist(
        &mut self,
        path: &Path,
        identity: &Identity,
        snapshot: AgentConnectionsSnapshot,
    ) -> Result<(), AgentError> {
        self.check(path, identity)?;
        let public_key = self.keys.public_key().ok_or(AgentError::Invalid(
            "connection identity has not been published",
        ))?;
        validation::validate(
            &snapshot,
            identity,
            public_key,
            self.cache.as_ref().map(|cache| &cache.snapshot),
        )?;
        if snapshot.payload.revision < self.published_revision
            || snapshot.payload.capabilities != compiled_connection_capabilities()
        {
            return Err(AgentError::Invalid(
                "connection configuration predates publication or mismatches compiled capabilities",
            ));
        }
        if self
            .cache
            .as_ref()
            .is_some_and(|cache| cache.snapshot.digest == snapshot.digest)
        {
            return Ok(());
        }
        let cache = Cache {
            format_version: 1,
            controller: identity.controller.clone(),
            node_id: snapshot.payload.node_id.clone(),
            snapshot,
        };
        let bytes = serde_json::to_vec(&cache)
            .map_err(|_| AgentError::Invalid("cannot encode connections cache"))?;
        let result = local::atomic_write(&cache_path(path), &bytes, self.cache.is_some());
        if result.is_ok() || result.as_ref().is_err_and(|error| error.committed()) {
            self.cache = Some(cache);
        }
        result?;
        Ok(())
    }
}

pub fn connections_status(path: impl AsRef<Path>) -> Result<ConnectionStatus, AgentError> {
    let path = path.as_ref();
    let identity = Identity::load(path)?;
    Ok(State::load(path, &identity)?.status(&identity))
}

// Generated transport DTOs are converted into the authoritative protocol types
// for digest/reference checks. No handwritten HTTP DTO or untyped JSON escapes.
fn convert<T: DeserializeOwned>(value: impl Serialize) -> Result<T, AgentError> {
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| AgentError::Invalid("cannot encode connection contract"))?;
    serde_json::from_slice(&bytes).map_err(|_| AgentError::Invalid("invalid connection contract"))
}

impl Agent {
    fn load_connections(&mut self) -> Result<(), AgentError> {
        if self.connections.is_none() {
            self.connections = Some(State::load(&self.path, &self.identity)?);
        }
        Ok(())
    }

    pub fn connection_status(&mut self) -> Result<ConnectionStatus, AgentError> {
        self.load_connections()?;
        Ok(self
            .connections
            .as_ref()
            .expect("loaded connection state")
            .status(&self.identity))
    }

    pub async fn publish_connection_identity(
        &mut self,
    ) -> Result<PublishConnectionIdentityResponse, AgentError> {
        if self.identity.node_id.is_none() {
            self.heartbeat().await?;
        }
        self.load_connections()?;
        let state = self.connections.as_mut().expect("loaded connection state");
        state.check(&self.path, &self.identity)?;
        let public_key = state.keys.prepare(&self.path, &self.identity)?.to_owned();
        let capabilities = compiled_connection_capabilities();
        let request = convert(meshway_protocol::PublishConnectionIdentityRequest {
            public_key: public_key.clone(),
            capabilities: capabilities.clone(),
        })?;
        let response = HttpClient::new()
            .with_base_url(&self.identity.controller)
            .with_api_key(&self.identity.node_credential)
            .publish_connection_identity(request)
            .await?;
        let response: PublishConnectionIdentityResponse = convert(response)?;
        // Protect against out-of-band replacement while the request was in flight.
        state.check(&self.path, &self.identity)?;
        if Some(&response.node_id) != self.identity.node_id.as_ref()
            || response.public_key != public_key
            || response.capabilities != capabilities
            || response.revision == 0
            || response.revision < state.published_revision
            || state
                .cache
                .as_ref()
                .is_some_and(|cache| response.revision < cache.snapshot.payload.revision)
        {
            return Err(AgentError::Invalid(
                "connection identity response does not match durable node/key binding",
            ));
        }
        state.published_revision = response.revision;
        Ok(response)
    }

    /// Publish before pulling: first binding/capability changes bump global revision.
    pub async fn sync_connections(&mut self) -> Result<ConnectionSyncReceipt, AgentError> {
        self.publish_connection_identity().await?;
        self.sync_published_connections().await
    }

    /// Ordering prevents our own publication from immediately staling the IP ACK.
    pub async fn sync_with_connections(&mut self) -> Result<ControlSyncReceipts, AgentError> {
        self.publish_connection_identity().await?;
        let configuration = self.sync().await?;
        let connections = self.sync_published_connections().await?;
        Ok(ControlSyncReceipts {
            configuration,
            connections,
        })
    }

    async fn sync_published_connections(&mut self) -> Result<ConnectionSyncReceipt, AgentError> {
        self.sync_published_connections_observe(|_| {}).await
    }

    async fn sync_published_connections_observe(
        &mut self,
        mut on_saved: impl FnMut(&AgentConnectionsSnapshot),
    ) -> Result<ConnectionSyncReceipt, AgentError> {
        let client = HttpClient::new()
            .with_base_url(&self.identity.controller)
            .with_api_key(&self.identity.node_credential);
        let state = self
            .connections
            .as_mut()
            .expect("published connection state");
        for _ in 0..3 {
            let snapshot = convert(client.get_agent_connections().await?)?;
            state.persist(&self.path, &self.identity, snapshot)?;
            // Recheck even when an unchanged cache was loaded after a crash.
            state.check(&self.path, &self.identity)?;
            local::ensure_durable(&super::connection_key::path(&self.path))?;
            local::ensure_durable(&cache_path(&self.path))?;
            let cache = state.cache.as_ref().expect("persisted connections cache");
            on_saved(&cache.snapshot);
            let response = client
                .acknowledge_agent_connections(AcknowledgeAgentConnectionsRequest {
                    revision: cache.snapshot.payload.revision.into(),
                    digest: cache.snapshot.digest.clone(),
                })
                .await;
            match response {
                Err(error) if error.api().is_some_and(|error| error.status == 409) => continue,
                Err(error) => return Err(error.into()),
                Ok(receipt) => {
                    let receipt: meshway_protocol::AcknowledgeAgentConnectionsResponse =
                        convert(receipt)?;
                    state.check(&self.path, &self.identity)?;
                    if receipt.revision != cache.snapshot.payload.revision
                        || receipt.digest != cache.snapshot.digest
                        || receipt.status != ConnectionReceiptStatus::Saved
                    {
                        return Err(AgentError::Invalid(
                            "connection ACK receipt does not match saved snapshot",
                        ));
                    }
                    return Ok(ConnectionSyncReceipt {
                        controller: self.identity.controller.clone(),
                        node_id: cache.node_id.clone(),
                        stored_revision: cache.snapshot.payload.revision,
                        digest: cache.snapshot.digest.clone(),
                        configuration_valid: cache.snapshot.payload.valid,
                        status: receipt.status,
                        applied: false,
                    });
                }
            }
        }
        Err(AgentError::Http {
            status: 409,
            retryable: true,
        })
    }
}
