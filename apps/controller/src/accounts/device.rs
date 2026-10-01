//! Authentication-source-independent device authorization integration point.
//! PKCE/code/node creation stays in the existing production device service.

use super::*;

/// Persist beside the future device code/request in the same signing transaction.
/// This is internal SQLite data, not an HTTP DTO. No cookie/token is copied;
/// the session digest is enough to recheck revocation. Never expose it to App/CLI.
pub(crate) struct DeviceAuthorizationBinding {
    pub(crate) network: AccountNetwork,
    pub(crate) user: UserId,
    pub(crate) membership_id: MembershipId,
    pub(crate) session_digest: Vec<u8>,
    pub(crate) credential_version: i64,
}

/// Validated result of the shared human authorization service. Cannot be
/// constructed outside this module; bootstrap and node credentials cannot
/// masquerade as a user-owned device admission.
pub(crate) struct DeviceAuthorizationSubject {
    binding: DeviceAuthorizationBinding,
    membership: Membership,
}

impl DeviceAuthorizationSubject {
    pub(crate) fn binding(&self) -> &DeviceAuthorizationBinding {
        &self.binding
    }

    pub(crate) fn owner(&self) -> &Membership {
        &self.membership
    }
}

impl Accounts<'_> {
    /// Browser authorization: an already logged-in user can select only a
    /// network they currently belong to. Member is sufficient for their own
    /// device; this is not network-management authority or tag-owned admission.
    /// Call inside the transaction that signs the PKCE code/request binding.
    pub(crate) fn authorize_device(
        &self,
        principal: &Principal,
        network: &AccountNetwork,
        now: i64,
    ) -> Result<DeviceAuthorizationSubject> {
        let user = self.session_user(principal, now)?;
        let membership = self
            .membership(network, &user)?
            .ok_or(AccountError::Forbidden)?;
        let PrincipalKind::Session {
            digest, version, ..
        } = &principal.0
        else {
            return Err(AccountError::Forbidden);
        };
        Ok(DeviceAuthorizationSubject {
            binding: DeviceAuthorizationBinding {
                network: network.clone(),
                user: user.clone(),
                membership_id: membership.id.clone(),
                session_digest: digest.clone(),
                credential_version: *version,
            },
            membership,
        })
    }

    /// Revalidate persisted approval during *first* code exchange, before
    /// node/address creation, in the same root transaction as consuming code.
    /// Logout/session expiry/reset or membership removal rejects admission.
    /// expected_network comes from the immutable PKCE code/request, never from
    /// a caller-supplied override. Existing-node exact retry recovery remains
    /// the device service's separate binding/revocation check.
    pub(crate) fn revalidate_device(
        &self,
        binding: &DeviceAuthorizationBinding,
        expected_network: &AccountNetwork,
        now: i64,
    ) -> Result<DeviceAuthorizationSubject> {
        if binding.network != *expected_network || binding.session_digest.len() != 32 {
            return Err(AccountError::Unauthorized);
        }
        let principal = Principal(PrincipalKind::Session {
            digest: binding.session_digest.clone(),
            user: binding.user.clone(),
            version: binding.credential_version,
        });
        let subject = self.authorize_device(&principal, expected_network, now)?;
        if subject.owner().id != binding.membership_id {
            return Err(AccountError::Unauthorized);
        }
        Ok(subject)
    }
}
