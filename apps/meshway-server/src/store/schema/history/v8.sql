-- Committed databases always contain this one immutable binding. The private
-- key file is durably published before this row and the schema commit together.
CREATE TABLE master_key_binding (
    singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
    key_id BLOB NOT NULL UNIQUE CHECK (length(key_id) = 16 AND key_id != zeroblob(16)),
    key_check BLOB NOT NULL CHECK (length(key_check) = 61)
) STRICT;

CREATE TRIGGER master_key_binding_no_update BEFORE UPDATE ON master_key_binding
BEGIN SELECT RAISE(ABORT, 'immutable master key binding'); END;
CREATE TRIGGER master_key_binding_no_delete BEFORE DELETE ON master_key_binding
BEGIN SELECT RAISE(ABORT, 'immutable master key binding'); END;

-- All timestamp INTEGER fields in users, user_sessions, and activity_events
-- store Unix milliseconds (UTC), including nullable lifecycle timestamps.
CREATE TABLE users (
    id BLOB NOT NULL PRIMARY KEY CHECK (length(id) = 16 AND id != zeroblob(16)),
    email TEXT NOT NULL UNIQUE CHECK (length(email) BETWEEN 3 AND 254 AND email = lower(trim(email))),
    -- This is only a format prefix guard. The account service must parse and
    -- validate the complete Argon2id PHC before storing or verifying it.
    password_hash TEXT NOT NULL CHECK (password_hash GLOB '$argon2id$v=19$*'),
    credential_version INTEGER NOT NULL CHECK (credential_version BETWEEN 1 AND 9007199254740991),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    disabled_at INTEGER CHECK (disabled_at >= created_at)
) STRICT;

CREATE TABLE user_sessions (
    id BLOB NOT NULL PRIMARY KEY CHECK (length(id) = 16 AND id != zeroblob(16)),
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    user_id BLOB NOT NULL REFERENCES users(id) ON DELETE RESTRICT CHECK (length(user_id) = 16 AND user_id != zeroblob(16)),
    credential_version INTEGER NOT NULL CHECK (credential_version BETWEEN 1 AND 9007199254740991),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    last_used_at INTEGER CHECK (last_used_at >= created_at),
    client_label TEXT,
    expires_at INTEGER NOT NULL CHECK (expires_at > created_at),
    revoked_at INTEGER CHECK (revoked_at >= created_at),
    -- Set for a session issued to a delegation client through an authorization
    -- code; such sessions cannot manage the account or other sessions.
    delegation_client TEXT CHECK (length(delegation_client) BETWEEN 1 AND 32),
    UNIQUE (id, user_id)
) STRICT;

CREATE INDEX user_sessions_user_id ON user_sessions(user_id);

-- One-time authorization codes a signed-in user issues for a delegation
-- client. The code is stored hashed; the PKCE challenge binds the exchange.
CREATE TABLE delegation_codes (
    code_hash BLOB NOT NULL PRIMARY KEY CHECK (length(code_hash) = 32),
    user_id BLOB NOT NULL REFERENCES users(id) ON DELETE RESTRICT CHECK (length(user_id) = 16 AND user_id != zeroblob(16)),
    -- The user session that authorized; the exchange requires it to still be valid.
    session_id BLOB NOT NULL CHECK (length(session_id) = 16 AND session_id != zeroblob(16)),
    client TEXT NOT NULL CHECK (length(client) BETWEEN 1 AND 32),
    redirect_uri TEXT NOT NULL CHECK (length(redirect_uri) BETWEEN 1 AND 2048),
    code_challenge TEXT NOT NULL CHECK (length(code_challenge) = 43),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    expires_at INTEGER NOT NULL CHECK (expires_at > created_at),
    consumed_at INTEGER CHECK (consumed_at >= created_at),
    FOREIGN KEY (session_id, user_id) REFERENCES user_sessions(id, user_id) ON DELETE RESTRICT
) STRICT;

CREATE INDEX delegation_codes_expires_at ON delegation_codes(expires_at);

CREATE TABLE password_resets (
    token_hash BLOB NOT NULL PRIMARY KEY CHECK (length(token_hash) = 32),
    user_id BLOB NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    credential_version INTEGER NOT NULL CHECK (credential_version BETWEEN 1 AND 9007199254740991),
    expires_at INTEGER NOT NULL CHECK (expires_at > 0),
    revoked_at INTEGER,
    consumed_at INTEGER
) STRICT;

CREATE TABLE networks (
    sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT CHECK (sequence BETWEEN 1 AND 9007199254740991),
    id BLOB NOT NULL UNIQUE CHECK (length(id) = 16 AND id != zeroblob(16)),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 128 AND name = trim(name)),
    created_by BLOB NOT NULL REFERENCES users(id) ON DELETE RESTRICT CHECK (length(created_by) = 16 AND created_by != zeroblob(16)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    creation_request_id BLOB NOT NULL CHECK (length(creation_request_id) = 16 AND creation_request_id != zeroblob(16)),
    creation_request_digest BLOB NOT NULL CHECK (length(creation_request_digest) = 32),
    deleted_at INTEGER CHECK (deleted_at >= created_at),
    current_revision INTEGER NOT NULL CHECK (current_revision BETWEEN 1 AND 9007199254740991),
    publication_sequence INTEGER NOT NULL CHECK (publication_sequence BETWEEN 0 AND 9007199254740991),
    UNIQUE (created_by, creation_request_id),
    FOREIGN KEY (id, current_revision) REFERENCES configuration_versions(network_id, revision) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
) STRICT;

CREATE TABLE memberships (
    sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT CHECK (sequence BETWEEN 1 AND 9007199254740991),
    id BLOB NOT NULL UNIQUE CHECK (length(id) = 16 AND id != zeroblob(16)),
    network_id BLOB NOT NULL REFERENCES networks(id) ON DELETE RESTRICT CHECK (length(network_id) = 16 AND network_id != zeroblob(16)),
    user_id BLOB NOT NULL REFERENCES users(id) ON DELETE RESTRICT CHECK (length(user_id) = 16 AND user_id != zeroblob(16)),
    role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    version INTEGER NOT NULL CHECK (version BETWEEN 1 AND 9007199254740991),
    joined_at INTEGER NOT NULL CHECK (joined_at >= 0),
    revoked_at INTEGER CHECK (revoked_at >= joined_at),
    UNIQUE (network_id, id),
    UNIQUE (network_id, id, user_id)
) STRICT;
CREATE UNIQUE INDEX memberships_active_user ON memberships(network_id, user_id) WHERE revoked_at IS NULL;
CREATE INDEX memberships_user_network ON memberships(user_id, network_id) WHERE revoked_at IS NULL;
CREATE INDEX memberships_network_sequence ON memberships(network_id, sequence) WHERE revoked_at IS NULL;

CREATE TABLE join_links (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT CHECK (sequence BETWEEN 1 AND 9007199254740991),
    id BLOB NOT NULL UNIQUE CHECK(length(id)=16 AND id!=zeroblob(16)),
    network_id BLOB NOT NULL REFERENCES networks(id),
    role TEXT NOT NULL CHECK(role IN ('owner','admin','member')),
    token_hash BLOB NOT NULL UNIQUE CHECK(length(token_hash)=32),
    created_by BLOB NOT NULL REFERENCES users(id),
    issuer_membership_id BLOB NOT NULL,
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    expires_at INTEGER NOT NULL CHECK(expires_at>created_at),
    revoked_at INTEGER CHECK(revoked_at>=created_at),
    consumed_at INTEGER CHECK(consumed_at>=created_at),
    consumed_by_user_id BLOB REFERENCES users(id),
    consumed_membership_id BLOB,
    creation_request_id BLOB NOT NULL CHECK(length(creation_request_id)=16),
    creation_request_digest BLOB NOT NULL CHECK(length(creation_request_digest)=32),
    UNIQUE(network_id,created_by,creation_request_id),
    FOREIGN KEY(network_id,issuer_membership_id,created_by) REFERENCES memberships(network_id,id,user_id),
    FOREIGN KEY(network_id,consumed_membership_id,consumed_by_user_id) REFERENCES memberships(network_id,id,user_id),
    CHECK((consumed_at IS NULL AND consumed_by_user_id IS NULL AND consumed_membership_id IS NULL) OR (consumed_at IS NOT NULL AND consumed_by_user_id IS NOT NULL AND consumed_membership_id IS NOT NULL))
) STRICT;

CREATE INDEX join_links_network_sequence ON join_links(network_id,sequence);
CREATE TABLE configuration_versions (
    network_id BLOB NOT NULL REFERENCES networks(id) ON DELETE RESTRICT CHECK (length(network_id) = 16 AND network_id != zeroblob(16)),
    revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
    source_jsonc BLOB NOT NULL CHECK (length(source_jsonc) >= 61),
    configuration BLOB NOT NULL CHECK (length(configuration) >= 61),
    bindings BLOB NOT NULL CHECK (length(bindings) >= 61),
    created_by BLOB NOT NULL REFERENCES users(id) ON DELETE RESTRICT CHECK (length(created_by) = 16 AND created_by != zeroblob(16)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    cause TEXT NOT NULL CHECK (cause IN ('networkCreated','edit','restore','enrollment','nodeDeleted','emailSync','memberRemoved','identityTransition')),
    request_id BLOB NOT NULL CHECK (length(request_id) = 16 AND request_id != zeroblob(16)),
    rollback_from INTEGER,
    execution_targets TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(execution_targets) AND json_type(execution_targets)='object'),
    created_by_email TEXT,
    PRIMARY KEY (network_id, revision),
    FOREIGN KEY(network_id,rollback_from) REFERENCES configuration_versions(network_id,revision)
) STRICT;
CREATE TRIGGER configuration_versions_no_update BEFORE UPDATE OF network_id,revision,source_jsonc,configuration,bindings,created_by,created_by_email,created_at,cause,request_id,rollback_from ON configuration_versions
BEGIN SELECT RAISE(ABORT, 'immutable configuration version'); END;
-- The apply transaction fills targets after compiling against its new source,
-- then seals the version with its activity row before committing.
CREATE TRIGGER configuration_targets_no_update BEFORE UPDATE OF execution_targets ON configuration_versions
WHEN OLD.cause='networkCreated' OR OLD.execution_targets!='{}' OR EXISTS (
    SELECT 1 FROM activity_events WHERE network_id=OLD.network_id AND after_revision=OLD.revision
)
BEGIN SELECT RAISE(ABORT, 'immutable configuration targets'); END;
CREATE TRIGGER configuration_versions_no_delete BEFORE DELETE ON configuration_versions
BEGIN SELECT RAISE(ABORT, 'immutable configuration version'); END;

CREATE TABLE configuration_commits (
    network_id BLOB NOT NULL REFERENCES networks(id),
    user_id BLOB NOT NULL REFERENCES users(id),
    request_id BLOB NOT NULL CHECK(length(request_id)=16),
    base_revision INTEGER NOT NULL CHECK(base_revision BETWEEN 1 AND 9007199254740991),
    input BLOB NOT NULL CHECK(length(input)>=61),
    result TEXT NOT NULL,
    PRIMARY KEY(network_id,user_id,request_id)
) STRICT;

CREATE TABLE credential_revocations (
    network_id BLOB NOT NULL REFERENCES networks(id),
    kind TEXT NOT NULL CHECK(kind IN ('vlessUuid','trojanPassword','tlsCertificate')),
    fingerprint BLOB NOT NULL CHECK(length(fingerprint)=32),
    revoked_at INTEGER NOT NULL CHECK(revoked_at>=0),
    revoked_by BLOB NOT NULL REFERENCES users(id),
    PRIMARY KEY(network_id,kind,fingerprint)
) STRICT;
CREATE TRIGGER credential_revocations_no_update BEFORE UPDATE ON credential_revocations
BEGIN SELECT RAISE(ABORT, 'irreversible credential revocation'); END;
CREATE TRIGGER credential_revocations_no_delete BEFORE DELETE ON credential_revocations
BEGIN SELECT RAISE(ABORT, 'irreversible credential revocation'); END;

-- Successful account/network mutations and their activities commit together.
CREATE TABLE activity_events (
    sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT CHECK (sequence BETWEEN 1 AND 9007199254740991),
    id BLOB NOT NULL UNIQUE CHECK (length(id) = 16 AND id != zeroblob(16)),
    scope TEXT NOT NULL CHECK (scope IN ('account', 'network')),
    account_user_id BLOB REFERENCES users(id) ON DELETE RESTRICT CHECK (length(account_user_id) = 16 AND account_user_id != zeroblob(16)),
    network_id BLOB REFERENCES networks(id) ON DELETE RESTRICT CHECK (length(network_id) = 16 AND network_id != zeroblob(16)),
    at INTEGER NOT NULL CHECK (at >= 0),
    actor_kind TEXT NOT NULL DEFAULT 'user' CHECK(actor_kind IN ('user','system','node')),
    actor_user_id BLOB REFERENCES users(id) ON DELETE RESTRICT CHECK (length(actor_user_id) = 16 AND actor_user_id != zeroblob(16)),
    actor_node_id BLOB CHECK (length(actor_node_id) = 16 AND actor_node_id != zeroblob(16)),
    actor_email TEXT,
    action TEXT NOT NULL CHECK (action IN ('account.register', 'session.login', 'session.logout', 'session.revoke', 'session.revokeOthers', 'session.revokeAll', 'session.delegate', 'account.password', 'account.passwordReset', 'account.resetIssued', 'account.email', 'network.create', 'network.rename', 'configuration.apply','membership.role','membership.join','membership.remove','joinLink.create','joinLink.revoke','routing.select','routing.autoResolve')),
    target_user_id BLOB REFERENCES users(id) ON DELETE RESTRICT,
    target_session_id BLOB CHECK (length(target_session_id) = 16 AND target_session_id != zeroblob(16)),
    target_membership_id BLOB,
    target_join_link_id BLOB REFERENCES join_links(id),
    target_node_id BLOB CHECK (length(target_node_id) = 16 AND target_node_id != zeroblob(16)),
    target_profile TEXT,
    before_revision INTEGER,
    after_revision INTEGER,
    request_id BLOB NOT NULL CHECK (length(request_id) = 16 AND request_id != zeroblob(16)),
    previous_email TEXT CHECK (previous_email IS NULL OR action = 'account.email'),
    CHECK ((actor_kind='user' AND actor_user_id IS NOT NULL AND actor_node_id IS NULL) OR (actor_kind='system' AND actor_user_id IS NULL AND actor_node_id IS NULL)
        OR (actor_kind='node' AND actor_user_id IS NULL AND actor_node_id IS NOT NULL AND action='routing.select')),
    CHECK ((action IN ('routing.select','routing.autoResolve')) = (target_node_id IS NOT NULL AND target_profile IS NOT NULL)),
    CHECK ((scope = 'account' AND account_user_id IS NOT NULL AND network_id IS NULL AND (actor_user_id = account_user_id OR (actor_kind='system' AND action='account.resetIssued')) AND
        ((action IN ('account.register', 'session.revokeOthers', 'session.revokeAll', 'account.password', 'account.passwordReset', 'account.resetIssued', 'account.email') AND target_user_id IS NOT NULL AND target_user_id = account_user_id AND target_session_id IS NULL)
        OR (action IN ('session.login', 'session.logout', 'session.revoke', 'session.delegate') AND target_user_id IS NULL AND target_session_id IS NOT NULL)))
        OR (scope = 'network' AND network_id IS NOT NULL AND account_user_id IS NULL AND action IN ('network.create', 'network.rename','configuration.apply','membership.role','membership.join','membership.remove','joinLink.create','joinLink.revoke','routing.select','routing.autoResolve') AND target_user_id IS NULL AND target_session_id IS NULL)),
    FOREIGN KEY (target_session_id, account_user_id) REFERENCES user_sessions(id, user_id) ON DELETE RESTRICT,
    FOREIGN KEY (network_id,target_membership_id) REFERENCES memberships(network_id,id),
    FOREIGN KEY (network_id,target_node_id) REFERENCES nodes(network_id,id),
    FOREIGN KEY (network_id,actor_node_id) REFERENCES nodes(network_id,id),
    FOREIGN KEY (network_id,before_revision) REFERENCES configuration_versions(network_id,revision),
    FOREIGN KEY (network_id,after_revision) REFERENCES configuration_versions(network_id,revision)
) STRICT;

CREATE INDEX activity_events_account_sequence ON activity_events(account_user_id, sequence);
CREATE INDEX activity_events_network_sequence ON activity_events(network_id, sequence);
-- A user's routing selection is idempotent per (network, user, request ID):
-- its activity row is the retry record.
CREATE UNIQUE INDEX activity_events_routing_request ON activity_events(network_id, actor_user_id, request_id)
    WHERE action = 'routing.select' AND actor_kind = 'user';

-- Node identity, key generations and instances.
-- Node identity is only in Configuration. These rows contain immutable admission
-- binding, independent key generations and irreversible revocation facts.
CREATE TABLE nodes (
    id BLOB NOT NULL PRIMARY KEY CHECK(length(id)=16 AND id!=zeroblob(16)),
    network_id BLOB NOT NULL REFERENCES networks(id),
    registered_at INTEGER NOT NULL CHECK(registered_at>=0),
    credentials_revoked_at INTEGER CHECK(credentials_revoked_at>=registered_at),
    UNIQUE(network_id,id)
) STRICT;
-- Unknown teardown bounds intentionally stay NULL. A saved/applied receipt is
-- not evidence that old DNS/NAT/reply leases can no longer use this binding.
CREATE TABLE address_retirements (
    network_id BLOB NOT NULL,
    address TEXT NOT NULL,
    previous_node_id BLOB NOT NULL,
    removed_revision INTEGER NOT NULL,
    release_after INTEGER,
    PRIMARY KEY(network_id,address,removed_revision),
    FOREIGN KEY(network_id,previous_node_id) REFERENCES nodes(network_id,id),
    FOREIGN KEY(network_id,removed_revision) REFERENCES configuration_versions(network_id,revision)
) STRICT;
CREATE TABLE node_keys (
    id BLOB NOT NULL PRIMARY KEY CHECK(length(id)=16 AND id!=zeroblob(16)),
    network_id BLOB NOT NULL,
    node_id BLOB NOT NULL,
    purpose TEXT NOT NULL CHECK(purpose IN ('control','dataplane')),
    algorithm TEXT NOT NULL CHECK((purpose='control' AND algorithm='ed25519') OR (purpose='dataplane' AND algorithm='x25519')),
    public_key BLOB NOT NULL UNIQUE CHECK(length(public_key)=32 AND public_key!=zeroblob(32)),
    generation INTEGER NOT NULL CHECK(generation BETWEEN 1 AND 9007199254740991),
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    retired_at INTEGER CHECK(retired_at>=created_at),
    revoked_at INTEGER CHECK(revoked_at>=created_at),
    UNIQUE(network_id,node_id,purpose,generation),
    UNIQUE(network_id,node_id,id),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
) STRICT;
CREATE UNIQUE INDEX node_keys_current ON node_keys(network_id,node_id,purpose) WHERE retired_at IS NULL AND revoked_at IS NULL;
CREATE TRIGGER nodes_irreversible BEFORE UPDATE OF credentials_revoked_at ON nodes
WHEN OLD.credentials_revoked_at IS NOT NULL AND NEW.credentials_revoked_at IS NOT OLD.credentials_revoked_at
BEGIN SELECT RAISE(ABORT,'irreversible node revocation'); END;
CREATE TRIGGER node_keys_irreversible BEFORE UPDATE OF revoked_at,retired_at ON node_keys
WHEN (OLD.revoked_at IS NOT NULL AND NEW.revoked_at IS NOT OLD.revoked_at) OR (OLD.retired_at IS NOT NULL AND NEW.retired_at IS NOT OLD.retired_at)
BEGIN SELECT RAISE(ABORT,'irreversible key retirement'); END;
-- This replay boundary survives both process restart and challenge GC.
CREATE TABLE node_challenge_clock (
    singleton INTEGER NOT NULL PRIMARY KEY CHECK(singleton=1),
    utc_ms INTEGER NOT NULL CHECK(utc_ms>=0)
) STRICT;
INSERT INTO node_challenge_clock VALUES(1,0);
CREATE TRIGGER node_challenge_clock_no_replace BEFORE INSERT ON node_challenge_clock
WHEN EXISTS(SELECT 1 FROM node_challenge_clock)
BEGIN SELECT RAISE(ABORT, 'immutable challenge clock identity'); END;
CREATE TRIGGER node_challenge_clock_monotonic BEFORE UPDATE ON node_challenge_clock
WHEN NEW.singleton!=OLD.singleton OR NEW.utc_ms<OLD.utc_ms
BEGIN SELECT RAISE(ABORT, 'challenge clock cannot regress'); END;
CREATE TRIGGER node_challenge_clock_no_delete BEFORE DELETE ON node_challenge_clock
BEGIN SELECT RAISE(ABORT, 'challenge clock cannot be deleted'); END;
CREATE TABLE node_challenges (
    id BLOB NOT NULL PRIMARY KEY CHECK(length(id)=16),
    controller_id BLOB NOT NULL CHECK(length(controller_id)=16),
    network_id BLOB NOT NULL,
    node_id BLOB NOT NULL,
    key_id BLOB NOT NULL,
    nonce BLOB NOT NULL CHECK(length(nonce)=32),
    request_digest BLOB NOT NULL CHECK(length(request_digest)=32),
    operation TEXT NOT NULL CHECK(operation IN ('session','rotate')),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK(expires_at>created_at),
    consumed_at INTEGER,
    session_anchor INTEGER NOT NULL DEFAULT 0 CHECK(session_anchor IN (0,1) AND (session_anchor=0 OR (operation='session' AND consumed_at IS NOT NULL))),
    FOREIGN KEY(network_id,node_id,key_id) REFERENCES node_keys(network_id,node_id,id)
) STRICT;
CREATE TABLE node_sessions (
    token_hash BLOB NOT NULL PRIMARY KEY CHECK(length(token_hash)=32),
    network_id BLOB NOT NULL,
    node_id BLOB NOT NULL,
    key_id BLOB NOT NULL,
    boot_id BLOB NOT NULL CHECK(length(boot_id)=16),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK(expires_at>created_at),
    FOREIGN KEY(network_id,node_id,key_id) REFERENCES node_keys(network_id,node_id,id)
) STRICT;
-- Recovery contains public metadata only. The signed original challenge and
-- request digest bind both key generations; current keys are rechecked on read.
CREATE TABLE node_rotations (
    network_id BLOB NOT NULL,
    node_id BLOB NOT NULL,
    request_id BLOB NOT NULL CHECK(length(request_id)=16),
    request_digest BLOB NOT NULL CHECK(length(request_digest)=32),
    challenge_id BLOB NOT NULL UNIQUE CHECK(length(challenge_id)=16),
    challenge BLOB NOT NULL,
    control_key_id BLOB NOT NULL,
    dataplane_key_id BLOB NOT NULL,
    generation INTEGER NOT NULL CHECK(generation BETWEEN 1 AND 9007199254740991),
    result BLOB NOT NULL,
    PRIMARY KEY(network_id,node_id,request_id),
    FOREIGN KEY(network_id,node_id,control_key_id) REFERENCES node_keys(network_id,node_id,id),
    FOREIGN KEY(network_id,node_id,dataplane_key_id) REFERENCES node_keys(network_id,node_id,id)
) STRICT;
CREATE TABLE node_execution (
    network_id BLOB NOT NULL,
    node_id BLOB NOT NULL,
    instance_id BLOB CHECK(length(instance_id)=16),
    generation INTEGER NOT NULL DEFAULT 0 CHECK(generation BETWEEN 0 AND 9007199254740991),
    boot_id BLOB CHECK(length(boot_id)=16),
    instance_expires_at INTEGER,
    capabilities BLOB NOT NULL,
    PRIMARY KEY(network_id,node_id),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
) STRICT;
CREATE TABLE node_instance_claims (
    network_id BLOB NOT NULL,
    node_id BLOB NOT NULL,
    instance_id BLOB NOT NULL CHECK(length(instance_id)=16),
    generation INTEGER NOT NULL CHECK(generation BETWEEN 1 AND 9007199254740991),
    PRIMARY KEY(network_id,node_id,instance_id),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
) STRICT;
CREATE TABLE routing_selections (
    network_id BLOB NOT NULL,
    node_id BLOB NOT NULL,
    identity_digest BLOB NOT NULL CHECK(length(identity_digest)=32),
    profile TEXT NOT NULL,
    version INTEGER NOT NULL CHECK(version BETWEEN 1 AND 9007199254740991),
    selected_at INTEGER NOT NULL,
    PRIMARY KEY(network_id,node_id),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
) STRICT;
CREATE TABLE enrollment_keys (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT CHECK(sequence BETWEEN 1 AND 9007199254740991),
    id BLOB NOT NULL UNIQUE CHECK(length(id)=16),
    network_id BLOB NOT NULL REFERENCES networks(id),
    token_hash BLOB NOT NULL UNIQUE CHECK(length(token_hash)=32),
    tags BLOB NOT NULL,
    initial_settings BLOB NOT NULL CHECK(length(initial_settings)>=61),
    created_by BLOB NOT NULL REFERENCES users(id),
    membership_id BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK(expires_at>created_at),
    revoked_at INTEGER,
    consumed_request_id BLOB,
    creation_request_id BLOB NOT NULL CHECK(length(creation_request_id)=16),
    creation_request_digest BLOB NOT NULL CHECK(length(creation_request_digest)=32),
    UNIQUE(network_id,created_by,creation_request_id),
    FOREIGN KEY(network_id,membership_id,created_by) REFERENCES memberships(network_id,id,user_id)
) STRICT;
CREATE INDEX enrollment_keys_network_sequence ON enrollment_keys(network_id,sequence);
CREATE TABLE enrollment_requests (
    id BLOB NOT NULL PRIMARY KEY CHECK(length(id)=16),
    control_key BLOB NOT NULL CHECK(length(control_key)=32),
    dataplane_key BLOB NOT NULL CHECK(length(dataplane_key)=32),
    request_digest BLOB NOT NULL CHECK(length(request_digest)=32),
    code_challenge TEXT NOT NULL,
    hostname TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK(expires_at>created_at),
    approval BLOB,
    result_hash BLOB CHECK(length(result_hash)=32),
    delivery BLOB,
    denied_at INTEGER,
    cancelled_at INTEGER CHECK(cancelled_at>=created_at),
    consumed_at INTEGER,
    node_id BLOB REFERENCES nodes(id),
    binding_digest BLOB CHECK(length(binding_digest)=32),
    existing_binding BLOB,
    CHECK(denied_at IS NULL OR cancelled_at IS NULL)
) STRICT;

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
-- the SHA-256 of `data` is `digest`. Rows are immutable; one no configuration
-- version can restore and no netmap still under lease references is collected
-- (`resources::artifacts::collect`) together with its records.
CREATE TABLE rule_set_artifacts (
    digest BLOB PRIMARY KEY NOT NULL CHECK(length(digest)=32),
    data BLOB NOT NULL CHECK(length(data) BETWEEN 64 AND 16777216),
    created_at INTEGER NOT NULL CHECK(created_at>=0)
) STRICT;
CREATE TRIGGER rule_set_artifacts_immutable BEFORE UPDATE ON rule_set_artifacts
BEGIN SELECT RAISE(ABORT, 'rule-set artifacts are immutable'); END;

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

-- The record the latest successful preparation of one source gave `digest`
-- (catalog sequence, upstream, held set and report): a refresh that yields
-- the same digest replaces it, while rule_set_records keeps the first one.
CREATE TABLE rule_set_record_heads (
    source TEXT NOT NULL,
    digest BLOB NOT NULL REFERENCES rule_set_artifacts(digest),
    record TEXT NOT NULL CHECK(json_valid(record) AND json_type(record)='object'),
    updated_at INTEGER NOT NULL CHECK(updated_at>=0),
    PRIMARY KEY(source, digest)
) STRICT;

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

-- The Server's own last fetch of each effective catalog (URL, trusted key
-- set), independent of any set: schedules the next one and its backoff. The
-- accepted catalog stays in rule_set_catalogs; a failure keeps it.
CREATE TABLE rule_set_catalog_checks (
    catalog TEXT NOT NULL,
    trust TEXT NOT NULL,
    problem TEXT,
    checked_at INTEGER NOT NULL CHECK(checked_at>=0),
    failures INTEGER NOT NULL CHECK(failures BETWEEN 0 AND 16),
    PRIMARY KEY(catalog, trust)
) STRICT;

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

-- Per-Node netmap content. A Node keeps its current serial and the one before
-- it (the delta base). Content rows are immutable; a withdrawn head keeps the
-- serial high-water so a Node that returns never sees its serial regress.
-- Compact immutable references outlive the two retained payloads, so receipt
-- history never pins an obsolete configuration body or blocks a new serial.
CREATE TABLE node_netmap_inputs (
    network_id BLOB NOT NULL,
    node_id BLOB NOT NULL,
    digest BLOB NOT NULL CHECK(length(digest)=32),
    PRIMARY KEY(network_id,node_id),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
) STRICT;

CREATE TABLE node_netmap_history (
    network_id BLOB NOT NULL,
    node_id BLOB NOT NULL,
    serial INTEGER NOT NULL CHECK(serial BETWEEN 1 AND 9007199254740991),
    revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
    digest BLOB NOT NULL CHECK(length(digest)=32),
    PRIMARY KEY(network_id,node_id,serial),
    UNIQUE(network_id,node_id,serial,revision,digest),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id),
    FOREIGN KEY(network_id,revision) REFERENCES configuration_versions(network_id,revision)
) STRICT;
CREATE TRIGGER node_netmap_history_immutable BEFORE UPDATE ON node_netmap_history
BEGIN SELECT RAISE(ABORT,'immutable netmap reference'); END;
CREATE TRIGGER node_netmap_history_retained BEFORE DELETE ON node_netmap_history
BEGIN SELECT RAISE(ABORT,'netmap references are retained'); END;
CREATE TABLE node_netmaps (
    network_id BLOB NOT NULL,
    node_id BLOB NOT NULL,
    serial INTEGER NOT NULL CHECK(serial BETWEEN 1 AND 9007199254740991),
    revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
    digest BLOB NOT NULL CHECK(length(digest)=32),
    dependency_digest BLOB NOT NULL CHECK(length(dependency_digest)=32),
    payload BLOB NOT NULL,
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    withdrawn_at INTEGER CHECK(withdrawn_at>=created_at),
    PRIMARY KEY(network_id,node_id,serial),
    FOREIGN KEY(network_id,node_id,serial,revision,digest) REFERENCES node_netmap_history(network_id,node_id,serial,revision,digest),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id),
    FOREIGN KEY(network_id,revision) REFERENCES configuration_versions(network_id,revision)
) STRICT;
CREATE TRIGGER node_netmaps_monotonic BEFORE INSERT ON node_netmaps
WHEN EXISTS(SELECT 1 FROM node_netmaps WHERE network_id=NEW.network_id AND node_id=NEW.node_id AND serial>=NEW.serial)
BEGIN SELECT RAISE(ABORT,'netmap serial must increase'); END;
CREATE TRIGGER node_netmaps_immutable BEFORE UPDATE OF network_id,node_id,serial,revision,digest,dependency_digest,payload,created_at ON node_netmaps
BEGIN SELECT RAISE(ABORT,'immutable netmap content'); END;
-- Rule-set artifacts each stored netmap serial references. A Node may fetch
-- a digest while some netmap of its own that references it can still be
-- under a lease: the current one, or one superseded less than a lease
-- validity ago. Rows superseded longer than the longest validity are removed.
CREATE TABLE node_netmap_artifacts (
    network_id BLOB NOT NULL,
    node_id BLOB NOT NULL,
    serial INTEGER NOT NULL CHECK(serial BETWEEN 1 AND 9007199254740991),
    digest BLOB NOT NULL CHECK(length(digest)=32),
    superseded_at INTEGER CHECK(superseded_at>=0),
    PRIMARY KEY(network_id,node_id,serial,digest),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
) STRICT;
CREATE INDEX node_netmap_artifacts_digest ON node_netmap_artifacts(network_id,node_id,digest);
-- Node-reported save/installation outcome of one exact netmap, ordered by
-- instance, attempt and sequence. Rows are history: a receipt is current
-- evidence only for the Node's current netmap and live instance.
CREATE TABLE netmap_receipts (
    network_id BLOB NOT NULL,
    node_id BLOB NOT NULL,
    serial INTEGER NOT NULL,
    instance_id BLOB NOT NULL CHECK(length(instance_id)=16),
    generation INTEGER NOT NULL CHECK(generation BETWEEN 1 AND 9007199254740991),
    attempt INTEGER NOT NULL CHECK(attempt BETWEEN 1 AND 9007199254740991),
    sequence INTEGER NOT NULL CHECK(sequence BETWEEN 0 AND 9007199254740991),
    phase TEXT NOT NULL CHECK(phase IN ('saved','applied','failed')),
    reported_at INTEGER NOT NULL,
    received_at INTEGER NOT NULL,
    failure TEXT CHECK(failure IN ('validation','storage','listeners','underlay','tun','dns','dataplane')),
    PRIMARY KEY(network_id,node_id,serial,instance_id,attempt,sequence),
    FOREIGN KEY(network_id,node_id,serial) REFERENCES node_netmap_history(network_id,node_id,serial)
) STRICT;
CREATE TRIGGER netmap_receipts_immutable BEFORE UPDATE ON netmap_receipts
BEGIN SELECT RAISE(ABORT,'immutable netmap receipt'); END;
-- Over-limit Nodes excluded by a non-configuration recompilation: their
-- netmap is unavailable until a later recompilation fits the bound again.
CREATE TABLE node_netmap_unavailable (
    network_id BLOB NOT NULL,
    node_id BLOB NOT NULL,
    reason TEXT NOT NULL CHECK(reason='limit'),
    since INTEGER NOT NULL CHECK(since>=0),
    PRIMARY KEY(network_id,node_id),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
) STRICT;
-- The Server's netmap signing keys. The private key is sealed with the master
-- key and bound to its ID and generation; exactly one key is current.
CREATE TABLE server_signing_keys (
    id BLOB NOT NULL PRIMARY KEY CHECK(length(id)=16 AND id!=zeroblob(16)),
    purpose TEXT NOT NULL CHECK(purpose='netmap'),
    generation INTEGER NOT NULL CHECK(generation BETWEEN 1 AND 9007199254740991),
    public_key BLOB NOT NULL UNIQUE CHECK(length(public_key)=32 AND public_key!=zeroblob(32)),
    private_key BLOB NOT NULL,
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    retired_at INTEGER CHECK(retired_at>=created_at),
    UNIQUE(purpose,generation)
) STRICT;
CREATE UNIQUE INDEX server_signing_keys_current ON server_signing_keys(purpose) WHERE retired_at IS NULL;
CREATE TRIGGER server_signing_keys_immutable BEFORE UPDATE OF id,purpose,generation,public_key,private_key,created_at ON server_signing_keys
BEGIN SELECT RAISE(ABORT,'immutable signing key'); END;
CREATE TRIGGER server_signing_keys_no_delete BEFORE DELETE ON server_signing_keys
BEGIN SELECT RAISE(ABORT,'signing keys are retained'); END;
