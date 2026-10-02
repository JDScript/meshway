//! Public intent is deliberately distinct from node-only recoverable material.
use crate::{
    AgentConnectionsSnapshot, ConnectionEndpoint, ConnectionTransport,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum VlessStreamTransport {
    Tcp,
    Websocket,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedVlessSettings {
    pub stream: VlessStreamTransport,
    /// Certificate name and client verification name, independent of HTTP Host.
    #[schema(min_length = 1, max_length = 253)]
    pub server_name: String,
    #[schema(required = true, min_length = 1, max_length = 1024)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub websocket_path: Option<String>,
    #[schema(required = true, min_length = 1, max_length = 253)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub websocket_host: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedListenerSettings {
    #[schema(min_length = 1, max_length = 48, pattern = "^[A-Za-z0-9._-]+$")]
    pub id: String,
    pub node_id: String,
    pub enabled: bool,
    pub transport: ConnectionTransport,
    pub bind_address: String,
    #[schema(minimum = 1, maximum = 65535)]
    pub bind_port: u16,
    /// Explicit announcement, not proof of reachability. Empty means no dial endpoint.
    #[schema(max_items = 16)]
    pub advertised_endpoints: Vec<ConnectionEndpoint>,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub vless: Option<ManagedVlessSettings>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedListenerSummary {
    pub settings: ManagedListenerSettings,
    #[schema(minimum = 1, maximum = 4294967295_u64)]
    pub material_version: u32,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub certificate_sha256: Option<String>,
    /// False is retained intent awaiting a compatible identity/capability report.
    pub compiled_capability_available: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishManagedTransportCapabilitiesRequest {
    #[schema(max_items = 2)]
    pub vless_streams: Vec<VlessStreamTransport>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishManagedTransportCapabilitiesResponse {
    #[schema(minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedListenersSnapshot {
    #[schema(minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    pub listeners: Vec<ManagedListenerSummary>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveManagedListenerRequest {
    #[schema(minimum = 1, maximum = 4294967295_u64)]
    pub expected_revision: u32,
    pub settings: ManagedListenerSettings,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateManagedEnrollmentTokenRequest {
    #[schema(minimum = 60, maximum = 86400)]
    pub expires_in_seconds: u32,
    /// Initial intent only: node_id MUST be empty and is bound by enrollment.
    pub initial_listener: ManagedListenerSettings,
}

/// No Debug: serialization is permitted only on the node-authenticated endpoint.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedVlessMaterial {
    pub listener_id: String,
    pub credential_ref: String,
    #[schema(minimum = 1, maximum = 4294967295_u64)]
    pub version: u32,
    pub settings: ManagedVlessSettings,
    #[schema(format = Password)]
    pub uuid: String,
    #[schema(min_length = 1, max_length = 32768)]
    pub certificate_pem: String,
    pub certificate_sha256: String,
    /// Only listener owner receives this; dialers receive null.
    #[schema(required = true, format = Password, min_length = 1, max_length = 16384)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub private_key_pem: Option<String>,
}

impl Drop for ManagedVlessMaterial {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.uuid.zeroize();
        if let Some(key) = &mut self.private_key_pem {
            key.zeroize();
        }
    }
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentManagedConnectionsSnapshot {
    pub network_id: String,
    pub connections: AgentConnectionsSnapshot,
    #[schema(max_items = 128)]
    pub materials: Vec<ManagedVlessMaterial>,
    pub digest: String,
}

pub fn managed_digest<T: Serialize>(
    network_id: &str,
    snapshot: &T,
    materials: &[ManagedVlessMaterial],
) -> Result<String, serde_json::Error> {
    use sha2::{Digest, Sha256};
    Ok(
        Sha256::digest(crate::canonical_json(&(network_id, snapshot, materials))?)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}
