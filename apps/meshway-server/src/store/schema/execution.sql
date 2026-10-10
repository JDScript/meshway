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
