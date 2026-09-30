//! Versioned control intent. An ACK means validated and durably saved, not applied.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use crate::{FlowQuery, MeshNetwork, MeshNode, PolicyDiagnostic, RouteDecision, RoutingPolicy};

pub const CONFIGURATION_SCHEMA_VERSION: u32 = 1;
pub const MAX_CONFIGURATION_BYTES: usize = 1024 * 1024;
pub const MAX_INVENTORY_NODES: usize = 1024;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicySnapshot {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    pub network: MeshNetwork,
    pub policy: RoutingPolicy,
    pub valid: bool,
    pub diagnostics: Vec<PolicyDiagnostic>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewPolicyRequest {
    pub policy: RoutingPolicy,
    pub flow: FlowQuery,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewPolicyResponse {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    pub valid: bool,
    pub diagnostics: Vec<PolicyDiagnostic>,
    pub decision: RouteDecision,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SavePolicyRequest {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub expected_revision: u32,
    pub policy: RoutingPolicy,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetNodeCountryRequest {
    pub node_id: String,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub country: Option<String>,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub expected_revision: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeConfiguration {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub schema_version: u32,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    pub network: MeshNetwork,
    pub node: MeshNode,
    pub peers: Vec<MeshNode>,
    pub routing_policy: RoutingPolicy,
    pub policy_valid: bool,
    pub diagnostics: Vec<PolicyDiagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationSnapshot {
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub digest: String,
    pub payload: NodeConfiguration,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeConfigurationRequest {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub digest: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeConfigurationResponse {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub digest: String,
}

/// Compact UTF-8 JSON with recursively lexicographically sorted object keys.
/// Arrays retain semantic order; serialization is independent of DTO field order.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    fn sort(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(object) => serde_json::Value::Object(
                object
                    .into_iter()
                    .collect::<std::collections::BTreeMap<_, _>>()
                    .into_iter()
                    .map(|(key, value)| (key, sort(value)))
                    .collect(),
            ),
            serde_json::Value::Array(array) => {
                serde_json::Value::Array(array.into_iter().map(sort).collect())
            }
            value => value,
        }
    }
    serde_json::to_vec(&sort(serde_json::to_value(value)?))
}

pub fn configuration_digest(payload: &NodeConfiguration) -> Result<String, serde_json::Error> {
    Ok(Sha256::digest(canonical_json(payload)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::canonical_json;
    use serde::Serialize;

    #[test]
    fn canonical_json_sorts_nested_objects_but_preserves_array_order() {
        #[derive(Serialize)]
        struct NonAlphabetic {
            z: u32,
            a: u32,
        }
        let source = NonAlphabetic { z: 2, a: 1 };
        assert_eq!(canonical_json(&source).unwrap(), br#"{"a":1,"z":2}"#);
        let nested = serde_json::json!({"z":[{"z":2,"a":1},3,1],"a":null});
        assert_eq!(
            canonical_json(&nested).unwrap(),
            br#"{"a":null,"z":[{"a":1,"z":2},3,1]}"#
        );
    }
}
