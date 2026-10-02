-- Versionless migration fragment. Install transactionally at controller startup.
CREATE TABLE IF NOT EXISTS latency_runtime_epochs (
    node_id TEXT PRIMARY KEY,
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    epoch INTEGER NOT NULL,
    started_ms INTEGER NOT NULL,
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
);
CREATE TABLE IF NOT EXISTS latency_samples (
    row_id INTEGER PRIMARY KEY AUTOINCREMENT,
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    source_node_id TEXT NOT NULL,
    peer_node_id TEXT NOT NULL,
    measurement_id TEXT NOT NULL,
    runtime_epoch INTEGER NOT NULL,
    measured_ms INTEGER NOT NULL,
    received_ms INTEGER NOT NULL,
    body BLOB NOT NULL,
    UNIQUE(network_id, source_node_id, measurement_id),
    FOREIGN KEY(network_id,source_node_id) REFERENCES nodes(network_id,id),
    FOREIGN KEY(network_id,peer_node_id) REFERENCES nodes(network_id,id)
);
CREATE INDEX IF NOT EXISTS latency_pair_time
    ON latency_samples(network_id, source_node_id, peer_node_id, measured_ms, row_id);
CREATE INDEX IF NOT EXISTS latency_source_receive
    ON latency_samples(network_id, source_node_id, received_ms);
CREATE INDEX IF NOT EXISTS latency_retention ON latency_samples(measured_ms);
