CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK (id = 1), admin_hash BLOB NOT NULL);
CREATE TABLE networks (
    network_id TEXT PRIMARY KEY CHECK(length(network_id)=32 AND network_id NOT GLOB '*[^0-9a-f]*'),
    created_at TEXT NOT NULL,
    initial INTEGER NOT NULL CHECK(initial IN (0,1))
);
CREATE UNIQUE INDEX initial_network ON networks(initial) WHERE initial=1;
CREATE TABLE nodes (
    id TEXT PRIMARY KEY,
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    credential_hash BLOB NOT NULL UNIQUE,
    name TEXT NOT NULL, agent_version TEXT NOT NULL,
    registered_at TEXT NOT NULL, last_seen TEXT NOT NULL, revoked_at TEXT,
    mesh_ipv4 TEXT NOT NULL, mesh_ipv6 TEXT NOT NULL, country TEXT,
    acknowledged_revision INTEGER NOT NULL DEFAULT 0,
    acknowledged_digest TEXT,
    UNIQUE(network_id,id),
    UNIQUE(network_id,mesh_ipv4), UNIQUE(network_id,mesh_ipv6)
);
CREATE TABLE enrollment_tokens (
    id TEXT PRIMARY KEY,
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    token_hash BLOB NOT NULL UNIQUE, expires_at INTEGER NOT NULL,
    used_node_id TEXT, binding_hash BLOB, device_binding TEXT,
    CHECK((used_node_id IS NULL)=(binding_hash IS NULL)),
    FOREIGN KEY(network_id,used_node_id) REFERENCES nodes(network_id,id)
);
CREATE TABLE configuration (
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    id INTEGER NOT NULL CHECK(id=1),
    revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 4294967295),
    ipv4_prefix TEXT NOT NULL, ipv6_prefix TEXT NOT NULL,
    PRIMARY KEY(network_id,id)
);
CREATE TABLE network_identity (
    id INTEGER NOT NULL CHECK(id=1),
    network_id TEXT PRIMARY KEY REFERENCES networks(network_id) CHECK(length(network_id)=32),
    name TEXT NOT NULL, dns TEXT NOT NULL
);
CREATE TABLE connection_identities (
    node_id TEXT PRIMARY KEY,
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    public_key TEXT NOT NULL UNIQUE, capabilities TEXT NOT NULL,
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
);
CREATE TABLE connection_source (
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    id INTEGER NOT NULL CHECK(id=1), configuration TEXT NOT NULL,
    PRIMARY KEY(network_id,id)
);
CREATE TABLE connection_receipts (
    node_id TEXT PRIMARY KEY,
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 4294967295),
    digest TEXT NOT NULL, saved_at TEXT NOT NULL,
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
);
CREATE TABLE connection_runtime_leases (
    node_id TEXT PRIMARY KEY,
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    instance_id TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch BETWEEN 1 AND 4294967295),
    UNIQUE(network_id,node_id),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
);
CREATE TABLE connection_runtime_reports (
    node_id TEXT PRIMARY KEY,
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    sequence INTEGER NOT NULL CHECK(sequence BETWEEN 1 AND 4294967295),
    body BLOB NOT NULL, reported_at TEXT NOT NULL,
    FOREIGN KEY(network_id,node_id) REFERENCES connection_runtime_leases(network_id,node_id)
);
CREATE TABLE tag_definitions (
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    tag TEXT NOT NULL, description TEXT NOT NULL,
    PRIMARY KEY(network_id,tag)
);
CREATE TABLE node_tags (
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    node_id TEXT NOT NULL, tag TEXT NOT NULL,
    PRIMARY KEY(network_id,node_id,tag),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id),
    FOREIGN KEY(network_id,tag) REFERENCES tag_definitions(network_id,tag)
);

CREATE TABLE managed_listeners (
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    listener_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    settings TEXT NOT NULL,
    material_version INTEGER NOT NULL CHECK(material_version BETWEEN 1 AND 4294967295),
    certificate_sha256 TEXT,
    material_nonce BLOB,
    material_ciphertext BLOB,
    PRIMARY KEY(network_id, listener_id),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id),
    CHECK ((material_nonce IS NULL) = (material_ciphertext IS NULL))
);
CREATE UNIQUE INDEX enrollment_tokens_network_id ON enrollment_tokens(network_id,id);
CREATE TABLE managed_transport_capabilities (
    network_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    vless_streams TEXT NOT NULL,
    PRIMARY KEY(network_id,node_id),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
);
CREATE TABLE managed_enrollment_intents (
    network_id TEXT NOT NULL,
    token_id TEXT NOT NULL,
    settings TEXT NOT NULL,
    PRIMARY KEY(network_id,token_id),
    FOREIGN KEY(network_id,token_id) REFERENCES enrollment_tokens(network_id,id)
);
CREATE TABLE account_users (
    user_id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    email_verified INTEGER NOT NULL DEFAULT 0 CHECK(email_verified IN (0,1)),
    password_phc TEXT NOT NULL CHECK(password_phc LIKE '$argon2id$%'),
    credential_version INTEGER NOT NULL DEFAULT 1 CHECK(credential_version>0),
    created_at INTEGER NOT NULL
);
CREATE TABLE account_memberships (
    membership_id TEXT PRIMARY KEY,
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    user_id TEXT NOT NULL REFERENCES account_users(user_id),
    role TEXT NOT NULL CHECK(role IN ('owner','admin','member')),
    revoked_at INTEGER,
    UNIQUE(network_id,membership_id)
);
CREATE UNIQUE INDEX account_active_membership ON account_memberships(network_id,user_id)
WHERE revoked_at IS NULL;
CREATE TABLE account_invitations (
    invitation_id TEXT PRIMARY KEY,
    token_hash BLOB NOT NULL UNIQUE CHECK(length(token_hash)=32),
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    email TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('owner','admin','member')),
    issuer_user_id TEXT REFERENCES account_users(user_id),
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER,
    consumed_by TEXT REFERENCES account_users(user_id),
    consumed_at INTEGER,
    CHECK((consumed_by IS NULL)=(consumed_at IS NULL))
);
CREATE TABLE account_sessions (
    token_hash BLOB PRIMARY KEY CHECK(length(token_hash)=32),
    user_id TEXT NOT NULL REFERENCES account_users(user_id),
    credential_version INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER
);
CREATE INDEX account_sessions_user ON account_sessions(user_id);
CREATE TABLE account_password_resets (
    reset_id TEXT PRIMARY KEY,
    token_hash BLOB NOT NULL UNIQUE CHECK(length(token_hash)=32),
    user_id TEXT NOT NULL REFERENCES account_users(user_id),
    credential_version INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER,
    consumed_at INTEGER
);
CREATE INDEX account_resets_user ON account_password_resets(user_id);
CREATE TRIGGER account_reset_binding BEFORE UPDATE ON account_password_resets
WHEN NEW.reset_id IS NOT OLD.reset_id OR NEW.token_hash IS NOT OLD.token_hash
 OR NEW.user_id IS NOT OLD.user_id OR NEW.credential_version IS NOT OLD.credential_version
 OR NEW.expires_at IS NOT OLD.expires_at
 OR (OLD.consumed_at IS NOT NULL AND NEW.consumed_at IS NOT OLD.consumed_at)
BEGIN SELECT RAISE(ABORT,'immutable password reset binding'); END;
CREATE TRIGGER account_user_identity BEFORE UPDATE OF user_id ON account_users
WHEN NEW.user_id IS NOT OLD.user_id
BEGIN SELECT RAISE(ABORT,'immutable global user identity'); END;
CREATE TRIGGER account_membership_identity BEFORE UPDATE ON account_memberships
WHEN NEW.membership_id IS NOT OLD.membership_id
 OR NEW.network_id IS NOT OLD.network_id OR NEW.user_id IS NOT OLD.user_id
 OR (OLD.revoked_at IS NOT NULL AND NEW.revoked_at IS NOT OLD.revoked_at)
BEGIN SELECT RAISE(ABORT,'immutable network membership identity'); END;
CREATE TRIGGER account_invitation_binding BEFORE UPDATE ON account_invitations
WHEN NEW.invitation_id IS NOT OLD.invitation_id
 OR NEW.network_id IS NOT OLD.network_id OR NEW.email IS NOT OLD.email
 OR NEW.role IS NOT OLD.role OR NEW.token_hash IS NOT OLD.token_hash
 OR NEW.issuer_user_id IS NOT OLD.issuer_user_id OR NEW.expires_at IS NOT OLD.expires_at
 OR (OLD.consumed_by IS NOT NULL AND
     (NEW.consumed_by IS NOT OLD.consumed_by OR NEW.consumed_at IS NOT OLD.consumed_at))
BEGIN SELECT RAISE(ABORT,'immutable account invitation binding'); END;

CREATE TABLE node_ownership (
 network_id TEXT NOT NULL, node_id TEXT NOT NULL,
 membership_id TEXT,
 PRIMARY KEY(network_id,node_id),
 FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id),
 FOREIGN KEY(network_id,membership_id) REFERENCES account_memberships(network_id,membership_id)
);
CREATE TRIGGER ownership_new_node AFTER INSERT ON nodes
BEGIN INSERT INTO node_ownership(network_id,node_id) VALUES(NEW.network_id,NEW.id); END;
CREATE TRIGGER ownership_immutable BEFORE UPDATE ON node_ownership
WHEN NEW.network_id IS NOT OLD.network_id OR NEW.node_id IS NOT OLD.node_id
 OR (OLD.membership_id IS NOT NULL AND NEW.membership_id IS NOT OLD.membership_id)
BEGIN SELECT RAISE(ABORT,'immutable device owner'); END;
CREATE TRIGGER ownership_no_delete BEFORE DELETE ON node_ownership
BEGIN SELECT RAISE(ABORT,'persistent device ownership'); END;
CREATE UNIQUE INDEX account_enrollment_scope ON enrollment_tokens(network_id,id);
CREATE UNIQUE INDEX account_membership_subject ON account_memberships(network_id,membership_id,user_id);
CREATE TABLE device_account_bindings (
 token_id TEXT PRIMARY KEY, network_id TEXT NOT NULL, user_id TEXT NOT NULL,
 membership_id TEXT NOT NULL, session_digest BLOB NOT NULL CHECK(length(session_digest)=32),
 credential_version INTEGER NOT NULL CHECK(credential_version>0),
 FOREIGN KEY(network_id,token_id) REFERENCES enrollment_tokens(network_id,id),
 FOREIGN KEY(network_id,membership_id,user_id) REFERENCES account_memberships(network_id,membership_id,user_id),
 FOREIGN KEY(user_id) REFERENCES account_users(user_id)
);
CREATE TRIGGER device_account_binding_immutable BEFORE UPDATE ON device_account_bindings
BEGIN SELECT RAISE(ABORT,'immutable device account binding'); END;
CREATE TRIGGER device_account_binding_no_delete BEFORE DELETE ON device_account_bindings
BEGIN SELECT RAISE(ABORT,'persistent device account binding'); END;

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
CREATE INDEX IF NOT EXISTS latency_source_oldest
    ON latency_samples(network_id, source_node_id, measured_ms, row_id);
CREATE TABLE IF NOT EXISTS latency_admissions (
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    source_node_id TEXT NOT NULL,
    measurement_id TEXT NOT NULL,
    body_digest BLOB NOT NULL,
    received_ms INTEGER NOT NULL,
    expires_ms INTEGER NOT NULL,
    PRIMARY KEY(network_id, source_node_id, measurement_id),
    FOREIGN KEY(network_id,source_node_id) REFERENCES nodes(network_id,id)
);
CREATE INDEX IF NOT EXISTS latency_admission_receive
    ON latency_admissions(network_id, source_node_id, received_ms);
CREATE INDEX IF NOT EXISTS latency_admission_expiry
    ON latency_admissions(network_id, source_node_id, expires_ms);
CREATE TABLE IF NOT EXISTS latency_coverage (
    network_id TEXT NOT NULL REFERENCES networks(network_id),
    source_node_id TEXT NOT NULL,
    capacity_before_ms INTEGER NOT NULL,
    PRIMARY KEY(network_id, source_node_id),
    FOREIGN KEY(network_id,source_node_id) REFERENCES nodes(network_id,id)
);

CREATE TABLE network_policy (network_id TEXT PRIMARY KEY REFERENCES networks(network_id), policy TEXT NOT NULL, source_jsonc TEXT);
CREATE TABLE mesh_execution_capabilities (
    node_id TEXT PRIMARY KEY,
    network_id TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch BETWEEN 1 AND 4294967295),
    instance_id TEXT NOT NULL,
    capability INTEGER NOT NULL CHECK(capability = 3),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
);
CREATE TABLE mesh_execution_receipts (
    node_id TEXT PRIMARY KEY,
    network_id TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch BETWEEN 1 AND 4294967295),
    revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 4294967295),
    digest TEXT NOT NULL,
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
);

CREATE TRIGGER IF NOT EXISTS nodes_no_rebind
BEFORE UPDATE OF id,network_id,credential_hash ON nodes
WHEN NEW.id IS NOT OLD.id OR NEW.network_id IS NOT OLD.network_id
 OR NEW.credential_hash IS NOT OLD.credential_hash
BEGIN SELECT RAISE(ABORT,'node identity is immutable'); END;

CREATE TRIGGER IF NOT EXISTS connection_identity_no_rebind
BEFORE UPDATE OF node_id,network_id,public_key ON connection_identities
WHEN NEW.node_id IS NOT OLD.node_id OR NEW.network_id IS NOT OLD.network_id
 OR NEW.public_key IS NOT OLD.public_key
BEGIN SELECT RAISE(ABORT,'connection identity is immutable'); END;

CREATE TRIGGER IF NOT EXISTS enrollment_identity_no_rebind
BEFORE UPDATE OF id,network_id,token_hash,device_binding,used_node_id,binding_hash ON enrollment_tokens
WHEN NEW.id IS NOT OLD.id OR NEW.network_id IS NOT OLD.network_id
 OR NEW.token_hash IS NOT OLD.token_hash OR NEW.device_binding IS NOT OLD.device_binding
 OR (OLD.used_node_id IS NOT NULL AND
     (NEW.used_node_id IS NOT OLD.used_node_id OR NEW.binding_hash IS NOT OLD.binding_hash))
BEGIN SELECT RAISE(ABORT,'enrollment identity is immutable'); END;

PRAGMA user_version = 15;
