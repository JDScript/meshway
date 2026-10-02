-- Additive migration: never rewrites any v9 node, tag, address, key or source text.
CREATE TABLE node_ownership (
 network_id TEXT NOT NULL, node_id TEXT NOT NULL,
 membership_id TEXT,
 PRIMARY KEY(network_id,node_id),
 FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id),
 FOREIGN KEY(network_id,membership_id) REFERENCES account_memberships(network_id,membership_id)
);
INSERT INTO node_ownership(network_id,node_id) SELECT network_id,id FROM nodes;
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
