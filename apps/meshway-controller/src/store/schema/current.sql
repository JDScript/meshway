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
    UNIQUE (id, user_id)
) STRICT;

CREATE INDEX user_sessions_user_id ON user_sessions(user_id);

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

CREATE TABLE configuration_versions (
    network_id BLOB NOT NULL REFERENCES networks(id) ON DELETE RESTRICT CHECK (length(network_id) = 16 AND network_id != zeroblob(16)),
    revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
    source_jsonc BLOB NOT NULL CHECK (length(source_jsonc) >= 61),
    configuration BLOB NOT NULL CHECK (length(configuration) >= 61),
    bindings BLOB NOT NULL CHECK (length(bindings) >= 61),
    created_by BLOB NOT NULL REFERENCES users(id) ON DELETE RESTRICT CHECK (length(created_by) = 16 AND created_by != zeroblob(16)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    cause TEXT NOT NULL CHECK (cause = 'networkCreated'),
    request_id BLOB NOT NULL CHECK (length(request_id) = 16 AND request_id != zeroblob(16)),
    PRIMARY KEY (network_id, revision)
) STRICT;
CREATE TRIGGER configuration_versions_no_update BEFORE UPDATE ON configuration_versions
BEGIN SELECT RAISE(ABORT, 'immutable configuration version'); END;
CREATE TRIGGER configuration_versions_no_delete BEFORE DELETE ON configuration_versions
BEGIN SELECT RAISE(ABORT, 'immutable configuration version'); END;

-- Successful account/network mutations and their activities commit together.
CREATE TABLE activity_events (
    sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT CHECK (sequence BETWEEN 1 AND 9007199254740991),
    id BLOB NOT NULL UNIQUE CHECK (length(id) = 16 AND id != zeroblob(16)),
    scope TEXT NOT NULL CHECK (scope IN ('account', 'network')),
    account_user_id BLOB REFERENCES users(id) ON DELETE RESTRICT CHECK (length(account_user_id) = 16 AND account_user_id != zeroblob(16)),
    network_id BLOB REFERENCES networks(id) ON DELETE RESTRICT CHECK (length(network_id) = 16 AND network_id != zeroblob(16)),
    at INTEGER NOT NULL CHECK (at >= 0),
    actor_user_id BLOB NOT NULL REFERENCES users(id) ON DELETE RESTRICT CHECK (length(actor_user_id) = 16 AND actor_user_id != zeroblob(16)),
    action TEXT NOT NULL CHECK (action IN ('account.register', 'session.login', 'session.logout', 'network.create', 'network.rename')),
    target_user_id BLOB REFERENCES users(id) ON DELETE RESTRICT,
    target_session_id BLOB CHECK (length(target_session_id) = 16 AND target_session_id != zeroblob(16)),
    request_id BLOB NOT NULL CHECK (length(request_id) = 16 AND request_id != zeroblob(16)),
    CHECK ((scope = 'account' AND account_user_id IS NOT NULL AND network_id IS NULL AND actor_user_id = account_user_id AND
        ((action = 'account.register' AND target_user_id IS NOT NULL AND target_user_id = account_user_id AND target_session_id IS NULL)
        OR (action IN ('session.login', 'session.logout') AND target_user_id IS NULL AND target_session_id IS NOT NULL)))
        OR (scope = 'network' AND network_id IS NOT NULL AND account_user_id IS NULL AND action IN ('network.create', 'network.rename') AND target_user_id IS NULL AND target_session_id IS NULL)),
    FOREIGN KEY (target_session_id, account_user_id) REFERENCES user_sessions(id, user_id) ON DELETE RESTRICT
) STRICT;

CREATE INDEX activity_events_account_sequence ON activity_events(account_user_id, sequence);
CREATE INDEX activity_events_network_sequence ON activity_events(network_id, sequence);
