CREATE TABLE dataset_artifacts (
    id TEXT PRIMARY KEY NOT NULL,
    network_id BLOB REFERENCES networks(id),
    kind TEXT NOT NULL CHECK(kind IN ('geoip', 'asn')),
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

-- Current deployment datasets are explicit references, never inferred from
-- wall-clock timestamps, random IDs or VACUUM-sensitive implicit rowids.
CREATE TABLE ip_dataset_heads (
    kind TEXT PRIMARY KEY NOT NULL CHECK(kind IN ('geoip', 'asn')),
    artifact_id TEXT NOT NULL REFERENCES dataset_artifacts(id)
) STRICT;
CREATE TRIGGER ip_dataset_heads_insert BEFORE INSERT ON ip_dataset_heads
WHEN NOT EXISTS(SELECT 1 FROM dataset_artifacts WHERE id=NEW.artifact_id AND kind=NEW.kind AND network_id IS NULL AND format='mmdb')
BEGIN SELECT RAISE(ABORT, 'invalid IP dataset head'); END;
CREATE TRIGGER ip_dataset_heads_update BEFORE UPDATE ON ip_dataset_heads
WHEN NEW.kind!=OLD.kind OR NOT EXISTS(SELECT 1 FROM dataset_artifacts WHERE id=NEW.artifact_id AND kind=NEW.kind AND network_id IS NULL AND format='mmdb')
BEGIN SELECT RAISE(ABORT, 'invalid IP dataset head'); END;
CREATE TRIGGER ip_dataset_heads_no_delete BEFORE DELETE ON ip_dataset_heads
BEGIN SELECT RAISE(ABORT, 'IP dataset heads require a replacement'); END;

-- Rule-set artifacts (`MWRS`), content addressed and shared by every network:
-- the SHA-256 of `data` is `digest`. Rows are immutable and retained.
CREATE TABLE rule_set_artifacts (
    digest BLOB PRIMARY KEY NOT NULL CHECK(length(digest)=32),
    data BLOB NOT NULL CHECK(length(data) BETWEEN 64 AND 16777216),
    created_at INTEGER NOT NULL CHECK(created_at>=0)
) STRICT;
CREATE TRIGGER rule_set_artifacts_immutable BEFORE UPDATE ON rule_set_artifacts
BEGIN SELECT RAISE(ABORT, 'rule-set artifacts are immutable'); END;
CREATE TRIGGER rule_set_artifacts_retained BEFORE DELETE ON rule_set_artifacts
BEGIN SELECT RAISE(ABORT, 'rule-set artifacts are retained'); END;

-- Where an artifact came from for one source (a remote URL and format, or a
-- catalog set): provenance and the conversion report stay out of the
-- artifact, so equal content keeps one digest. Immutable once written.
CREATE TABLE rule_set_records (
    source TEXT NOT NULL,
    digest BLOB NOT NULL REFERENCES rule_set_artifacts(digest),
    record TEXT NOT NULL CHECK(json_valid(record) AND json_type(record)='object'),
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    PRIMARY KEY(source, digest)
) STRICT;
CREATE TRIGGER rule_set_records_immutable BEFORE UPDATE ON rule_set_records
BEGIN SELECT RAISE(ABORT, 'rule-set records are immutable'); END;

-- Highest accepted `sequence` of each signed catalog under one trusted key
-- set, with the exact `index.json` and `index.json.sig` bytes verified for
-- it. A catalog whose sequence is lower is never used again.
CREATE TABLE rule_set_catalogs (
    catalog TEXT NOT NULL,
    trust TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK(sequence BETWEEN 0 AND 9007199254740991),
    verified_at INTEGER NOT NULL CHECK(verified_at>=0),
    document BLOB NOT NULL CHECK(length(document) BETWEEN 1 AND 33554432),
    signature BLOB NOT NULL CHECK(length(signature) BETWEEN 1 AND 4096),
    PRIMARY KEY(catalog, trust)
) STRICT;
CREATE TRIGGER rule_set_catalogs_monotonic BEFORE UPDATE ON rule_set_catalogs
WHEN NEW.sequence < OLD.sequence OR NEW.catalog != OLD.catalog OR NEW.trust != OLD.trust
BEGIN SELECT RAISE(ABORT, 'catalog sequence must not decrease'); END;

-- Current check outcome is separate from immutable configuration and publication
-- bindings. A failed refresh retains the last successfully published artifact.
CREATE TABLE rule_set_checks (
    network_id BLOB NOT NULL,
    revision INTEGER NOT NULL,
    rule_set_key TEXT NOT NULL,
    problem TEXT,
    checked_at INTEGER NOT NULL,
    last_success_at INTEGER,
    failures INTEGER NOT NULL DEFAULT 0 CHECK(failures BETWEEN 0 AND 16),
    check_version INTEGER NOT NULL DEFAULT 1 CHECK(check_version BETWEEN 1 AND 9007199254740991),
    PRIMARY KEY(network_id, revision, rule_set_key),
    FOREIGN KEY(network_id, revision) REFERENCES configuration_versions(network_id, revision)
) STRICT;

-- Current rule-set binding of one configuration version, after the version's
-- initial binding was refreshed. Absent: the version's own bindings apply.
CREATE TABLE rule_set_bindings (
    network_id BLOB NOT NULL,
    revision INTEGER NOT NULL,
    bindings TEXT NOT NULL CHECK(json_valid(bindings) AND json_type(bindings)='object'),
    bound_at INTEGER NOT NULL CHECK(bound_at>=0),
    PRIMARY KEY(network_id, revision),
    FOREIGN KEY(network_id, revision) REFERENCES configuration_versions(network_id, revision)
) STRICT;

-- Manual refreshes are idempotent per (network, user, request ID): a retry
-- with the same key returns the current view without downloading again.
-- Rows are kept for one day; a request ID seen after that refreshes anew.
CREATE TABLE rule_set_refreshes (
    network_id BLOB NOT NULL REFERENCES networks(id),
    user_id BLOB NOT NULL REFERENCES users(id),
    request_id BLOB NOT NULL CHECK (length(request_id) = 16 AND request_id != zeroblob(16)),
    rule_set_key TEXT NOT NULL,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    PRIMARY KEY(network_id, user_id, request_id)
) STRICT;
CREATE INDEX rule_set_refreshes_created_at ON rule_set_refreshes(created_at);

-- Consumed subscriber identifiers survive volatile task loss. This is an
-- idempotency tombstone, not a persisted task, permit or authorization cache.
CREATE TABLE observation_subscriber_ids (
    id BLOB NOT NULL PRIMARY KEY CHECK(length(id)=16 AND id!=zeroblob(16))
) STRICT;
CREATE TRIGGER observation_subscriber_ids_immutable BEFORE UPDATE ON observation_subscriber_ids
BEGIN SELECT RAISE(ABORT,'immutable subscriber identifier'); END;
CREATE TRIGGER observation_subscriber_ids_retained BEFORE DELETE ON observation_subscriber_ids
BEGIN SELECT RAISE(ABORT,'subscriber identifiers are retained'); END;

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
