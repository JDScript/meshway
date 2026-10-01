use std::{
    fs::File,
    path::{Path, PathBuf},
};

#[cfg(test)]
use crate::test_local as local;
#[cfg(not(test))]
use meshway_protocol::local;
use serde::{Deserialize, Serialize};

use crate::generated::{
    client::{ApiOpError, HttpClient},
    types::{AgentNodeResponse, EnrollAgentRequest, HeartbeatRequest},
};

mod configuration;
mod connection_key;
mod connections;
pub use connections::{
    ConnectionStatus, ConnectionSyncReceipt, ControlSyncReceipts, compiled_connection_capabilities,
    connections_status,
};
#[cfg(test)]
mod connection_key_tests;
#[cfg(test)]
mod durability_tests;
pub use configuration::SyncReceipt;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("agent state file error: {0}")]
    File(#[from] std::io::Error),
    #[error("{0}")]
    Write(#[from] local::AtomicWriteError),
    #[error("{0}")]
    Invalid(&'static str),
    // Never format arbitrary server bodies or request data in CLI output.
    #[error("controller request failed (HTTP {status})")]
    Http { status: u16, retryable: bool },
    #[error("controller transport failed")]
    Transport { retryable: bool },
    #[cfg(feature = "managed-connections")]
    #[error("connection runtime observation changed; resynchronizing")]
    RuntimeObservationChanged,
}

impl AgentError {
    pub fn retryable(&self) -> bool {
        match self {
            Self::Http { retryable, .. } | Self::Transport { retryable } => *retryable,
            #[cfg(feature = "managed-connections")]
            Self::RuntimeObservationChanged => true,
            _ => false,
        }
    }
}

impl<E: std::fmt::Debug> From<ApiOpError<E>> for AgentError {
    fn from(error: ApiOpError<E>) -> Self {
        match error {
            ApiOpError::Api(error) => Self::Http {
                status: error.status,
                retryable: error.is_retryable(),
            },
            ApiOpError::Transport(error) => Self::Transport {
                retryable: error.is_retryable(),
            },
        }
    }
}

// This is a local disk format, not an HTTP DTO. No Debug implementation: it
// contains the node's only plaintext credential and must never be logged.
#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    format_version: u32,
    controller: String,
    name: String,
    enrollment_version: String,
    enrollment_token_hash: Vec<u8>,
    node_credential: String,
    node_id: Option<String>,
}

impl Identity {
    fn load(path: &Path) -> Result<Self, AgentError> {
        let identity: Self = serde_json::from_str(&local::read_private(path)?).map_err(|_| {
            AgentError::Invalid("unrecognized or damaged agent state; refusing to overwrite")
        })?;
        if identity.format_version != 1
            || normalize_controller(&identity.controller)? != identity.controller
            || !valid_name(&identity.name)
            || identity.enrollment_version.trim().is_empty()
            || identity.enrollment_version.chars().count() > 64
            || identity.enrollment_token_hash.len() != 32
            || !local::valid_credential(&identity.node_credential)
            || identity
                .node_id
                .as_ref()
                .is_some_and(|id| !local::valid_credential(id))
        {
            return Err(AgentError::Invalid(
                "invalid agent state; refusing to overwrite",
            ));
        }
        Ok(identity)
    }

    fn status(&self) -> AgentStatus {
        AgentStatus {
            controller: self.controller.clone(),
            name: self.name.clone(),
            node_id: self.node_id.clone(),
            enrollment: if self.node_id.is_some() {
                "enrolled"
            } else {
                "pending"
            },
            stored_revision: 0,
            stored_digest: None,
            mesh_ipv4: None,
            mesh_ipv6: None,
        }
    }
}

/// Safe local status. Does not expose credentials or imply a live connection.
#[derive(Debug, Serialize)]
pub struct AgentStatus {
    pub controller: String,
    pub name: String,
    pub node_id: Option<String>,
    pub enrollment: &'static str,
    pub stored_revision: u32,
    pub stored_digest: Option<String>,
    pub mesh_ipv4: Option<String>,
    pub mesh_ipv6: Option<String>,
}

pub fn status(path: impl AsRef<Path>) -> Result<AgentStatus, AgentError> {
    let identity = Identity::load(path.as_ref())?;
    let cache = configuration::load(path.as_ref(), &identity)?;
    Ok(configuration::status(&identity, cache.as_ref()))
}

pub struct Agent {
    path: PathBuf,
    identity: Identity,
    cache: Option<configuration::Cache>,
    // Opt-in and lazy: ordinary heartbeat/routing commands do not touch sidecars.
    connections: Option<connections::State>,
    _lock: File,
}

impl Agent {
    /// Persist credential + immutable enrollment binding before any network I/O.
    /// Existing state must have the same controller, name and enrollment token.
    pub fn prepare_enrollment(
        path: impl AsRef<Path>,
        controller: &str,
        token: &str,
        name: &str,
    ) -> Result<Self, AgentError> {
        let controller = normalize_controller(controller)?;
        if !valid_name(name) {
            return Err(AgentError::Invalid(
                "name must contain 1–128 Unicode characters and non-whitespace text",
            ));
        }
        if !local::valid_credential(token) {
            return Err(AgentError::Invalid("invalid enrollment token format"));
        }
        let path = path.as_ref().to_path_buf();
        let lock = lock_state(&path)?;
        let token_hash = local::digest(token);
        let identity = match std::fs::symlink_metadata(&path) {
            Ok(_) => {
                let identity = Identity::load(&path)?;
                if identity.controller != controller
                    || identity.name != name
                    || identity.enrollment_token_hash != token_hash
                {
                    return Err(AgentError::Invalid(
                        "existing state belongs to a different enrollment; use its original controller, name and token or a different state file",
                    ));
                }
                identity
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let identity = Identity {
                    format_version: 1,
                    controller,
                    name: name.to_owned(),
                    enrollment_version: env!("CARGO_PKG_VERSION").to_owned(),
                    enrollment_token_hash: token_hash,
                    node_credential: local::credential()?,
                    node_id: None,
                };
                write_identity(&path, &identity, false)?;
                identity
            }
            Err(error) => return Err(error.into()),
        };
        let cache = configuration::load(&path, &identity)?;
        Ok(Self {
            path,
            identity,
            cache,
            connections: None,
            _lock: lock,
        })
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, AgentError> {
        let path = path.as_ref().to_path_buf();
        let lock = lock_state(&path)?;
        let identity = Identity::load(&path)?;
        let cache = configuration::load(&path, &identity)?;
        Ok(Self {
            path,
            identity,
            cache,
            connections: None,
            _lock: lock,
        })
    }

    pub async fn enroll(&mut self, token: &str) -> Result<AgentNodeResponse, AgentError> {
        if local::digest(token) != self.identity.enrollment_token_hash {
            return Err(AgentError::Invalid(
                "enrollment token does not match persisted binding",
            ));
        }
        // A previous creation/replacement may be visible but have failed its
        // directory sync. Confirm credential durability before network use.
        local::ensure_durable(&self.path)?;
        let response = HttpClient::new()
            .with_base_url(&self.identity.controller)
            .with_api_key(token)
            .enroll_agent(EnrollAgentRequest {
                name: self.identity.name.clone(),
                agent_version: self.identity.enrollment_version.clone(),
                node_credential: self.identity.node_credential.clone(),
            })
            .await?;
        self.accept(response)
    }

    /// Works for pending local state after the server committed enrollment but
    /// its response was lost. No enrollment token is needed for this recovery.
    pub async fn heartbeat(&mut self) -> Result<AgentNodeResponse, AgentError> {
        local::ensure_durable(&self.path)?;
        let response = HttpClient::new()
            .with_base_url(&self.identity.controller)
            .with_api_key(&self.identity.node_credential)
            .heartbeat_agent(HeartbeatRequest {
                agent_version: env!("CARGO_PKG_VERSION").to_owned(),
            })
            .await?;
        self.accept(response)
    }

    fn accept(&mut self, response: AgentNodeResponse) -> Result<AgentNodeResponse, AgentError> {
        if !local::valid_credential(&response.node.id)
            || response.node.name != self.identity.name
            || response.node.revoked_at.is_some()
            || self
                .identity
                .node_id
                .as_ref()
                .is_some_and(|id| id != &response.node.id)
        {
            return Err(AgentError::Invalid(
                "controller response does not match the persisted active identity",
            ));
        }
        if self.identity.node_id.is_none() {
            if Identity::load(&self.path)? != self.identity {
                return Err(AgentError::Invalid(
                    "state changed during enrollment; refusing to overwrite",
                ));
            }
            self.identity.node_id = Some(response.node.id.clone());
            if let Err(error) = write_identity(&self.path, &self.identity, true) {
                if !matches!(&error, AgentError::Write(write) if write.committed()) {
                    self.identity.node_id = None;
                }
                return Err(error);
            }
        }
        Ok(response)
    }

    pub fn status(&self) -> AgentStatus {
        configuration::status(&self.identity, self.cache.as_ref())
    }
}

fn write_identity(path: &Path, identity: &Identity, replace: bool) -> Result<(), AgentError> {
    let bytes = serde_json::to_vec_pretty(identity)
        .map_err(|_| AgentError::Invalid("cannot encode local identity"))?;
    local::atomic_write(path, &bytes, replace)?;
    Ok(())
}

fn lock_state(path: &Path) -> Result<File, AgentError> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    local::private_dir(parent)?;
    let mut lock = path.as_os_str().to_os_string();
    lock.push(".lock");
    Ok(local::lock(Path::new(&lock))?)
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 128
}

fn normalize_controller(controller: &str) -> Result<String, AgentError> {
    let url = reqwest::Url::parse(controller)
        .map_err(|_| AgentError::Invalid("invalid controller URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AgentError::Invalid(
            "controller URL must be HTTP(S) without credentials, query or fragment",
        ));
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}
