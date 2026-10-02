CREATE TABLE network_policy_sources (
    network_id TEXT PRIMARY KEY REFERENCES networks(network_id),
    source_mode TEXT NOT NULL CHECK(source_mode IN ('legacy_v2','mesh_v3')),
    policy TEXT,
    source_jsonc TEXT,
    CHECK((source_mode='legacy_v2' AND policy IS NULL AND source_jsonc IS NULL) OR
          (source_mode='mesh_v3' AND policy IS NOT NULL))
);
CREATE TABLE mesh_execution_capabilities (
    node_id TEXT PRIMARY KEY,
    network_id TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch BETWEEN 1 AND 4294967295),
    instance_id TEXT NOT NULL,
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
);
CREATE TABLE mesh_execution_receipts (
    node_id TEXT PRIMARY KEY,
    network_id TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch BETWEEN 1 AND 4294967295),
    revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 4294967295),
    digest TEXT NOT NULL,
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
);
