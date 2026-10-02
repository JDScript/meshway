//! Immutable local recovery binding. This file conveys no business authorization.
use super::{Agent, AgentError, Identity, local};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Binding {
    pub(crate) version: u8,
    pub(crate) controller: String,
    pub(crate) node_id: String,
    pub(crate) credential_hash: Vec<u8>,
    pub(crate) network_id: String,
}
pub(crate) fn sidecar(state: &Path, suffix: &str) -> PathBuf {
    let mut path = state.as_os_str().to_owned();
    path.push(suffix);
    PathBuf::from(path)
}
fn valid_network(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn load_binding(agent: &Agent) -> Result<Option<Binding>, AgentError> {
    let path = sidecar(&agent.path, ".network-binding.json");
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
        Ok(_) => {}
    }
    let binding: Binding = serde_json::from_str(&local::read_private_bounded(&path, 16_384)?)
        .map_err(|_| AgentError::Invalid("damaged network binding; preserve sidecar"))?;
    if binding.version != 1
        || binding.controller != agent.identity.controller
        || Some(&binding.node_id) != agent.identity.node_id.as_ref()
        || binding.credential_hash != local::digest(&agent.identity.node_credential)
        || !valid_network(&binding.network_id)
    {
        return Err(AgentError::Invalid(
            "network binding belongs to different identity or is invalid",
        ));
    }
    local::ensure_durable(&path)?;
    Ok(Some(binding))
}
pub(crate) fn fingerprint(identity: &Identity) -> Result<String, AgentError> {
    let encoded = serde_json::to_string(&(
        "meshway-enrollment-underlay-v1",
        &identity.controller,
        &identity.name,
        &identity.enrollment_version,
        &identity.enrollment_token_hash,
        local::digest(&identity.node_credential),
    ))
    .map_err(|_| AgentError::Invalid("cannot encode enrollment binding"))?;
    Ok(local::digest(&encoded)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

pub(crate) fn persist_binding(agent: &Agent, network: &str) -> Result<(), AgentError> {
    if !valid_network(network) || Identity::load(&agent.path)? != agent.identity {
        return Err(AgentError::Invalid(
            "invalid or changed identity before network binding",
        ));
    }
    if let Some(previous) = load_binding(agent)? {
        if previous.network_id != network {
            return Err(AgentError::Invalid("network binding cannot change"));
        }
        return Ok(());
    }
    let binding = Binding {
        version: 1,
        controller: agent.identity.controller.clone(),
        node_id: agent
            .identity
            .node_id
            .clone()
            .ok_or(AgentError::Invalid("binding requires enrolled identity"))?,
        credential_hash: local::digest(&agent.identity.node_credential),
        network_id: network.into(),
    };
    let bytes = serde_json::to_vec(&binding)
        .map_err(|_| AgentError::Invalid("cannot encode network binding"))?;
    local::atomic_write(
        &sidecar(&agent.path, ".network-binding.json"),
        &bytes,
        false,
    )?;
    Ok(())
}

pub(crate) fn check_network(agent: &Agent, network: &str) -> Result<(), AgentError> {
    if let Some(binding) = load_binding(agent)?
        && binding.network_id != network
    {
        return Err(AgentError::Invalid(
            "controller snapshot differs from durable network binding",
        ));
    }
    Ok(())
}
