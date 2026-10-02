CREATE TABLE settings(id INTEGER PRIMARY KEY CHECK(id=1),admin_hash BLOB NOT NULL);
CREATE TABLE nodes(id TEXT PRIMARY KEY,credential_hash BLOB NOT NULL UNIQUE,name TEXT NOT NULL,
 agent_version TEXT NOT NULL,registered_at TEXT NOT NULL,last_seen TEXT NOT NULL,revoked_at TEXT,
 mesh_ipv4 TEXT,mesh_ipv6 TEXT,country TEXT,acknowledged_revision INTEGER NOT NULL DEFAULT 0,acknowledged_digest TEXT);
CREATE UNIQUE INDEX node_ipv4 ON nodes(mesh_ipv4);
CREATE UNIQUE INDEX node_ipv6 ON nodes(mesh_ipv6);
CREATE TABLE enrollment_tokens(id TEXT PRIMARY KEY,token_hash BLOB NOT NULL UNIQUE,expires_at INTEGER NOT NULL,
 used_node_id TEXT REFERENCES nodes(id),binding_hash BLOB,CHECK((used_node_id IS NULL)=(binding_hash IS NULL)));
CREATE TABLE configuration(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 4294967295),
 ipv4_prefix TEXT NOT NULL,ipv6_prefix TEXT NOT NULL,policy TEXT NOT NULL);
CREATE TABLE connection_identities(node_id TEXT PRIMARY KEY REFERENCES nodes(id),public_key TEXT NOT NULL UNIQUE,capabilities TEXT NOT NULL);
CREATE TABLE connection_source(id INTEGER PRIMARY KEY CHECK(id=1),configuration TEXT NOT NULL);
CREATE TABLE connection_receipts(node_id TEXT PRIMARY KEY REFERENCES nodes(id),revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 4294967295),digest TEXT NOT NULL,saved_at TEXT NOT NULL);
PRAGMA user_version=3;
