-- A diagnostic lifetime is distinct from a Node-owned business demand.
CREATE TABLE diagnostic_tasks (
    id BLOB PRIMARY KEY NOT NULL CHECK(length(id)=16),
    network_id BLOB NOT NULL REFERENCES networks(id),
    node_id BLOB NOT NULL,
    instance_id BLOB NOT NULL CHECK(length(instance_id)=16),
    boot_id BLOB NOT NULL CHECK(length(boot_id)=16),
    revision INTEGER NOT NULL CHECK(revision>0),
    reuse_key BLOB NOT NULL CHECK(length(reuse_key)=32),
    payload BLOB NOT NULL,
    compilation BLOB,
    dirty INTEGER NOT NULL DEFAULT 1 CHECK(dirty IN (0,1)),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
) STRICT;
CREATE INDEX diagnostic_tasks_network ON diagnostic_tasks(network_id);
CREATE TABLE diagnostic_subscriptions (
    id BLOB PRIMARY KEY NOT NULL CHECK(length(id)=16),
    task_id BLOB NOT NULL REFERENCES diagnostic_tasks(id) ON DELETE CASCADE,
    principal_kind TEXT NOT NULL DEFAULT 'user_session' CHECK(principal_kind IN ('user_session','node_instance')),
    session_id BLOB REFERENCES user_sessions(id),
    membership_id BLOB REFERENCES memberships(id),
    node_network_id BLOB,
    node_id BLOB,
    control_key_id BLOB,
    instance_id BLOB,
    instance_generation INTEGER,
    boot_id BLOB,
    request_payload BLOB,
    request_digest BLOB NOT NULL CHECK(length(request_digest)=32),
    expires_at INTEGER NOT NULL CHECK(expires_at>0),
    cancelled INTEGER NOT NULL DEFAULT 0 CHECK(cancelled IN (0,1)),
    CHECK((principal_kind='user_session' AND session_id IS NOT NULL AND membership_id IS NOT NULL
      AND node_network_id IS NULL AND node_id IS NULL AND control_key_id IS NULL AND instance_id IS NULL AND instance_generation IS NULL AND boot_id IS NULL AND request_payload IS NULL)
      OR (principal_kind='node_instance' AND session_id IS NULL AND membership_id IS NULL
      AND node_network_id IS NOT NULL AND node_id IS NOT NULL AND control_key_id IS NOT NULL
      AND instance_id IS NOT NULL AND length(instance_id)=16 AND instance_generation IS NOT NULL AND instance_generation BETWEEN 1 AND 9007199254740991
      AND boot_id IS NOT NULL AND length(boot_id)=16 AND request_payload IS NOT NULL)),
    FOREIGN KEY(node_network_id,node_id,control_key_id) REFERENCES node_keys(network_id,node_id,id),
    FOREIGN KEY(node_network_id,node_id,instance_id) REFERENCES node_instance_claims(network_id,node_id,instance_id)
) STRICT;
CREATE INDEX diagnostic_subscriptions_task ON diagnostic_subscriptions(task_id);
CREATE TRIGGER diagnostic_node_binding BEFORE INSERT ON diagnostic_subscriptions
WHEN NEW.principal_kind='node_instance' AND NOT EXISTS(
 SELECT 1 FROM diagnostic_tasks t JOIN node_execution e ON e.network_id=t.network_id AND e.node_id=t.node_id
 JOIN node_keys k ON k.network_id=t.network_id AND k.node_id=t.node_id AND k.id=NEW.control_key_id
 WHERE t.id=NEW.task_id AND t.network_id=NEW.node_network_id AND t.node_id=NEW.node_id
 AND t.instance_id=NEW.instance_id AND t.boot_id=NEW.boot_id
 AND e.instance_id=NEW.instance_id AND e.generation=NEW.instance_generation AND e.boot_id=NEW.boot_id
 AND k.purpose='control' AND k.retired_at IS NULL AND k.revoked_at IS NULL)
BEGIN SELECT RAISE(ABORT,'invalid diagnostic node binding'); END;
CREATE TRIGGER diagnostic_principal_immutable BEFORE UPDATE OF principal_kind,session_id,membership_id,node_network_id,node_id,control_key_id,instance_id,instance_generation,boot_id,task_id,request_digest,request_payload ON diagnostic_subscriptions
BEGIN SELECT RAISE(ABORT,'immutable diagnostic principal'); END;
CREATE TRIGGER diagnostic_subscription_changed AFTER UPDATE ON diagnostic_subscriptions
BEGIN UPDATE diagnostic_tasks SET dirty=1 WHERE id=NEW.task_id; END;
-- Revocation removes persistent diagnostic eligibility in the upstream account
-- transaction. The execution publisher promptly withdraws the resulting slices;
-- already installed measurement permits additionally have a hard short deadline.
CREATE TRIGGER diagnostic_session_revoked AFTER UPDATE OF revoked_at ON user_sessions
WHEN NEW.revoked_at IS NOT NULL
BEGIN UPDATE diagnostic_subscriptions SET cancelled=1 WHERE session_id=NEW.id AND cancelled=0; END;
CREATE TRIGGER diagnostic_user_revoked AFTER UPDATE OF credential_version,disabled_at ON users
BEGIN UPDATE diagnostic_subscriptions SET cancelled=1 WHERE session_id IN
 (SELECT id FROM user_sessions WHERE user_id=NEW.id AND (credential_version!=NEW.credential_version OR NEW.disabled_at IS NOT NULL)) AND cancelled=0; END;
CREATE TRIGGER diagnostic_membership_changed AFTER UPDATE OF role,revoked_at ON memberships
BEGIN UPDATE diagnostic_subscriptions SET cancelled=1 WHERE membership_id=NEW.id AND cancelled=0; END;
CREATE TRIGGER diagnostic_configuration_changed AFTER UPDATE OF current_revision ON networks
WHEN NEW.current_revision!=OLD.current_revision
BEGIN UPDATE diagnostic_subscriptions SET cancelled=1 WHERE task_id IN
 (SELECT id FROM diagnostic_tasks WHERE network_id=NEW.id) AND cancelled=0; END;
CREATE TRIGGER diagnostic_node_key_changed AFTER UPDATE OF retired_at,revoked_at ON node_keys
WHEN NEW.retired_at IS NOT NULL OR NEW.revoked_at IS NOT NULL
BEGIN UPDATE diagnostic_subscriptions SET cancelled=1 WHERE control_key_id=NEW.id AND cancelled=0; END;
CREATE TRIGGER diagnostic_node_revoked AFTER UPDATE OF credentials_revoked_at ON nodes
WHEN NEW.credentials_revoked_at IS NOT NULL
BEGIN UPDATE diagnostic_subscriptions SET cancelled=1 WHERE task_id IN
 (SELECT id FROM diagnostic_tasks WHERE node_id=NEW.id AND network_id=NEW.network_id) AND cancelled=0; END;
CREATE TRIGGER diagnostic_instance_changed AFTER UPDATE OF instance_id,generation,boot_id ON node_execution
WHEN NEW.instance_id IS NOT OLD.instance_id OR NEW.generation!=OLD.generation OR NEW.boot_id IS NOT OLD.boot_id
BEGIN UPDATE diagnostic_subscriptions SET cancelled=1 WHERE task_id IN
 (SELECT id FROM diagnostic_tasks WHERE node_id=NEW.node_id AND network_id=NEW.network_id) AND cancelled=0; END;
