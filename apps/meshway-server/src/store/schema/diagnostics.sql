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
    dirty INTEGER NOT NULL DEFAULT 1 CHECK(dirty IN (0,1)),
    FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)
) STRICT;
CREATE INDEX diagnostic_tasks_network ON diagnostic_tasks(network_id);
CREATE TABLE diagnostic_subscriptions (
    id BLOB PRIMARY KEY NOT NULL CHECK(length(id)=16),
    task_id BLOB NOT NULL REFERENCES diagnostic_tasks(id) ON DELETE CASCADE,
    session_id BLOB NOT NULL REFERENCES user_sessions(id),
    membership_id BLOB NOT NULL REFERENCES memberships(id),
    request_digest BLOB NOT NULL CHECK(length(request_digest)=32),
    expires_at INTEGER NOT NULL CHECK(expires_at>0),
    cancelled INTEGER NOT NULL DEFAULT 0 CHECK(cancelled IN (0,1))
) STRICT;
CREATE INDEX diagnostic_subscriptions_task ON diagnostic_subscriptions(task_id);
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
