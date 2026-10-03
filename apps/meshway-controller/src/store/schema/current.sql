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

-- The account branch of ActivityEvent. Other scopes/actions are added by their
-- owning use cases; successful account mutations and activities commit together.
CREATE TABLE activity_events (
    sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT CHECK (sequence BETWEEN 1 AND 9007199254740991),
    id BLOB NOT NULL UNIQUE CHECK (length(id) = 16 AND id != zeroblob(16)),
    scope TEXT NOT NULL CHECK (scope = 'account'),
    account_user_id BLOB NOT NULL REFERENCES users(id) ON DELETE RESTRICT CHECK (length(account_user_id) = 16 AND account_user_id != zeroblob(16)),
    at INTEGER NOT NULL CHECK (at >= 0),
    actor_user_id BLOB NOT NULL REFERENCES users(id) ON DELETE RESTRICT CHECK (actor_user_id = account_user_id),
    action TEXT NOT NULL CHECK (action IN ('account.register', 'session.login', 'session.logout')),
    target_user_id BLOB REFERENCES users(id) ON DELETE RESTRICT,
    target_session_id BLOB CHECK (length(target_session_id) = 16 AND target_session_id != zeroblob(16)),
    request_id BLOB NOT NULL CHECK (length(request_id) = 16 AND request_id != zeroblob(16)),
    CHECK ((action = 'account.register' AND target_user_id IS NOT NULL AND target_user_id = account_user_id AND target_session_id IS NULL)
        OR (action IN ('session.login', 'session.logout') AND target_user_id IS NULL AND target_session_id IS NOT NULL)),
    FOREIGN KEY (target_session_id, account_user_id) REFERENCES user_sessions(id, user_id) ON DELETE RESTRICT
) STRICT;

CREATE INDEX activity_events_account_sequence ON activity_events(account_user_id, sequence);
