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
