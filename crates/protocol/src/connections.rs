//! Separate connection control intent. Publication binds control-plane identity;
//! receipts mean saved only, never peer proof, connectivity or flow authorization.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use crate::NodeSummary;

pub const CONNECTIONS_SCHEMA_VERSION: u32 = 1;
pub const MAX_CONNECTIONS_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionTransport {
    NativeUdp,
    NativeTcp,
    Vless,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionSemantics {
    Datagram,
    Stream,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionCapability {
    pub transport: ConnectionTransport,
    pub can_dial: bool,
    pub can_accept: bool,
    pub semantics: ConnectionSemantics,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishConnectionIdentityRequest {
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub public_key: String,
    #[schema(max_items = 3)]
    pub capabilities: Vec<ConnectionCapability>,
}

impl PublishConnectionIdentityRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !valid_connection_public_key(&self.public_key) {
            return Err("public_key must be canonical non-low-order X25519 lowercase hex");
        }
        if self.capabilities.len() > 3 {
            return Err("at most three compiled transport capabilities are supported");
        }
        let mut seen = std::collections::BTreeSet::new();
        for capability in &self.capabilities {
            if !seen.insert(capability.transport) {
                return Err("duplicate transport capability");
            }
            let semantics = match capability.transport {
                ConnectionTransport::NativeUdp => ConnectionSemantics::Datagram,
                ConnectionTransport::NativeTcp | ConnectionTransport::Vless => {
                    ConnectionSemantics::Stream
                }
            };
            if capability.semantics != semantics || !(capability.can_dial || capability.can_accept)
            {
                return Err("capability roles or transport semantics are invalid");
            }
        }
        Ok(())
    }
}

/// Reject noncanonical field encodings so one key cannot acquire alias bindings.
/// Dalek performs the X25519 low-order/contributory check; no custom curve math.
pub fn valid_connection_public_key(value: &str) -> bool {
    if !crate::local::valid_credential(value) {
        return false;
    }
    let mut bytes = [0_u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let Ok(decoded) = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16) else {
            return false;
        };
        *byte = decoded;
    }
    // The canonical little-endian u-coordinate is strictly below 2^255 - 19.
    let mut modulus = [255_u8; 32];
    modulus[0] = 237;
    modulus[31] = 127;
    if bytes.iter().rev().cmp(modulus.iter().rev()) != std::cmp::Ordering::Less {
        return false;
    }
    x25519_dalek::StaticSecret::from([0x42; 32])
        .diffie_hellman(&x25519_dalek::PublicKey::from(bytes))
        .was_contributory()
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishConnectionIdentityResponse {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    pub node_id: String,
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub public_key: String,
    pub capabilities: Vec<ConnectionCapability>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionEndpoint {
    #[schema(min_length = 1, max_length = 253)]
    pub host: String,
    #[schema(format = Int32, minimum = 1, maximum = 65535)]
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionListener {
    #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9._-]+$")]
    pub id: String,
    pub node_id: String,
    pub transport: ConnectionTransport,
    pub bind: String,
    #[schema(min_items = 1, max_items = 16)]
    pub advertised_endpoints: Vec<ConnectionEndpoint>,
    #[schema(
        required = true,
        min_length = 1,
        max_length = 64,
        pattern = "^[A-Za-z0-9._-]+$"
    )]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub credential_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionCandidate {
    #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9._-]+$")]
    pub id: String,
    pub dialer_node_id: String,
    pub listener_id: String,
    pub endpoint: ConnectionEndpoint,
    #[schema(format = Int32, minimum = 0, maximum = 65535)]
    pub priority: u16,
    #[schema(
        required = true,
        min_length = 1,
        max_length = 64,
        pattern = "^[A-Za-z0-9._-]+$"
    )]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub credential_ref: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionPlan {
    #[schema(max_items = 128)]
    pub listeners: Vec<ConnectionListener>,
    #[schema(max_items = 256)]
    pub candidates: Vec<ConnectionCandidate>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveConnectionsRequest {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub expected_revision: u32,
    pub configuration: ConnectionPlan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionDiagnostic {
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub item_id: Option<String>,
    pub field: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionNode {
    pub node: NodeSummary,
    #[schema(
        required = true,
        min_length = 64,
        max_length = 64,
        pattern = "^[0-9a-f]{64}$"
    )]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub public_key: Option<String>,
    pub capabilities: Vec<ConnectionCapability>,
    #[schema(format = Int64, minimum = 0, maximum = 4294967295_u64)]
    pub acknowledged_revision: u32,
    #[schema(
        required = true,
        min_length = 64,
        max_length = 64,
        pattern = "^[0-9a-f]{64}$"
    )]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub acknowledged_digest: Option<String>,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub acknowledged_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionsSnapshot {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    pub configuration: ConnectionPlan,
    pub valid: bool,
    pub diagnostics: Vec<ConnectionDiagnostic>,
    pub nodes: Vec<ConnectionNode>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionPeer {
    pub node_id: String,
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub public_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OutgoingConnectionCandidate {
    pub id: String,
    pub listener_id: String,
    pub peer_node_id: String,
    pub transport: ConnectionTransport,
    pub endpoint: ConnectionEndpoint,
    #[schema(format = Int32, minimum = 0, maximum = 65535)]
    pub priority: u16,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub credential_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IncomingConnectionApproval {
    pub candidate_id: String,
    pub listener_id: String,
    pub peer_node_id: String,
    pub transport: ConnectionTransport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeConnections {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub schema_version: u32,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    pub node_id: String,
    #[schema(
        required = true,
        min_length = 64,
        max_length = 64,
        pattern = "^[0-9a-f]{64}$"
    )]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub public_key: Option<String>,
    pub capabilities: Vec<ConnectionCapability>,
    pub valid: bool,
    pub diagnostics: Vec<ConnectionDiagnostic>,
    pub listeners: Vec<ConnectionListener>,
    pub outgoing: Vec<OutgoingConnectionCandidate>,
    pub incoming: Vec<IncomingConnectionApproval>,
    pub peers: Vec<ConnectionPeer>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentConnectionsSnapshot {
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub digest: String,
    pub payload: NodeConnections,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeAgentConnectionsRequest {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub digest: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionReceiptStatus {
    Saved,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeAgentConnectionsResponse {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub digest: String,
    pub status: ConnectionReceiptStatus,
}

pub fn connection_digest(payload: &NodeConnections) -> Result<String, serde_json::Error> {
    Ok(Sha256::digest(crate::canonical_json(payload)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x25519_validation_rejects_low_order_and_noncanonical_aliases() {
        for key in [
            "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a",
            "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f",
        ] {
            assert!(valid_connection_public_key(key));
            assert!(!valid_connection_public_key(&key.to_uppercase()));
        }
        for key in [
            "00".repeat(32),
            format!("01{}", "00".repeat(31)),
            "e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800".into(),
            "5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f1157".into(),
            format!("ec{}7f", "ff".repeat(30)),
            format!("ed{}7f", "ff".repeat(30)),
            format!("ee{}7f", "ff".repeat(30)),
            // p+9 would otherwise be an alias of the valid X25519 basepoint.
            format!("f6{}7f", "ff".repeat(30)),
            format!("ff{}7f", "ff".repeat(30)),
            format!("09{}80", "00".repeat(30)),
            "g".repeat(64),
            "0".repeat(63),
            "0".repeat(65),
        ] {
            assert!(!valid_connection_public_key(&key), "accepted {key}");
        }
    }

    #[test]
    fn capability_validation_and_closed_dtos_reject_mismatches() {
        let base = serde_json::json!({"public_key":format!("09{}", "00".repeat(31)),"capabilities":[{"transport":"native_udp","can_dial":true,"can_accept":false,"semantics":"datagram"}]});
        serde_json::from_value::<PublishConnectionIdentityRequest>(base.clone())
            .unwrap()
            .validate()
            .unwrap();
        let mut value = base.clone();
        value["node_id"] = "caller-chosen".into();
        assert!(serde_json::from_value::<PublishConnectionIdentityRequest>(value).is_err());
        for (field, bad) in [
            ("transport", serde_json::json!("wireguard")),
            ("can_dial", serde_json::json!("true")),
            ("unexpected", serde_json::json!(false)),
        ] {
            let mut value = base.clone();
            value["capabilities"][0][field] = bad;
            assert!(serde_json::from_value::<PublishConnectionIdentityRequest>(value).is_err());
        }
        let mut value = base.clone();
        value["capabilities"][0]["semantics"] = "stream".into();
        assert!(
            serde_json::from_value::<PublishConnectionIdentityRequest>(value)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut value = base.clone();
        value["capabilities"][0]["can_dial"] = false.into();
        assert!(
            serde_json::from_value::<PublishConnectionIdentityRequest>(value)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut value = base;
        let duplicate = value["capabilities"][0].clone();
        value["capabilities"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(
            serde_json::from_value::<PublishConnectionIdentityRequest>(value)
                .unwrap()
                .validate()
                .is_err()
        );
        let listener = serde_json::json!({"id":"x","node_id":"n","transport":"native_tcp","bind":"127.0.0.1:9","advertised_endpoints":[{"host":"localhost","port":9}],"credential_ref":null});
        assert!(serde_json::from_value::<ConnectionListener>(listener.clone()).is_ok());
        let mut value = listener;
        value.as_object_mut().unwrap().remove("credential_ref");
        assert!(serde_json::from_value::<ConnectionListener>(value).is_err());
    }
}
