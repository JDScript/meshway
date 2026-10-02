//! Read-only presentation snapshots. No ACK, persistence or dataplane activation.
use meshway_control_client::{self as generated, ApiOpError};
use meshway_protocol::{self as protocol, local};

use crate::SignInError;
use crate::signin::{client, on_runtime, sanitized};

#[derive(uniffi::Record)]
pub struct InventoryDevice {
    pub id: String,
    pub name: String,
    pub country: Option<String>,
    pub mesh_ipv4: String,
    pub mesh_ipv6: String,
}

#[derive(uniffi::Record)]
pub struct DeviceInventory {
    pub revision: u32,
    pub digest: String,
    pub ipv4_prefix: String,
    pub ipv6_prefix: String,
    pub own_device: InventoryDevice,
    pub peers: Vec<InventoryDevice>,
}

#[derive(uniffi::Record)]
pub struct DeviceDnsName {
    pub node_id: String,
    pub name: String,
    pub canonical_name: String,
    pub mesh_ipv4: String,
    pub mesh_ipv6: String,
}

#[derive(uniffi::Record)]
pub struct DeviceDnsInventory {
    pub revision: u32,
    pub digest: String,
    pub zone: Option<String>,
    pub names: Vec<DeviceDnsName>,
}

fn rejected<E: std::fmt::Debug>(error: ApiOpError<E>) -> SignInError {
    if error.api().is_some_and(|error| error.status == 401) {
        SignInError::CredentialRejected
    } else {
        sanitized(error)
    }
}

fn device(node: protocol::MeshNode) -> InventoryDevice {
    InventoryDevice {
        id: node.id,
        name: node.name,
        country: node.country,
        mesh_ipv4: node.mesh_ipv4,
        mesh_ipv6: node.mesh_ipv6,
    }
}

fn configuration(
    snapshot: generated::ConfigurationSnapshot,
    node_id: &str,
) -> Result<DeviceInventory, SignInError> {
    // Generated wire DTO -> authoritative protocol type, not untyped JSON.
    let bytes = serde_json::to_vec(&snapshot).map_err(|_| SignInError::Server)?;
    if bytes.len() > protocol::MAX_CONFIGURATION_BYTES {
        return Err(SignInError::Server);
    }
    let snapshot: protocol::ConfigurationSnapshot =
        serde_json::from_slice(&bytes).map_err(|_| SignInError::Server)?;
    let payload = snapshot.payload;
    if payload.schema_version != protocol::CONFIGURATION_SCHEMA_VERSION
        || payload.revision == 0
        || payload.node.id != node_id
        || !local::valid_credential(node_id)
        || payload.peers.len() >= protocol::MAX_INVENTORY_NODES
        || payload
            .peers
            .windows(2)
            .any(|pair| pair[0].id >= pair[1].id)
        || protocol::configuration_digest(&payload).map_err(|_| SignInError::Server)?
            != snapshot.digest
    {
        return Err(SignInError::Server);
    }
    let nodes: Vec<_> = std::iter::once(&payload.node)
        .chain(&payload.peers)
        .cloned()
        .collect();
    if nodes.iter().any(|node| {
        !local::valid_credential(&node.id)
            || node.name.trim().is_empty()
            || node.name.chars().count() > 128
            || node.name.chars().any(char::is_control)
            || node.country.as_ref().is_some_and(|country| {
                country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_uppercase())
            })
    }) || !meshway_policy::compile(&payload.routing_policy, &payload.network, &nodes).is_valid()
    {
        return Err(SignInError::Server);
    }
    Ok(DeviceInventory {
        revision: payload.revision,
        digest: snapshot.digest,
        ipv4_prefix: payload.network.ipv4_prefix,
        ipv6_prefix: payload.network.ipv6_prefix,
        own_device: device(payload.node),
        peers: payload.peers.into_iter().map(device).collect(),
    })
}

fn dns(
    snapshot: generated::AgentDnsSnapshot,
    node_id: &str,
) -> Result<DeviceDnsInventory, SignInError> {
    let bytes = serde_json::to_vec(&snapshot).map_err(|_| SignInError::Server)?;
    if bytes.len() > protocol::MAX_DNS_BYTES {
        return Err(SignInError::Server);
    }
    let snapshot: protocol::AgentDnsSnapshot =
        serde_json::from_slice(&bytes).map_err(|_| SignInError::Server)?;
    let payload = snapshot.payload;
    payload.validate().map_err(|_| SignInError::Server)?;
    if payload.node_id != node_id
        || protocol::dns_digest(&payload).map_err(|_| SignInError::Server)? != snapshot.digest
    {
        return Err(SignInError::Server);
    }
    Ok(DeviceDnsInventory {
        revision: payload.revision,
        digest: snapshot.digest,
        zone: payload.dns.enabled.then_some(payload.dns.zone),
        names: payload
            .records
            .into_iter()
            .map(|record| DeviceDnsName {
                node_id: record.node_id,
                name: record
                    .aliases
                    .first()
                    .cloned()
                    .unwrap_or_else(|| record.canonical_name.clone()),
                canonical_name: record.canonical_name,
                mesh_ipv4: record.mesh_ipv4,
                mesh_ipv6: record.mesh_ipv6,
            })
            .collect(),
    })
}

#[uniffi::export]
pub async fn get_device_configuration(
    server_url: String,
    credential: String,
    node_id: String,
) -> Result<DeviceInventory, SignInError> {
    on_runtime(async move {
        let snapshot = client(&server_url, &credential)?
            .with_max_response_body_bytes(protocol::MAX_CONFIGURATION_BYTES)
            .get_agent_configuration()
            .await
            .map_err(rejected)?;
        configuration(snapshot, &node_id)
    })
    .await
}

#[uniffi::export]
pub async fn get_device_dns(
    server_url: String,
    credential: String,
    node_id: String,
) -> Result<DeviceDnsInventory, SignInError> {
    on_runtime(async move {
        let snapshot = client(&server_url, &credential)?
            .with_max_response_body_bytes(protocol::MAX_DNS_BYTES)
            .get_agent_dns()
            .await
            .map_err(rejected)?;
        dns(snapshot, &node_id)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> protocol::ConfigurationSnapshot {
        let payload = protocol::NodeConfiguration {
            schema_version: protocol::CONFIGURATION_SCHEMA_VERSION,
            revision: 1,
            network: protocol::MeshNetwork {
                ipv4_prefix: "100.64.0.0/24".into(),
                ipv6_prefix: "fd42::/64".into(),
            },
            node: protocol::MeshNode {
                id: "a".repeat(64),
                name: "Mac".into(),
                country: None,
                mesh_ipv4: "100.64.0.1".into(),
                mesh_ipv6: "fd42::1".into(),
            },
            peers: vec![],
            routing_policy: Default::default(),
            policy_valid: true,
            diagnostics: vec![],
        };
        protocol::ConfigurationSnapshot {
            digest: protocol::configuration_digest(&payload).unwrap(),
            payload,
        }
    }
    fn wire(snapshot: &protocol::ConfigurationSnapshot) -> generated::ConfigurationSnapshot {
        serde_json::from_slice(&serde_json::to_vec(snapshot).unwrap()).unwrap()
    }
    #[test]
    fn inventory_rejects_digest_tampering_binding_and_invalid_addresses() {
        let mut snapshot = fixture();
        assert!(configuration(wire(&snapshot), &"a".repeat(64)).is_ok());
        assert!(configuration(wire(&snapshot), &"b".repeat(64)).is_err());
        snapshot.payload.node.name = "Changed".into();
        assert!(configuration(wire(&snapshot), &"a".repeat(64)).is_err());
        snapshot.payload.node.mesh_ipv4 = "8.8.8.8".into();
        snapshot.digest = protocol::configuration_digest(&snapshot.payload).unwrap();
        assert!(configuration(wire(&snapshot), &"a".repeat(64)).is_err());
    }
}
