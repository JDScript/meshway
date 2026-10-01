use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub(crate) enum AccountError {
    #[error("invalid account input")]
    Invalid,
    #[error("invalid or expired account credential")]
    Unauthorized,
    #[error("account operation not permitted")]
    Forbidden,
    #[error("account state changed; retry operation")]
    Conflict,
    #[error("the last network owner cannot be removed")]
    LastOwner,
    #[error("authentication capacity exhausted")]
    Busy,
    #[error("account storage unavailable")]
    Storage,
    #[error("authentication worker unavailable")]
    Worker,
}

impl From<rusqlite::Error> for AccountError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

pub(crate) type Result<T> = std::result::Result<T, AccountError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UserId(pub(super) String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MembershipId(pub(super) String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Email(String);

impl Email {
    /// ASCII mailbox policy for this local-account slice. No provider-specific
    /// dot/plus folding; Unicode domains must be supplied as IDNA ASCII.
    pub(crate) fn parse(input: &str) -> Result<Self> {
        let value = input.trim().to_ascii_lowercase();
        let Some((local, domain)) = value.split_once('@') else {
            return Err(AccountError::Invalid);
        };
        if value.len() > 254
            || local.is_empty()
            || local.len() > 64
            || domain.is_empty()
            || !value.is_ascii()
            || value
                .bytes()
                .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
            || !local
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&b))
            || local.starts_with('.')
            || local.ends_with('.')
            || local.contains("..")
            || domain.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return Err(AccountError::Invalid);
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A database-validated resource scope, never a permission. Kept internal until
/// Store's NetworkId string accessor is available to sibling modules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AccountNetwork(pub(super) String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Owner,
    Admin,
    Member,
}

impl Role {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }

    pub(super) fn stored(value: String) -> Result<Self> {
        match value.as_str() {
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            "member" => Ok(Self::Member),
            _ => Err(AccountError::Storage),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct User {
    pub(crate) id: UserId,
    pub(crate) email: Email,
    // A copied invitation link is not evidence of mailbox verification.
    pub(crate) email_verified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Membership {
    pub(crate) id: MembershipId,
    pub(crate) network: AccountNetwork,
    pub(crate) user: UserId,
    pub(crate) role: Role,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NetworkPermission {
    Read,
    Manage,
}

/// No Serialize or Debug implementation on secrets or credential snapshots.
pub(crate) struct Password(pub(super) Zeroizing<String>);

impl Password {
    pub(crate) fn new(value: String) -> Result<Self> {
        let value = Zeroizing::new(value);
        if !(12..=1024).contains(&value.len()) {
            return Err(AccountError::Invalid);
        }
        Ok(Self(value))
    }
}

pub(crate) struct HashedPassword(pub(super) Zeroizing<String>);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SecretKind {
    Invitation,
    Reset,
    Session,
}

impl SecretKind {
    fn prefix(self) -> &'static str {
        match self {
            Self::Invitation => "mwi_",
            Self::Reset => "mwr_",
            Self::Session => "mws_",
        }
    }
}

/// Returned once to the trusted caller for a cookie/link. Never logged.
pub(crate) struct Secret(pub(super) Zeroizing<String>);

impl Secret {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    pub(super) fn generate(kind: SecretKind) -> Result<Self> {
        Ok(Self(Zeroizing::new(format!(
            "{}{}",
            kind.prefix(),
            meshway_protocol::local::credential().map_err(|_| AccountError::Worker)?
        ))))
    }

    pub(super) fn digest(value: &str, kind: SecretKind) -> Result<Vec<u8>> {
        let Some(random) = value.strip_prefix(kind.prefix()) else {
            return Err(AccountError::Unauthorized);
        };
        if !meshway_protocol::local::valid_credential(random) {
            return Err(AccountError::Unauthorized);
        }
        Ok(Sha256::digest(value.as_bytes()).to_vec())
    }
}

pub(crate) struct IssuedSecret {
    pub(crate) secret: Secret,
    pub(crate) expires_at: i64,
}

pub(super) enum PrincipalKind {
    Bootstrap,
    Session {
        digest: Vec<u8>,
        user: UserId,
        version: i64,
    },
    Node,
}

/// Authentication proof, with private construction; session validity is rechecked
/// in each write transaction (including logout/reset between requests).
pub(crate) struct Principal(pub(super) PrincipalKind);

impl Principal {
    pub(crate) fn prove_bootstrap(store: &crate::store::Store, bearer: &str) -> Result<Self> {
        store
            .authorize_admin(bearer)
            .map_err(|_| AccountError::Unauthorized)?;
        Ok(Self(PrincipalKind::Bootstrap))
    }

    /// A node credential can never authorize human account operations. The
    /// existing node authenticator may use this marker after its own validation.
    pub(crate) fn node() -> Self {
        Self(PrincipalKind::Node)
    }
}
