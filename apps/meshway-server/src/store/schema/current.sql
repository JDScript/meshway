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
    action TEXT NOT NULL CHECK (action IN ('account.register', 'session.login', 'session.logout', 'session.revoke', 'session.revokeOthers', 'session.revokeAll', 'session.delegate', 'account.password', 'account.passwordReset', 'account.resetIssued', 'account.email', 'network.create', 'network.rename', 'configuration.apply','membership.role','membership.join','membership.remove','joinLink.create','joinLink.revoke','routing.select')),
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
    CHECK ((action = 'routing.select') = (target_node_id IS NOT NULL AND target_profile IS NOT NULL)),
    CHECK ((scope = 'account' AND account_user_id IS NOT NULL AND network_id IS NULL AND (actor_user_id = account_user_id OR (actor_kind='system' AND action='account.resetIssued')) AND
        ((action IN ('account.register', 'session.revokeOthers', 'session.revokeAll', 'account.password', 'account.passwordReset', 'account.resetIssued', 'account.email') AND target_user_id IS NOT NULL AND target_user_id = account_user_id AND target_session_id IS NULL)
        OR (action IN ('session.login', 'session.logout', 'session.revoke', 'session.delegate') AND target_user_id IS NULL AND target_session_id IS NOT NULL)))
        OR (scope = 'network' AND network_id IS NOT NULL AND account_user_id IS NULL AND action IN ('network.create', 'network.rename','configuration.apply','membership.role','membership.join','membership.remove','joinLink.create','joinLink.revoke','routing.select') AND target_user_id IS NULL AND target_session_id IS NULL)),
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
