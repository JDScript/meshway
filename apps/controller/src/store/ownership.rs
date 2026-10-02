use super::{NetworkId, Store, access, configuration, connections, network};
use crate::{Error, accounts::*};
use meshway_protocol::{AuthorizeDeviceRequest, AuthorizeDeviceResponse, accounts::*};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::result::Result;

pub(super) fn installed(db: &Connection) -> Result<bool, Error> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='device_account_bindings')",[],|r|r.get(0))?)
}

pub(super) fn binding(
    db: &Connection,
    network: &NetworkId,
    code: &str,
) -> Result<Option<DeviceAuthorizationBinding>, Error> {
    // Uninstalled legacy Store fixtures retain their original enrollment contract.
    if !installed(db)? {
        return Ok(None);
    }
    let stored = db.query_row("SELECT b.user_id,b.membership_id,b.session_digest,b.credential_version FROM device_account_bindings b JOIN enrollment_tokens e ON e.id=b.token_id AND e.network_id=b.network_id WHERE e.token_hash=?1 AND b.network_id=?2", params![meshway_protocol::local::digest(code),network.as_str()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,i64>(3)?))).optional()?;
    stored
        .map(|(user, id, digest, version)| {
            Ok(DeviceAuthorizationBinding {
                network: network.clone(),
                user: UserId::parse(&user)?,
                membership_id: MembershipId::parse(&id)?,
                session_digest: digest,
                credential_version: version,
            })
        })
        .transpose()
}

pub(super) fn retry(
    db: &Connection,
    binding: &DeviceAuthorizationBinding,
    node_id: &str,
) -> Result<(), Error> {
    let valid: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM node_ownership o JOIN account_memberships m ON m.membership_id=o.membership_id AND m.network_id=o.network_id WHERE o.network_id=?1 AND o.node_id=?2 AND o.membership_id=?3 AND m.user_id=?4 AND m.revoked_at IS NULL)", params![binding.network.as_str(),node_id,binding.membership_id.as_str(),binding.user.as_str()],|r|r.get(0))?;
    if !valid {
        return Err(Error::Forbidden);
    }
    Ok(())
}

pub(super) fn assign(
    db: &Connection,
    network: &NetworkId,
    node_id: &str,
    membership: &MembershipId,
) -> Result<(), Error> {
    let changed = db.execute("UPDATE node_ownership SET membership_id=?3 WHERE network_id=?1 AND node_id=?2 AND membership_id IS NULL",params![network.as_str(),node_id,membership.as_str()])?;
    if changed != 1 {
        return Err(Error::Conflict);
    }
    Ok(())
}

pub(super) fn withdraw_user(
    db: &Connection,
    scope: &NetworkId,
    user: &UserId,
) -> Result<(), Error> {
    let changed = db.execute("UPDATE nodes SET revoked_at=?3 WHERE network_id=?1 AND revoked_at IS NULL AND id IN (SELECT o.node_id FROM node_ownership o JOIN account_memberships m ON m.membership_id=o.membership_id AND m.network_id=o.network_id WHERE o.network_id=?1 AND m.user_id=?2 AND m.revoked_at IS NOT NULL)",params![scope.as_str(),user.as_str(),chrono::Utc::now().to_rfc3339()])?;
    // Include previously revoked devices of this user's revoked memberships.
    // This repairs old orphans on an authorized retry, without touching another
    // user's devices, an active membership, or the tag catalog.
    let withdrawn = "SELECT n.id FROM nodes n JOIN node_ownership o ON o.network_id=n.network_id AND o.node_id=n.id JOIN account_memberships m ON m.network_id=o.network_id AND m.membership_id=o.membership_id WHERE n.network_id=?1 AND n.revoked_at IS NOT NULL AND m.user_id=?2 AND m.revoked_at IS NOT NULL";
    let tags = db.execute(
        &format!("DELETE FROM node_tags WHERE network_id=?1 AND node_id IN ({withdrawn})"),
        params![scope.as_str(), user.as_str()],
    )?;
    db.execute(&format!("DELETE FROM connection_runtime_reports WHERE network_id=?1 AND node_id IN ({withdrawn})"),params![scope.as_str(),user.as_str()])?;
    if changed > 0 || tags > 0 {
        configuration::bump(db, scope)?;
        configuration::ensure_publishable(db, scope)?;
        connections::ensure_publishable(db, scope)?;
        access::ensure_publishable(db, scope)?;
        network::ensure_publishable(db, scope)?;
    }
    Ok(())
}

impl Store {
    /// Branch integration hook only: additive fragment, no schema version claim.
    /// Release integration must place install in the coordinated versioned migration.
    pub(crate) fn install_account_integration_schema(&mut self) -> Result<(), Error> {
        if installed(&self.connection)? {
            let tables: i64 = self.connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('account_users','account_memberships','account_invitations','account_sessions','account_password_resets','node_ownership','device_account_bindings')",[],|r|r.get(0))?;
            if tables != 7
                || self
                    .connection
                    .prepare("PRAGMA foreign_key_check")?
                    .exists([])?
            {
                return Err(Error::Configuration(
                    "incomplete account integration schema",
                ));
            }
            return Ok(());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        super::account_schema::install(&tx)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn authorize_user_device(
        &mut self,
        token: &str,
        scope: &str,
        request: &AuthorizeDeviceRequest,
    ) -> Result<AuthorizeDeviceResponse, Error> {
        request.validate().map_err(Error::Invalid)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = chrono::Utc::now();
        let principal = Accounts::new(&tx).authenticate(token, now.timestamp())?;
        let scope = NetworkId::lookup(&tx, scope)?;
        let subject = Accounts::new(&tx).authorize_device(&principal, &scope, now.timestamp())?;
        let binding = subject.binding();
        let code = meshway_protocol::local::credential()?;
        let id = meshway_protocol::local::credential()?;
        let expires_at = now + chrono::Duration::seconds(300);
        let request = serde_json::to_string(request)
            .map_err(|_| Error::Invalid("invalid device authorization"))?;
        tx.execute("INSERT INTO enrollment_tokens(id,token_hash,expires_at,device_binding,network_id) VALUES(?1,?2,?3,?4,?5)",params![id,meshway_protocol::local::digest(&code),expires_at.timestamp_millis(),request,scope.as_str()])?;
        tx.execute("INSERT INTO device_account_bindings(token_id,network_id,user_id,membership_id,session_digest,credential_version) VALUES(?1,?2,?3,?4,?5,?6)",params![id,scope.as_str(),binding.user.as_str(),binding.membership_id.as_str(),binding.session_digest,binding.credential_version])?;
        tx.commit()?;
        Ok(AuthorizeDeviceResponse { code, expires_at })
    }
    pub(crate) fn node_ownership(
        &mut self,
        token: &str,
        scope: &str,
    ) -> Result<NodeOwnershipList, Error> {
        self.network_operation(token,scope,NetworkPermission::Read,|store,scope| {
            let mut query = store.connection.prepare("SELECT node_id,membership_id FROM node_ownership WHERE network_id=?1 ORDER BY node_id")?;
            let nodes = query.query_map([scope.as_str()],|r|Ok(NodeOwnership { node_id: r.get(0)?, network_id: scope.as_str().into(), membership_id: r.get(1)? }))?.collect::<Result<_,_>>()?;
            Ok(NodeOwnershipList { nodes })
        })
    }
    pub(crate) fn claim_node_ownership(
        &mut self,
        token: &str,
        scope: &str,
        request: ClaimNodeOwnershipRequest,
    ) -> Result<(), Error> {
        self.network_operation(token, scope, NetworkPermission::Manage, |store, scope| {
            let now = chrono::Utc::now().timestamp();
            let p = store.principal(token, now)?;
            let repo = Accounts::new(&store.connection);
            repo.require_owner(&p, scope, now)?;
            let membership = repo
                .members(&p, scope, now)?
                .into_iter()
                .find(|m| m.id.as_str() == request.membership_id)
                .ok_or(Error::Forbidden)?;
            let node = super::find_node(&store.connection, scope, &request.node_id)?;
            if node.revoked_at.is_some() {
                return Err(Error::Forbidden);
            }
            assign(&store.connection, scope, &node.id, &membership.id)?;
            configuration::bump(&store.connection, scope)?;
            Ok(())
        })
    }
}
