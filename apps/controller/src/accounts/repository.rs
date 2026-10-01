use rusqlite::{Connection, OptionalExtension, Transaction, params};
use zeroize::Zeroizing;

use super::*;

pub(crate) const SCHEMA_FRAGMENT: &str = include_str!("schema.sql");
const INVITE_MAX: i64 = 7 * 86400;
const RESET_MAX: i64 = 3600;
const SESSION_MAX: i64 = 7 * 86400;

fn expiry(now: i64, ttl: i64, maximum: i64) -> Result<i64> {
    if now < 0 || !(1..=maximum).contains(&ttl) {
        return Err(AccountError::Invalid);
    }
    now.checked_add(ttl).ok_or(AccountError::Invalid)
}

/// Mutations create a savepoint within the caller's IMMEDIATE transaction.
/// Failure rolls back the complete domain operation even if the caller commits
/// other work. The caller owns the outer transaction/connection and clock.
fn atomic<T>(tx: &mut Transaction<'_>, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let savepoint = tx.savepoint()?;
    let output = f(&savepoint)?;
    savepoint.commit()?;
    Ok(output)
}

/// Reads have no DDL side effects. The caller must enable foreign keys once and
/// install SCHEMA_FRAGMENT in its explicit, versioned migration transaction.
pub(crate) struct Accounts<'a> {
    db: &'a Connection,
}

struct Invitation {
    network: AccountNetwork,
    email: Email,
    role: Role,
    issuer: Option<UserId>,
}

/// Safe-to-carry pre-KDF intent, revalidated during final consumption.
pub(crate) struct RegistrationIntent {
    digest: Vec<u8>,
    email: Email,
}

pub(crate) struct ResetIntent {
    digest: Vec<u8>,
    user: UserId,
    version: i64,
}

impl<'a> Accounts<'a> {
    pub(crate) fn new(db: &'a Connection) -> Self {
        Self { db }
    }

    pub(crate) fn network(&self, id: &str) -> Result<AccountNetwork> {
        let id = self
            .db
            .query_row(
                "SELECT network_id FROM networks WHERE network_id=?1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .ok_or(AccountError::Forbidden)?;
        Ok(AccountNetwork(id))
    }

    pub(crate) fn user(&self, id: &UserId) -> Result<User> {
        let (email, verified) = self.db.query_row(
            "SELECT email,email_verified FROM account_users WHERE user_id=?1",
            [&id.0],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
        )?;
        Ok(User {
            id: id.clone(),
            email: Email::parse(&email).map_err(|_| AccountError::Storage)?,
            email_verified: verified,
        })
    }

    pub(crate) fn credential(&self, email: &Email) -> Result<Option<CredentialSnapshot>> {
        Ok(self
            .db
            .query_row(
                "SELECT user_id,credential_version,password_phc FROM account_users WHERE email=?1",
                [email.as_str()],
                |r| {
                    Ok(CredentialSnapshot {
                        user: UserId(r.get(0)?),
                        version: r.get(1)?,
                        phc: Zeroizing::new(r.get(2)?),
                    })
                },
            )
            .optional()?)
    }

    pub(super) fn role(&self, network: &AccountNetwork, user: &UserId) -> Result<Option<Role>> {
        self.db
            .query_row(
                "SELECT role FROM account_memberships WHERE network_id=?1 AND user_id=?2 AND revoked_at IS NULL",
                params![network.0, user.0],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(Role::stored)
            .transpose()
    }

    pub(super) fn membership(
        &self,
        network: &AccountNetwork,
        user: &UserId,
    ) -> Result<Option<Membership>> {
        self.db.query_row(
            "SELECT membership_id,role FROM account_memberships WHERE network_id=?1 AND user_id=?2 AND revoked_at IS NULL",
            params![network.0,user.0],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        ).optional()?.map(|(id,role)| Ok(Membership {
            id: MembershipId(id), network: network.clone(), user: user.clone(), role: Role::stored(role)?,
        })).transpose()
    }

    pub(super) fn session_user(&self, principal: &Principal, now: i64) -> Result<UserId> {
        let PrincipalKind::Session {
            digest,
            user,
            version,
        } = &principal.0
        else {
            return Err(AccountError::Forbidden);
        };
        let valid: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM account_sessions s JOIN account_users u ON u.user_id=s.user_id WHERE s.token_hash=?1 AND s.user_id=?2 AND s.credential_version=?3 AND u.credential_version=s.credential_version AND s.revoked_at IS NULL AND s.expires_at>?4)", params![digest,user.0,version,now], |r| r.get(0))?;
        if !valid {
            return Err(AccountError::Unauthorized);
        }
        Ok(user.clone())
    }

    pub(crate) fn authenticate(&self, token: &str, now: i64) -> Result<Principal> {
        let digest = Secret::digest(token, SecretKind::Session)?;
        let (user, version) = self.db.query_row("SELECT s.user_id,s.credential_version FROM account_sessions s JOIN account_users u ON u.user_id=s.user_id WHERE s.token_hash=?1 AND s.revoked_at IS NULL AND s.expires_at>?2 AND s.credential_version=u.credential_version", params![digest,now], |r| Ok((UserId(r.get(0)?), r.get::<_, i64>(1)?))).optional()?.ok_or(AccountError::Unauthorized)?;
        Ok(Principal(PrincipalKind::Session {
            digest,
            user,
            version,
        }))
    }

    /// Owner grants any network role; Admin grants Member only. No instance
    /// privilege is acquired by owning any number of networks.
    fn may_grant(
        &self,
        principal: &Principal,
        network: &AccountNetwork,
        role: Role,
        now: i64,
    ) -> Result<Option<UserId>> {
        if matches!(principal.0, PrincipalKind::Bootstrap) {
            return Ok(None);
        }
        let user = self.session_user(principal, now)?;
        match self.role(network, &user)? {
            Some(Role::Owner) => Ok(Some(user)),
            Some(Role::Admin) if role == Role::Member => Ok(Some(user)),
            _ => Err(AccountError::Forbidden),
        }
    }

    pub(crate) fn memberships(&self, principal: &Principal, now: i64) -> Result<Vec<Membership>> {
        let user = self.session_user(principal, now)?;
        let mut statement = self.db.prepare(
            "SELECT network_id,role,membership_id FROM account_memberships WHERE user_id=?1 AND revoked_at IS NULL ORDER BY network_id",
        )?;
        let rows = statement.query_map([&user.0], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|r| {
            let (network, role, id) = r?;
            Ok(Membership {
                id: MembershipId(id),
                network: AccountNetwork(network),
                user: user.clone(),
                role: Role::stored(role)?,
            })
        })
        .collect()
    }

    /// Run inside the same read/write transaction as the protected resource
    /// operation. An earlier HTTP check is not a durable authorization grant.
    pub(crate) fn authorize_network(
        &self,
        principal: &Principal,
        network: &AccountNetwork,
        permission: NetworkPermission,
        now: i64,
    ) -> Result<()> {
        if matches!(principal.0, PrincipalKind::Bootstrap) {
            return Ok(());
        }
        let user = self.session_user(principal, now)?;
        match (self.role(network, &user)?, permission) {
            (Some(_), NetworkPermission::Read)
            | (Some(Role::Owner | Role::Admin), NetworkPermission::Manage) => Ok(()),
            _ => Err(AccountError::Forbidden),
        }
    }

    fn invitation(&self, digest: &[u8], now: i64) -> Result<Invitation> {
        let (network, email, role, issuer) = self.db.query_row("SELECT network_id,email,role,issuer_user_id FROM account_invitations WHERE token_hash=?1 AND expires_at>?2 AND revoked_at IS NULL AND consumed_at IS NULL", params![digest,now], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?,r.get::<_, String>(2)?,r.get::<_, Option<String>>(3)?))).optional()?.ok_or(AccountError::Unauthorized)?;
        let invitation = Invitation {
            network: AccountNetwork(network),
            email: Email::parse(&email).map_err(|_| AccountError::Storage)?,
            role: Role::stored(role)?,
            issuer: issuer.map(UserId),
        };
        // Losing membership/role also invalidates unconsumed delegated invites.
        if let Some(issuer) = &invitation.issuer {
            match self.role(&invitation.network, issuer)? {
                Some(Role::Owner) => {}
                Some(Role::Admin) if invitation.role == Role::Member => {}
                _ => return Err(AccountError::Unauthorized),
            }
        }
        Ok(invitation)
    }

    pub(crate) fn prepare_registration(
        &self,
        token: &str,
        email: &Email,
        now: i64,
    ) -> Result<RegistrationIntent> {
        let digest = Secret::digest(token, SecretKind::Invitation)?;
        let invite = self.invitation(&digest, now)?;
        if invite.email != *email || self.credential(email)?.is_some() {
            return Err(AccountError::Unauthorized);
        }
        Ok(RegistrationIntent {
            digest,
            email: email.clone(),
        })
    }

    pub(crate) fn prepare_reset(&self, token: &str, now: i64) -> Result<ResetIntent> {
        let digest = Secret::digest(token, SecretKind::Reset)?;
        let (user, version) = self.db.query_row("SELECT r.user_id,r.credential_version FROM account_password_resets r JOIN account_users u ON u.user_id=r.user_id WHERE r.token_hash=?1 AND r.expires_at>?2 AND r.revoked_at IS NULL AND r.consumed_at IS NULL AND r.credential_version=u.credential_version", params![digest,now], |r| Ok((UserId(r.get(0)?), r.get::<_, i64>(1)?))).optional()?.ok_or(AccountError::Unauthorized)?;
        Ok(ResetIntent {
            digest,
            user,
            version,
        })
    }
}

fn insert_user(
    db: &Connection,
    email: &Email,
    password: HashedPassword,
    now: i64,
) -> Result<UserId> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM account_users WHERE email=?1)",
        [email.as_str()],
        |r| r.get(0),
    )?;
    if exists {
        return Err(AccountError::Conflict);
    }
    let id = UserId(meshway_protocol::local::credential().map_err(|_| AccountError::Worker)?);
    db.execute(
        "INSERT INTO account_users(user_id,email,password_phc,created_at) VALUES (?1,?2,?3,?4)",
        params![id.0, email.as_str(), password.0.as_str(), now],
    )?;
    Ok(id)
}

fn join(
    db: &Connection,
    digest: &[u8],
    invite: &Invitation,
    user: &UserId,
    now: i64,
) -> Result<Membership> {
    // Existing membership cannot be silently elevated by accepting a link.
    if Accounts::new(db).role(&invite.network, user)?.is_some() {
        return Err(AccountError::Conflict);
    }
    let id = MembershipId(meshway_protocol::local::credential().map_err(|_| AccountError::Worker)?);
    db.execute(
        "INSERT INTO account_memberships(network_id,user_id,role,membership_id) VALUES (?1,?2,?3,?4)",
        params![invite.network.0, user.0, invite.role.as_str(),id.0],
    )?;
    let changed = db.execute("UPDATE account_invitations SET consumed_by=?1,consumed_at=?2 WHERE token_hash=?3 AND consumed_at IS NULL AND revoked_at IS NULL AND expires_at>?2", params![user.0,now,digest])?;
    if changed != 1 {
        return Err(AccountError::Unauthorized);
    }
    Ok(Membership {
        id,
        network: invite.network.clone(),
        user: user.clone(),
        role: invite.role,
    })
}

pub(crate) fn create_invitation(
    tx: &mut Transaction<'_>,
    principal: &Principal,
    network: &AccountNetwork,
    email: &Email,
    role: Role,
    now: i64,
    ttl: i64,
) -> Result<IssuedSecret> {
    atomic(tx, |db| {
        let issuer = Accounts::new(db).may_grant(principal, network, role, now)?;
        let expires_at = expiry(now, ttl, INVITE_MAX)?;
        let secret = Secret::generate(SecretKind::Invitation)?;
        let digest = Secret::digest(secret.expose(), SecretKind::Invitation)?;
        let id = meshway_protocol::local::credential().map_err(|_| AccountError::Worker)?;
        db.execute("INSERT INTO account_invitations(invitation_id,token_hash,network_id,email,role,issuer_user_id,expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![id,digest,network.0,email.as_str(),role.as_str(),issuer.map(|u| u.0),expires_at])?;
        Ok(IssuedSecret { secret, expires_at })
    })
}

pub(crate) fn revoke_invitation(
    tx: &mut Transaction<'_>,
    principal: &Principal,
    network: &AccountNetwork,
    token: &str,
    now: i64,
) -> Result<()> {
    atomic(tx, |db| {
        let digest = Secret::digest(token, SecretKind::Invitation)?;
        let role = db
            .query_row(
                "SELECT role FROM account_invitations WHERE network_id=?1 AND token_hash=?2",
                params![network.0, digest],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .ok_or(AccountError::Forbidden)?;
        Accounts::new(db).may_grant(principal, network, Role::stored(role)?, now)?;
        db.execute("UPDATE account_invitations SET revoked_at=COALESCE(revoked_at,?1) WHERE network_id=?2 AND token_hash=?3", params![now,network.0,digest])?;
        Ok(())
    })
}

pub(crate) fn register_invited(
    tx: &mut Transaction<'_>,
    intent: RegistrationIntent,
    password: HashedPassword,
    now: i64,
) -> Result<Membership> {
    atomic(tx, |db| {
        let invite = Accounts::new(db).invitation(&intent.digest, now)?;
        if invite.email != intent.email {
            return Err(AccountError::Unauthorized);
        }
        let user = insert_user(db, &intent.email, password, now)?;
        join(db, &intent.digest, &invite, &user, now)
    })
}

pub(crate) fn accept_invitation(
    tx: &mut Transaction<'_>,
    principal: &Principal,
    token: &str,
    now: i64,
) -> Result<Membership> {
    atomic(tx, |db| {
        let repo = Accounts::new(db);
        let user = repo.session_user(principal, now)?;
        let digest = Secret::digest(token, SecretKind::Invitation)?;
        let invite = repo.invitation(&digest, now)?;
        if repo.user(&user)?.email != invite.email {
            return Err(AccountError::Unauthorized);
        }
        join(db, &digest, &invite, &user, now)
    })
}

/// Explicit first-Owner claim, not a migration-generated fictitious user.
/// Caller has proven the existing instance bearer through prove_bootstrap.
pub(crate) fn claim_first_owner(
    tx: &mut Transaction<'_>,
    principal: &Principal,
    network: &AccountNetwork,
    email: &Email,
    password: HashedPassword,
    now: i64,
) -> Result<Membership> {
    atomic(tx, |db| {
        if !matches!(principal.0, PrincipalKind::Bootstrap) {
            return Err(AccountError::Forbidden);
        }
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM account_memberships WHERE network_id=?1 AND role='owner' AND revoked_at IS NULL)",
            [&network.0],
            |r| r.get(0),
        )?;
        if exists {
            return Err(AccountError::Conflict);
        }
        let user = insert_user(db, email, password, now)?;
        let id =
            MembershipId(meshway_protocol::local::credential().map_err(|_| AccountError::Worker)?);
        db.execute(
            "INSERT INTO account_memberships(network_id,user_id,role,membership_id) VALUES (?1,?2,'owner',?3)",
            params![network.0, user.0,id.0],
        )?;
        Ok(Membership {
            id,
            network: network.clone(),
            user,
            role: Role::Owner,
        })
    })
}

/// Claim another legacy network as an existing logged-in account without
/// replacing its password. Both bootstrap proof and user session are required.
pub(crate) fn claim_existing_owner(
    tx: &mut Transaction<'_>,
    bootstrap: &Principal,
    session: &Principal,
    network: &AccountNetwork,
    now: i64,
) -> Result<Membership> {
    atomic(tx, |db| {
        if !matches!(bootstrap.0, PrincipalKind::Bootstrap) {
            return Err(AccountError::Forbidden);
        }
        let user = Accounts::new(db).session_user(session, now)?;
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM account_memberships WHERE network_id=?1 AND role='owner' AND revoked_at IS NULL)",
            [&network.0],
            |r| r.get(0),
        )?;
        if exists {
            return Err(AccountError::Conflict);
        }
        let id = meshway_protocol::local::credential().map_err(|_| AccountError::Worker)?;
        db.execute("INSERT INTO account_memberships(network_id,user_id,role,membership_id) VALUES (?1,?2,'owner',?3) ON CONFLICT(network_id,user_id) WHERE revoked_at IS NULL DO UPDATE SET role='owner'", params![network.0,user.0,id])?;
        Accounts::new(db)
            .membership(network, &user)?
            .ok_or(AccountError::Storage)
    })
}

pub(crate) fn finish_login(
    tx: &mut Transaction<'_>,
    verified: VerifiedCredential,
    now: i64,
    ttl: i64,
) -> Result<IssuedSecret> {
    atomic(tx, |db| {
        let version: i64 = db.query_row(
            "SELECT credential_version FROM account_users WHERE user_id=?1",
            [&verified.user.0],
            |r| r.get(0),
        )?;
        if version != verified.version {
            return Err(AccountError::Unauthorized);
        }
        let expires_at = expiry(now, ttl, SESSION_MAX)?;
        let secret = Secret::generate(SecretKind::Session)?;
        db.execute("INSERT INTO account_sessions(token_hash,user_id,credential_version,expires_at) VALUES (?1,?2,?3,?4)", params![Secret::digest(secret.expose(),SecretKind::Session)?,verified.user.0,version,expires_at])?;
        Ok(IssuedSecret { secret, expires_at })
    })
}

pub(crate) fn logout(tx: &mut Transaction<'_>, principal: &Principal, now: i64) -> Result<()> {
    atomic(tx, |db| {
        Accounts::new(db).session_user(principal, now)?;
        let PrincipalKind::Session { digest, .. } = &principal.0 else {
            return Err(AccountError::Forbidden);
        };
        db.execute(
            "UPDATE account_sessions SET revoked_at=?1 WHERE token_hash=?2",
            params![now, digest],
        )?;
        Ok(())
    })
}

pub(crate) fn revoke_user_sessions(
    tx: &mut Transaction<'_>,
    principal: &Principal,
    user: &UserId,
    now: i64,
) -> Result<()> {
    atomic(tx, |db| {
        if !matches!(principal.0, PrincipalKind::Bootstrap)
            && Accounts::new(db).session_user(principal, now)? != *user
        {
            return Err(AccountError::Forbidden);
        }
        db.execute(
            "UPDATE account_sessions SET revoked_at=COALESCE(revoked_at,?1) WHERE user_id=?2",
            params![now, user.0],
        )?;
        Ok(())
    })
}

pub(crate) fn issue_reset(
    tx: &mut Transaction<'_>,
    principal: &Principal,
    user: &UserId,
    now: i64,
    ttl: i64,
) -> Result<IssuedSecret> {
    atomic(tx, |db| {
        if !matches!(principal.0, PrincipalKind::Bootstrap) {
            return Err(AccountError::Forbidden);
        }
        let version: i64 = db.query_row(
            "SELECT credential_version FROM account_users WHERE user_id=?1",
            [&user.0],
            |r| r.get(0),
        )?;
        let expires_at = expiry(now, ttl, RESET_MAX)?;
        let secret = Secret::generate(SecretKind::Reset)?;
        db.execute("INSERT INTO account_password_resets(token_hash,user_id,credential_version,expires_at) VALUES (?1,?2,?3,?4)", params![Secret::digest(secret.expose(),SecretKind::Reset)?,user.0,version,expires_at])?;
        Ok(IssuedSecret { secret, expires_at })
    })
}

fn replace_password(
    db: &Connection,
    user: &UserId,
    version: i64,
    password: HashedPassword,
    now: i64,
) -> Result<()> {
    let changed = db.execute("UPDATE account_users SET password_phc=?1,credential_version=credential_version+1 WHERE user_id=?2 AND credential_version=?3 AND credential_version<9223372036854775807", params![password.0.as_str(),user.0,version])?;
    if changed != 1 {
        return Err(AccountError::Unauthorized);
    }
    db.execute(
        "UPDATE account_sessions SET revoked_at=COALESCE(revoked_at,?1) WHERE user_id=?2",
        params![now, user.0],
    )?;
    db.execute("UPDATE account_password_resets SET revoked_at=COALESCE(revoked_at,?1) WHERE user_id=?2 AND consumed_at IS NULL", params![now,user.0])?;
    Ok(())
}

pub(crate) fn redeem_reset(
    tx: &mut Transaction<'_>,
    intent: ResetIntent,
    password: HashedPassword,
    now: i64,
) -> Result<()> {
    atomic(tx, |db| {
        let valid: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM account_password_resets WHERE token_hash=?1 AND user_id=?2 AND credential_version=?3 AND expires_at>?4 AND revoked_at IS NULL AND consumed_at IS NULL)", params![intent.digest,intent.user.0,intent.version,now], |r| r.get(0))?;
        if !valid {
            return Err(AccountError::Unauthorized);
        }
        db.execute(
            "UPDATE account_password_resets SET consumed_at=?1 WHERE token_hash=?2",
            params![now, intent.digest],
        )?;
        replace_password(db, &intent.user, intent.version, password, now)
    })
}

pub(crate) fn change_password(
    tx: &mut Transaction<'_>,
    principal: &Principal,
    verified: VerifiedCredential,
    password: HashedPassword,
    now: i64,
) -> Result<()> {
    atomic(tx, |db| {
        let user = Accounts::new(db).session_user(principal, now)?;
        if user != verified.user {
            return Err(AccountError::Forbidden);
        }
        replace_password(db, &user, verified.version, password, now)
    })
}

pub(crate) fn revoke_reset(
    tx: &mut Transaction<'_>,
    principal: &Principal,
    token: &str,
    now: i64,
) -> Result<()> {
    atomic(tx, |db| {
        if !matches!(principal.0, PrincipalKind::Bootstrap) {
            return Err(AccountError::Forbidden);
        }
        let digest = Secret::digest(token, SecretKind::Reset)?;
        db.execute("UPDATE account_password_resets SET revoked_at=COALESCE(revoked_at,?1) WHERE token_hash=?2", params![now,digest])?;
        Ok(())
    })
}

/// Owner/instance admin manage roles. A network Admin may remove Members only.
/// Baseline: refuse removal/demotion of the final Owner. Promote a successor in
/// the same outer transaction first; a dedicated transfer endpoint can wrap it.
pub(crate) fn set_membership(
    tx: &mut Transaction<'_>,
    principal: &Principal,
    network: &AccountNetwork,
    user: &UserId,
    desired: Option<Role>,
    now: i64,
) -> Result<()> {
    atomic(tx, |db| {
        let repo = Accounts::new(db);
        let old = repo.role(network, user)?.ok_or(AccountError::Forbidden)?;
        repo.may_grant(principal, network, old, now)?;
        if let Some(new) = desired {
            repo.may_grant(principal, network, new, now)?;
        }
        if old == Role::Owner && desired != Some(Role::Owner) {
            let owners: i64 = db.query_row(
                "SELECT COUNT(*) FROM account_memberships WHERE network_id=?1 AND role='owner' AND revoked_at IS NULL",
                [&network.0],
                |r| r.get(0),
            )?;
            if owners <= 1 {
                return Err(AccountError::LastOwner);
            }
        }
        if let Some(role) = desired {
            db.execute(
                "UPDATE account_memberships SET role=?1 WHERE network_id=?2 AND user_id=?3 AND revoked_at IS NULL",
                params![role.as_str(), network.0, user.0],
            )?;
        } else {
            db.execute(
                "UPDATE account_memberships SET revoked_at=?3 WHERE network_id=?1 AND user_id=?2 AND revoked_at IS NULL",
                params![network.0, user.0,now],
            )?;
        }
        Ok(())
    })
}
