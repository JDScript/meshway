CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK (id = 1), admin_hash BLOB NOT NULL);
CREATE TABLE nodes (
    id TEXT PRIMARY KEY,
    credential_hash BLOB NOT NULL UNIQUE,
    name TEXT NOT NULL,
    agent_version TEXT NOT NULL,
    registered_at TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    revoked_at TEXT
);
CREATE TABLE enrollment_tokens (
    id TEXT PRIMARY KEY,
    token_hash BLOB NOT NULL UNIQUE,
    expires_at INTEGER NOT NULL,
    used_node_id TEXT REFERENCES nodes(id),
    binding_hash BLOB,
    CHECK ((used_node_id IS NULL) = (binding_hash IS NULL))
);
PRAGMA user_version = 1;
