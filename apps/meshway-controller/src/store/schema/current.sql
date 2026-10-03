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
    revoked_at INTEGER CHECK (revoked_at >= created_at)
) STRICT;

CREATE INDEX user_sessions_user_id ON user_sessions(user_id);
