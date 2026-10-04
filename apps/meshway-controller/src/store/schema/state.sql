CREATE TABLE dataset_artifacts (
    id TEXT PRIMARY KEY NOT NULL,
    network_id BLOB REFERENCES networks(id),
    kind TEXT NOT NULL CHECK(kind IN ('ruleSet', 'geoip', 'asn')),
    source TEXT NOT NULL,
    format TEXT NOT NULL,
    data BLOB NOT NULL,
    digest TEXT NOT NULL CHECK(length(digest) = 64),
    created_at INTEGER NOT NULL,
    UNIQUE(network_id, kind, digest)
) STRICT;

CREATE TRIGGER dataset_artifacts_immutable_update BEFORE UPDATE ON dataset_artifacts
BEGIN SELECT RAISE(ABORT, 'dataset artifacts are immutable'); END;
CREATE TRIGGER dataset_artifacts_immutable_delete BEFORE DELETE ON dataset_artifacts
BEGIN SELECT RAISE(ABORT, 'dataset artifacts are retained by version'); END;

-- Current check outcome is separate from immutable configuration and publication
-- bindings. A failed refresh retains the last successfully published artifact.
CREATE TABLE rule_set_checks (
    network_id BLOB NOT NULL,
    revision INTEGER NOT NULL,
    rule_set_key TEXT NOT NULL,
    problem TEXT,
    checked_at INTEGER NOT NULL,
    last_success_at INTEGER,
    PRIMARY KEY(network_id, revision, rule_set_key),
    FOREIGN KEY(network_id, revision) REFERENCES configuration_versions(network_id, revision)
) STRICT;

CREATE TABLE observation_events (
    row_id INTEGER PRIMARY KEY AUTOINCREMENT,
    network_id BLOB NOT NULL REFERENCES networks(id),
    source_node_id TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    occurred_at INTEGER NOT NULL,
    received_at INTEGER NOT NULL,
    payload BLOB,
    payload_digest TEXT NOT NULL CHECK(length(payload_digest) = 64),
    admission_expires_at INTEGER NOT NULL,
    UNIQUE(network_id, source_node_id, event_id)
) STRICT;
CREATE INDEX observation_window ON observation_events(network_id, occurred_at, row_id);
CREATE TABLE history_coverage (
    network_id BLOB NOT NULL REFERENCES networks(id),
    source_node_id TEXT NOT NULL,
    before_at INTEGER NOT NULL,
    reason TEXT NOT NULL CHECK(reason IN ('retention', 'capacity')),
    PRIMARY KEY(network_id, source_node_id, reason)
) STRICT;
