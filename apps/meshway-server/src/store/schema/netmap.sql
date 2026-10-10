-- Per-Node netmap content. A Node keeps its current serial and the one before
-- it (the delta base). Content rows are immutable; a withdrawn head keeps the
-- serial high-water so a Node that returns never sees its serial regress.
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
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id),
    FOREIGN KEY(network_id,revision) REFERENCES configuration_versions(network_id,revision)
) STRICT;
CREATE TRIGGER node_netmaps_monotonic BEFORE INSERT ON node_netmaps
WHEN EXISTS(SELECT 1 FROM node_netmaps WHERE network_id=NEW.network_id AND node_id=NEW.node_id AND serial>=NEW.serial)
BEGIN SELECT RAISE(ABORT,'netmap serial must increase'); END;
CREATE TRIGGER node_netmaps_immutable BEFORE UPDATE OF network_id,node_id,serial,revision,digest,dependency_digest,payload,created_at ON node_netmaps
BEGIN SELECT RAISE(ABORT,'immutable netmap content'); END;
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
    FOREIGN KEY(network_id,node_id,serial) REFERENCES node_netmaps(network_id,node_id,serial)
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
