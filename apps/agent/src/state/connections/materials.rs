//! Private, node/controller/network-bound material journal. Disk only supplies
//! a rollback floor; a fresh authenticated response is required for execution.
use super::{Agent, AgentError, local};
use meshway_protocol::{ManagedVlessMaterial, managed_digest};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    format_version: u32,
    controller: String,
    node_id: String,
    credential_hash: Vec<u8>,
    network_id: String,
    revision: u32,
    digest: String,
    materials: Vec<ManagedVlessMaterial>,
}

fn path(state: &Path) -> PathBuf {
    let mut path = state.as_os_str().to_owned();
    path.push(".transport-materials.json");
    PathBuf::from(path)
}

pub(super) async fn publish(agent: &Agent) -> Result<u32, AgentError> {
    let response: meshway_protocol::PublishManagedTransportCapabilitiesResponse = super::convert(
        crate::generated::client::HttpClient::new()
            .with_base_url(&agent.identity.controller)
            .with_api_key(&agent.identity.node_credential)
            .publish_managed_transport_capabilities(super::convert(
                meshway_protocol::PublishManagedTransportCapabilitiesRequest {
                    vless_streams: vec![
                        meshway_protocol::VlessStreamTransport::Tcp,
                        meshway_protocol::VlessStreamTransport::Websocket,
                    ],
                },
            )?)
            .await?,
    )?;
    if response.revision == 0 {
        return Err(AgentError::Invalid("invalid managed capability revision"));
    }
    Ok(response.revision)
}

pub(super) fn persist<T: Serialize>(
    agent: &Agent,
    network_id: &str,
    revision: u32,
    snapshot: &T,
    materials: &[ManagedVlessMaterial],
    digest: &str,
) -> Result<(), AgentError> {
    if network_id.len() != 32
        || !network_id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || revision == 0
        || managed_digest(network_id, snapshot, materials)
            .map_err(|_| AgentError::Invalid("cannot hash managed snapshot"))?
            != digest
    {
        return Err(AgentError::Invalid(
            "managed snapshot digest or scope mismatch",
        ));
    }
    let path = path(&agent.path);
    let existing = match std::fs::symlink_metadata(&path) {
        Ok(_) => Some(
            serde_json::from_str::<Cache>(&local::read_private_bounded(&path, 2 * 1024 * 1024)?)
                .map_err(|_| AgentError::Invalid("invalid material journal; refusing overwrite"))?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let node_id = agent
        .identity
        .node_id
        .as_ref()
        .ok_or(AgentError::Invalid("missing material node binding"))?;
    let journal_digest = managed_digest(network_id, &revision, materials)
        .map_err(|_| AgentError::Invalid("cannot hash material journal"))?;
    if let Some(old) = &existing
        && (old.format_version != 1
            || old.controller != agent.identity.controller
            || &old.node_id != node_id
            || old.credential_hash != local::digest(&agent.identity.node_credential)
            || old.network_id != network_id
            || managed_digest(&old.network_id, &old.revision, &old.materials)
                .map_err(|_| AgentError::Invalid("cannot hash old material journal"))?
                != old.digest
            || old.revision > revision
            || (old.revision == revision && old.digest != journal_digest))
    {
        return Err(AgentError::Invalid(
            "material journal binding, rollback or same-revision mismatch",
        ));
    }
    if let Some(old) = &existing {
        for material in materials {
            if old.materials.iter().any(|previous| {
                previous.listener_id == material.listener_id
                    && (previous.version > material.version
                        || (previous.version == material.version && previous != material))
            }) {
                return Err(AgentError::Invalid(
                    "managed material rollback or same-version mutation",
                ));
            }
        }
    }
    if existing
        .as_ref()
        .is_none_or(|old| old.digest != journal_digest)
    {
        let cache = Cache {
            format_version: 1,
            controller: agent.identity.controller.clone(),
            node_id: node_id.clone(),
            credential_hash: local::digest(&agent.identity.node_credential),
            network_id: network_id.into(),
            revision,
            digest: journal_digest,
            materials: materials.to_vec(),
        };
        let bytes = zeroize::Zeroizing::new(
            serde_json::to_vec(&cache)
                .map_err(|_| AgentError::Invalid("cannot encode material journal"))?,
        );
        local::atomic_write(&path, &bytes, existing.is_some())?;
    }
    local::ensure_durable(&path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn private_journal_pins_network_credential_revision_and_recovers_a_durable_restart() {
        let directory = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let state = directory.path().join("agent.json");
        let mut agent =
            Agent::prepare_enrollment(&state, "http://127.0.0.1", &"a".repeat(64), "journal-test")
                .unwrap();
        agent.identity.node_id = Some("b".repeat(64));
        crate::state::write_identity(&state, &agent.identity, true).unwrap();
        let network = "0".repeat(32);
        let snapshot = "public-snapshot";
        let digest = managed_digest(&network, &snapshot, &[]).unwrap();
        persist(&agent, &network, 7, &snapshot, &[], &digest).unwrap();
        assert_eq!(
            std::fs::metadata(path(&state))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let journal = local::read_private(&path(&state)).unwrap();
        drop(agent);
        let agent = Agent::load(&state).unwrap();
        persist(&agent, &network, 7, &snapshot, &[], &digest).unwrap();
        assert_eq!(local::read_private(&path(&state)).unwrap(), journal);
        assert!(persist(&agent, &network, 6, &snapshot, &[], &digest).is_err());
        let other = "1".repeat(32);
        let digest_other = managed_digest(&other, &snapshot, &[]).unwrap();
        assert!(persist(&agent, &other, 8, &snapshot, &[], &digest_other).is_err());
        assert!(persist(&agent, &network, 8, &snapshot, &[], &"0".repeat(64)).is_err());
        let mut corrupted: Cache = serde_json::from_str(&journal).unwrap();
        corrupted.digest = "0".repeat(64);
        local::atomic_write(
            &path(&state),
            &serde_json::to_vec(&corrupted).unwrap(),
            true,
        )
        .unwrap();
        assert!(persist(&agent, &network, 8, &snapshot, &[], &digest).is_err());
    }
}
