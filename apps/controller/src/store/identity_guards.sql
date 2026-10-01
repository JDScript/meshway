-- Fixed identity invariants, never scope selection or query rewriting.
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
