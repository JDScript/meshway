-- Explicit migration fragment. Caller owns one migration transaction and FK mode.
-- No user_version, PRAGMA or automatic read-time schema installation.
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
-- Once copied to a future versioned migration these guards protect immutable
-- invitation binding and global identity even against accidental UPDATE SQL.
CREATE TRIGGER account_invitation_binding BEFORE UPDATE ON account_invitations
WHEN NEW.invitation_id IS NOT OLD.invitation_id
 OR NEW.network_id IS NOT OLD.network_id OR NEW.email IS NOT OLD.email
 OR NEW.role IS NOT OLD.role OR NEW.token_hash IS NOT OLD.token_hash
 OR NEW.issuer_user_id IS NOT OLD.issuer_user_id OR NEW.expires_at IS NOT OLD.expires_at
 OR (OLD.consumed_by IS NOT NULL AND
     (NEW.consumed_by IS NOT OLD.consumed_by OR NEW.consumed_at IS NOT OLD.consumed_at))
BEGIN SELECT RAISE(ABORT,'immutable account invitation binding'); END;
