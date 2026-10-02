-- Additive v9 -> v10: no existing row, revision or configuration bytes change.
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
PRAGMA user_version = 10;
