CREATE TABLE nodes (id TEXT PRIMARY KEY, credential_hash BLOB NOT NULL UNIQUE,
 name TEXT NOT NULL, agent_version TEXT NOT NULL, registered_at TEXT NOT NULL,
 last_seen TEXT NOT NULL, revoked_at TEXT, mesh_ipv4 TEXT, mesh_ipv6 TEXT, country TEXT,
 acknowledged_revision INTEGER NOT NULL DEFAULT 0, acknowledged_digest TEXT);
CREATE UNIQUE INDEX node_ipv4 ON nodes(mesh_ipv4);
CREATE UNIQUE INDEX node_ipv6 ON nodes(mesh_ipv6);
CREATE TABLE enrollment_tokens (id TEXT PRIMARY KEY,token_hash BLOB NOT NULL UNIQUE,
 expires_at INTEGER NOT NULL,used_node_id TEXT REFERENCES nodes(id),binding_hash BLOB,
 device_binding TEXT,CHECK((used_node_id IS NULL)=(binding_hash IS NULL)));
CREATE TABLE configuration (id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL
 CHECK(revision BETWEEN 1 AND 4294967295),ipv4_prefix TEXT NOT NULL,ipv6_prefix TEXT NOT NULL,
 policy TEXT NOT NULL,source_jsonc TEXT);
CREATE TABLE network_identity (id INTEGER PRIMARY KEY CHECK(id=1),network_id TEXT NOT NULL UNIQUE
 CHECK(length(network_id)=32),name TEXT NOT NULL,dns TEXT NOT NULL);
CREATE TABLE connection_identities (node_id TEXT PRIMARY KEY REFERENCES nodes(id),public_key TEXT NOT NULL UNIQUE,capabilities TEXT NOT NULL);
CREATE TABLE connection_source (id INTEGER PRIMARY KEY CHECK(id=1),configuration TEXT NOT NULL);
CREATE TABLE connection_receipts (node_id TEXT PRIMARY KEY REFERENCES nodes(id),revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 4294967295),digest TEXT NOT NULL,saved_at TEXT NOT NULL);
CREATE TABLE connection_runtime_leases (node_id TEXT PRIMARY KEY REFERENCES nodes(id),instance_id TEXT NOT NULL,epoch INTEGER NOT NULL CHECK(epoch BETWEEN 1 AND 4294967295));
CREATE TABLE connection_runtime_reports (node_id TEXT PRIMARY KEY REFERENCES connection_runtime_leases(node_id),sequence INTEGER NOT NULL CHECK(sequence BETWEEN 1 AND 4294967295),body BLOB NOT NULL,reported_at TEXT NOT NULL);
CREATE TABLE mesh_access_source (id INTEGER PRIMARY KEY CHECK(id=1),policy TEXT NOT NULL,source_jsonc TEXT);
CREATE TABLE mesh_access_legacy_v5 (id INTEGER PRIMARY KEY CHECK(id=1),policy TEXT NOT NULL);
CREATE TABLE tag_definitions (tag TEXT PRIMARY KEY,description TEXT NOT NULL);
CREATE TABLE node_tags (node_id TEXT NOT NULL REFERENCES nodes(id),tag TEXT NOT NULL REFERENCES tag_definitions(tag),PRIMARY KEY(node_id,tag));
