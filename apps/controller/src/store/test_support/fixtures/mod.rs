//! Frozen historical database inputs, independent of current Store startup.
pub(crate) mod original_v1;
pub(crate) mod original_v8;
pub(crate) mod release;

// Frozen row contract, deliberately independent of the production copy list.
// Changing a migration must not change the historical input or its oracle.
pub(super) const SINGLE_NETWORK_COLUMNS: &[(&str, &str)] = &[
    (
        "nodes",
        "id,credential_hash,name,agent_version,registered_at,last_seen,revoked_at,mesh_ipv4,mesh_ipv6,country,acknowledged_revision,acknowledged_digest",
    ),
    (
        "enrollment_tokens",
        "id,token_hash,expires_at,used_node_id,binding_hash,device_binding",
    ),
    (
        "configuration",
        "id,revision,ipv4_prefix,ipv6_prefix,policy,source_jsonc",
    ),
    ("connection_identities", "node_id,public_key,capabilities"),
    ("connection_source", "id,configuration"),
    ("connection_receipts", "node_id,revision,digest,saved_at"),
    ("connection_runtime_leases", "node_id,instance_id,epoch"),
    (
        "connection_runtime_reports",
        "node_id,sequence,body,reported_at",
    ),
    ("mesh_access_source", "id,policy,source_jsonc"),
    ("mesh_access_legacy_v5", "id,policy"),
    ("tag_definitions", "tag,description"),
    ("node_tags", "node_id,tag"),
    ("network_identity", "id,network_id,name,dns"),
];
