//! Account use cases, independent of HTTP DTOs.
//!
//! Request IDs correlate successful activities, not credentials or registration
//! recovery keys. A lost registration response is recovered by password login.
//! Authentication results attest one transaction only: subsequent mutations must
//! recheck the current session and account in their own transaction.

mod email;
mod management;
mod password;
mod secret;
pub use management::{
    AccountSession, AccountSessionPage, IssuedReset, ResetInspection, RevokeSessions,
};

use std::time::Duration;

use chrono::{DateTime, Utc};
use meshway_model::{RequestId, SessionId, UserId};

use crate::store::{Handle, accounts as store};

#[cfg(test)]
pub(crate) use password::TEST_KDF_LOCK;
pub use secret::Secret;
pub(crate) use secret::{issue_one_time, issue_token, one_time_digest, random_uuid, token_digest};

/// Safe, fixed error categories; never contains a supplied value or SQL error.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccountError {
    #[error("registration is disabled")]
    RegistrationDisabled,
    #[error("invalid email")]
    InvalidEmail,
    #[error("password does not meet the published rules")]
    InvalidPassword,
    #[error("email is already registered")]
    EmailTaken,
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("current password is incorrect")]
    InvalidCurrentPassword,
    #[error("invalid or expired session")]
    InvalidSession,
    #[error("resource is not available")]
    NotFound,
    #[error("invalid or consumed recovery credential")]
    InvalidReset,
    #[error("recovery credential has already been used")]
    ResetUsed,
    #[error("recovery credential has expired")]
    ResetExpired,
    #[error("recovery credential has been superseded")]
    ResetReplaced,
    #[error("invalid request")]
    InvalidRequest,
    #[error("pagination cursor is invalid or expired")]
    InvalidCursor,
    #[error("account computation budget is busy")]
    Busy,
    #[error("account service is unavailable")]
    Unavailable,
    #[error("account storage operation failed")]
    Storage,
}

impl From<rusqlite::Error> for AccountError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

impl From<crate::store::ExecutorError> for AccountError {
    fn from(_: crate::store::ExecutorError) -> Self {
        Self::Unavailable
    }
}

/// Unicode scalar values (Rust `char`), not graphemes or UTF-8 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PasswordRules {
    pub min_codepoints: usize,
    pub max_codepoints: usize,
    pub max_utf8_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub registration_enabled: bool,
    pub password: PasswordRules,
    pub session_ttl: Duration,
}

impl Options {
    pub(crate) fn new(registration_enabled: bool, session_ttl: Duration) -> Option<Self> {
        if !(Duration::from_secs(60)..=Duration::from_secs(30 * 24 * 60 * 60))
            .contains(&session_ttl)
            || session_ttl.subsec_nanos() != 0
        {
            return None;
        }
        Some(Self {
            registration_enabled,
            password: password::RULES,
            session_ttl,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    /// Canonical login identifier; email ownership has not been verified.
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedSession {
    pub user: User,
    pub session_id: SessionId,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Deliberately neither Debug nor Clone: token delivery is explicit.
pub struct IssuedSession {
    pub session: AuthenticatedSession,
    pub token: Secret,
}

#[derive(Clone)]
pub struct Accounts {
    handle: Handle,
    options: Options,
    publisher: crate::state::service::CommitPublisher,
}

impl Accounts {
    pub(crate) fn new(
        handle: Handle,
        options: Options,
        publisher: crate::state::service::CommitPublisher,
    ) -> Self {
        Self {
            handle,
            options,
            publisher,
        }
    }

    pub fn options(&self) -> Options {
        self.options
    }

    pub async fn register(
        &self,
        email: String,
        password: Secret,
        request_id: RequestId,
    ) -> Result<IssuedSession, AccountError> {
        self.register_from_client(email, password, request_id, None)
            .await
    }

    pub(crate) async fn register_from_client(
        &self,
        email: String,
        password: Secret,
        request_id: RequestId,
        client_label: Option<String>,
    ) -> Result<IssuedSession, AccountError> {
        if !self.options.registration_enabled {
            return Err(AccountError::RegistrationDisabled);
        }
        let email = email::normalize(&email)?;
        password::validate(&password)?;
        let hash = password::hash(password).await?;
        let ttl = self.options.session_ttl;
        self.handle
            .execute(move |connection| {
                store::register(
                    connection,
                    email,
                    hash,
                    ttl,
                    request_id,
                    client_label.as_deref(),
                )
            })
            .await?
    }

    pub async fn login(
        &self,
        email: String,
        password: Secret,
        request_id: RequestId,
    ) -> Result<IssuedSession, AccountError> {
        self.login_from_client(email, password, request_id, None)
            .await
    }

    pub(crate) async fn login_from_client(
        &self,
        email: String,
        password: Secret,
        request_id: RequestId,
        client_label: Option<String>,
    ) -> Result<IssuedSession, AccountError> {
        // Invalid identifiers and unknown users still take the bounded dummy KDF
        // path; malformed/oversize passwords are rejected before computation.
        password::validate(&password).map_err(|_| AccountError::InvalidCredentials)?;
        let email = email::normalize(&email).ok();
        let candidate = self
            .handle
            .execute(move |connection| store::login_candidate(connection, email.as_deref()))
            .await??;
        let candidate = password::verify(password, candidate).await?;
        let ttl = self.options.session_ttl;
        self.handle
            .execute(move |connection| {
                store::login(
                    connection,
                    candidate,
                    ttl,
                    request_id,
                    client_label.as_deref(),
                )
            })
            .await?
    }

    pub async fn authenticate(&self, token: &str) -> Result<AuthenticatedSession, AccountError> {
        let digest = secret::token_digest(token)?;
        self.handle
            .execute(move |connection| store::authenticate(connection, digest))
            .await?
    }

    /// Invalid/expired/already revoked sessions return InvalidSession. Only the
    /// successfully authenticated current session is revoked.
    pub async fn logout(&self, token: &str, request_id: RequestId) -> Result<(), AccountError> {
        let digest = secret::token_digest(token)?;
        let publisher = self.publisher.clone();
        self.handle
            .execute(move |connection| store::logout(connection, digest, request_id, &publisher))
            .await?
    }
}
