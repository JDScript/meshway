use super::{NetworkId, Store, ownership};
use crate::{Error, accounts::*};
use meshway_protocol::accounts::*;
use rusqlite::{Transaction, TransactionBehavior};
use std::result::Result;

impl From<AccountError> for Error {
    fn from(value: AccountError) -> Self {
        use axum::http::StatusCode;
        match value {
            AccountError::Invalid => Self::Invalid("invalid account input"),
            AccountError::Unauthorized => Self::Unauthorized,
            AccountError::Forbidden => {
                Self::Body(StatusCode::FORBIDDEN, "account operation not permitted")
            }
            AccountError::Conflict => Self::Conflict,
            AccountError::LastOwner => Self::Body(
                StatusCode::CONFLICT,
                "the last network owner cannot be removed",
            ),
            AccountError::Busy => Self::Body(
                StatusCode::TOO_MANY_REQUESTS,
                "authentication budget exhausted",
            ),
            AccountError::Storage | AccountError::Worker => Self::Worker,
        }
    }
}

fn member(m: Membership) -> AccountMembership {
    AccountMembership {
        membership_id: m.id.as_str().into(),
        network_id: m.network.as_str().into(),
        user_id: m.user.as_str().into(),
        role: m.role.into(),
    }
}
fn invite(m: InvitationMetadata) -> AccountInvitation {
    AccountInvitation {
        invitation_id: m.id.as_str().into(),
        network_id: m.network.as_str().into(),
        email: m.email.as_str().into(),
        role: m.role.into(),
        issuer_user_id: m.issuer.map(|u| u.as_str().into()),
        expires_at: m.expires_at,
        revoked_at: m.revoked_at,
        consumed_by: m.consumed_by.map(|u| u.as_str().into()),
        consumed_at: m.consumed_at,
    }
}
fn reset(m: PasswordResetMetadata) -> AccountPasswordReset {
    AccountPasswordReset {
        reset_id: m.id.as_str().into(),
        user_id: m.user.as_str().into(),
        expires_at: m.expires_at,
        revoked_at: m.revoked_at,
        consumed_at: m.consumed_at,
        credential_current: m.credential_current,
    }
}

impl Store {
    // Only these Store wrappers expose results; API never receives a Connection
    // or Transaction. Original token values escape only after commit succeeds.
    fn account_transaction<T>(
        &mut self,
        token: &str,
        f: impl FnOnce(&mut Transaction<'_>, &Principal, i64) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = chrono::Utc::now().timestamp();
        let principal = Principal::authenticate_source(&tx, token, now)?;
        let result = f(&mut tx, &principal, now)?;
        tx.commit()?;
        Ok(result)
    }
    pub(crate) fn account_credential(
        &self,
        email: &Email,
    ) -> Result<Option<CredentialSnapshot>, Error> {
        Ok(Accounts::new(&self.connection).credential(email)?)
    }
    pub(crate) fn account_registration(
        &self,
        token: &str,
        email: &Email,
    ) -> Result<RegistrationIntent, Error> {
        Ok(Accounts::new(&self.connection).prepare_registration(
            token,
            email,
            chrono::Utc::now().timestamp(),
        )?)
    }
    pub(crate) fn account_reset_intent(&self, token: &str) -> Result<ResetIntent, Error> {
        Ok(Accounts::new(&self.connection).prepare_reset(token, chrono::Utc::now().timestamp())?)
    }
    pub(crate) fn account_login(
        &mut self,
        proof: VerifiedCredential,
    ) -> Result<LoginResponse, Error> {
        let mut tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = finish_login(&mut tx, proof, chrono::Utc::now().timestamp(), 7 * 86400)?;
        tx.commit()?;
        Ok(LoginResponse {
            session_token: result.secret.expose().into(),
            expires_at: result.expires_at,
        })
    }
    pub(crate) fn account_session(&mut self, token: &str) -> Result<AccountSession, Error> {
        self.account_transaction(token, |tx, p, now| {
            let repo = Accounts::new(tx);
            let user = repo.user(&repo.session_user(p, now)?)?;
            Ok(AccountSession {
                user: AccountUser {
                    user_id: user.id.as_str().into(),
                    email: user.email.as_str().into(),
                    email_verified: user.email_verified,
                },
                memberships: repo.memberships(p, now)?.into_iter().map(member).collect(),
            })
        })
    }
    pub(crate) fn account_logout(&mut self, token: &str) -> Result<(), Error> {
        self.account_transaction(token, |tx, p, now| Ok(logout(tx, p, now)?))
    }
    pub(crate) fn account_register(
        &mut self,
        intent: RegistrationIntent,
        hash: HashedPassword,
    ) -> Result<AccountMembership, Error> {
        let mut tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let membership = register_invited(&mut tx, intent, hash, chrono::Utc::now().timestamp())?;
        tx.commit()?;
        Ok(member(membership))
    }
    pub(crate) fn account_accept(
        &mut self,
        session: &str,
        token: &str,
    ) -> Result<AccountMembership, Error> {
        self.account_transaction(session, |tx, p, now| {
            Ok(member(accept_invitation(tx, p, token, now)?))
        })
    }
    pub(crate) fn account_claim_new(
        &mut self,
        token: &str,
        id: &str,
        email: Email,
        hash: HashedPassword,
    ) -> Result<AccountMembership, Error> {
        self.account_transaction(token, |tx, p, now| {
            let network = NetworkId::lookup(tx, id)?;
            Ok(member(claim_first_owner(
                tx, p, &network, &email, hash, now,
            )?))
        })
    }
    pub(crate) fn account_claim_existing(
        &mut self,
        token: &str,
        id: &str,
        session: &str,
    ) -> Result<AccountMembership, Error> {
        self.account_transaction(token, |tx, p, now| {
            let network = NetworkId::lookup(tx, id)?;
            let session = Accounts::new(tx).authenticate(session, now)?;
            Ok(member(claim_existing_owner(
                tx, p, &session, &network, now,
            )?))
        })
    }
    pub(crate) fn account_members(
        &mut self,
        token: &str,
        id: &str,
    ) -> Result<AccountMembers, Error> {
        self.account_transaction(token, |tx, p, now| {
            let network = NetworkId::lookup(tx, id)?;
            Ok(AccountMembers {
                members: Accounts::new(tx)
                    .members(p, &network, now)?
                    .into_iter()
                    .map(|m| {
                        let user = Accounts::new(tx).user(&m.user)?;
                        Ok(AccountMember {
                            membership: member(m),
                            user: AccountUser {
                                user_id: user.id.as_str().into(),
                                email: user.email.as_str().into(),
                                email_verified: user.email_verified,
                            },
                        })
                    })
                    .collect::<Result<_, Error>>()?,
            })
        })
    }
    pub(crate) fn account_save_member(
        &mut self,
        token: &str,
        id: &str,
        request: SaveMembershipRequest,
    ) -> Result<(), Error> {
        let user = UserId::parse(&request.user_id)?;
        let desired = match request.role {
            AccountMembershipChange::Owner => Some(Role::Owner),
            AccountMembershipChange::Admin => Some(Role::Admin),
            AccountMembershipChange::Member => Some(Role::Member),
            AccountMembershipChange::Remove => None,
        };
        self.account_transaction(token, |tx, p, now| {
            let network = NetworkId::lookup(tx, id)?;
            set_membership(tx, p, &network, &user, desired, now)?;
            if desired.is_none() {
                ownership::withdraw_user(tx, &network, &user)?;
            }
            Ok(())
        })
    }
    pub(crate) fn account_leave(&mut self, token: &str, id: &str) -> Result<(), Error> {
        self.account_transaction(token, |tx, p, now| {
            let network = NetworkId::lookup(tx, id)?;
            let user = leave_network(tx, p, &network, now)?;
            ownership::withdraw_user(tx, &network, &user)
        })
    }
    pub(crate) fn account_invitations(
        &mut self,
        token: &str,
        id: &str,
    ) -> Result<AccountInvitations, Error> {
        self.account_transaction(token, |tx, p, now| {
            let network = NetworkId::lookup(tx, id)?;
            Ok(AccountInvitations {
                invitations: Accounts::new(tx)
                    .list_invitations(p, &network, now)?
                    .into_iter()
                    .map(invite)
                    .collect(),
            })
        })
    }
    pub(crate) fn account_invitation(
        &mut self,
        token: &str,
        id: &str,
        resource: &str,
    ) -> Result<AccountInvitation, Error> {
        let resource = InvitationId::parse(resource)?;
        self.account_transaction(token, |tx, p, now| {
            let network = NetworkId::lookup(tx, id)?;
            Ok(invite(
                Accounts::new(tx).invitation_metadata(p, &network, &resource, now)?,
            ))
        })
    }
    pub(crate) fn account_invite(
        &mut self,
        token: &str,
        id: &str,
        request: CreateInvitationRequest,
    ) -> Result<CreateInvitationResponse, Error> {
        let email = Email::parse(&request.email)?;
        let result = self.account_transaction(token, |tx, p, now| {
            let network = NetworkId::lookup(tx, id)?;
            Ok(create_invitation(
                tx,
                p,
                &network,
                &email,
                request.role.into(),
                now,
                request.expires_in_seconds.into(),
            )?)
        })?;
        Ok(CreateInvitationResponse {
            invitation: invite(result.metadata),
            invitation_token: result.secret.expose().into(),
        })
    }
    pub(crate) fn account_revoke_invite(
        &mut self,
        token: &str,
        id: &str,
        resource: &str,
    ) -> Result<(), Error> {
        let resource = InvitationId::parse(resource)?;
        self.account_transaction(token, |tx, p, now| {
            let network = NetworkId::lookup(tx, id)?;
            Ok(revoke_invitation(tx, p, &network, &resource, now)?)
        })
    }
    pub(crate) fn account_revoke_sessions(&mut self, token: &str, id: &str) -> Result<(), Error> {
        let user = UserId::parse(id)?;
        self.account_transaction(token, |tx, p, now| {
            Ok(revoke_user_sessions(tx, p, &user, now)?)
        })
    }
    pub(crate) fn account_resets(&mut self, token: &str) -> Result<AccountPasswordResets, Error> {
        self.account_transaction(token, |tx, p, _| {
            Ok(AccountPasswordResets {
                resets: Accounts::new(tx)
                    .list_password_resets(p)?
                    .into_iter()
                    .map(reset)
                    .collect(),
            })
        })
    }
    pub(crate) fn account_reset(
        &mut self,
        token: &str,
        resource: &str,
    ) -> Result<AccountPasswordReset, Error> {
        let resource = PasswordResetId::parse(resource)?;
        self.account_transaction(token, |tx, p, _| {
            Ok(reset(
                Accounts::new(tx).password_reset_metadata(p, &resource)?,
            ))
        })
    }
    pub(crate) fn account_issue_reset(
        &mut self,
        token: &str,
        request: CreatePasswordResetRequest,
    ) -> Result<CreatePasswordResetResponse, Error> {
        let user = UserId::parse(&request.user_id)?;
        let result = self.account_transaction(token, |tx, p, now| {
            Ok(issue_reset(
                tx,
                p,
                &user,
                now,
                request.expires_in_seconds.into(),
            )?)
        })?;
        Ok(CreatePasswordResetResponse {
            reset: reset(result.metadata),
            reset_token: result.secret.expose().into(),
        })
    }
    pub(crate) fn account_revoke_reset(
        &mut self,
        token: &str,
        resource: &str,
    ) -> Result<(), Error> {
        let resource = PasswordResetId::parse(resource)?;
        self.account_transaction(token, |tx, p, now| Ok(revoke_reset(tx, p, &resource, now)?))
    }
    pub(crate) fn account_redeem_reset(
        &mut self,
        intent: ResetIntent,
        hash: HashedPassword,
    ) -> Result<(), Error> {
        let mut tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        redeem_reset(&mut tx, intent, hash, chrono::Utc::now().timestamp())?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn account_change_password(
        &mut self,
        token: &str,
        proof: VerifiedCredential,
        hash: HashedPassword,
    ) -> Result<(), Error> {
        self.account_transaction(token, |tx, p, now| {
            Ok(change_password(tx, p, proof, hash, now)?)
        })
    }
}
