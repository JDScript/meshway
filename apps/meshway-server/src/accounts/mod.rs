//! Account use cases, independent of HTTP DTOs.
//!
//! Request IDs correlate successful activities, not credentials or registration
//! recovery keys. A lost registration response is recovered by password login.
//! Authentication results attest one transaction only: subsequent mutations must
//! recheck the current session and account in their own transaction.

pub mod delegation;
pub use crate::store::accounts::delegation::Authorization as DelegationAuthorization;
mod email;
mod management;
mod password;
mod secret;
mod throttle;
pub use management::{
    AccountSession, AccountSessionPage, IssuedReset, ResetInspection, RevokeSessions,
};

use crate::error::Error;
use std::time::Duration;

use chrono::{DateTime, Utc};
use meshway_model::{RequestId, SessionId, UserId};

use crate::store::{Handle, accounts as store};
use rusqlite::TransactionBehavior;

#[cfg(test)]
pub(crate) use password::TEST_KDF_LOCK;
pub use secret::Secret;
pub(crate) use secret::{issue_one_time, issue_token, one_time_digest, random_uuid, token_digest};

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
    /// The delegation client this session was issued to, if any. Delegated
    /// sessions act as the user but cannot manage the account, other
    /// sessions or further delegations.
    pub delegation_client: Option<String>,
}

impl AuthenticatedSession {
    pub fn delegated(&self) -> bool {
        self.delegation_client.is_some()
    }
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
    delegation: std::sync::Arc<delegation::DelegationClients>,
    /// Failed-login backoff, shared by every handle of one App.
    throttle: throttle::Throttle,
}

/// A one-time authorization code for a delegation client; the Console appends
/// it (and the client's own `state`) to the redirect target itself.
pub struct IssuedCode {
    pub code: Secret,
    pub expires_at: DateTime<Utc>,
}

/// What a delegation client receives for an exchanged code.
pub struct DelegatedSession {
    pub issued: IssuedSession,
    pub client: String,
}

impl Accounts {
    pub(crate) fn new(
        handle: Handle,
        options: Options,
        delegation: std::sync::Arc<delegation::DelegationClients>,
    ) -> Self {
        Self {
            handle,
            options,
            delegation,
            throttle: throttle::Throttle::default(),
        }
    }

    pub fn options(&self) -> Options {
        self.options
    }

    pub fn delegation_clients(&self) -> &delegation::DelegationClients {
        &self.delegation
    }

    /// A signed-in, non-delegated user authorizes a configured client: the
    /// result is the browser location carrying a one-time code and the
    /// caller's `state`.
    pub async fn authorize_delegation(
        &self,
        token: &str,
        authorization: store::delegation::Authorization,
        request_id: RequestId,
    ) -> Result<IssuedCode, Error> {
        let digest = secret::token_digest(token)?;
        let clients = self.delegation.clone();
        self.handle
            .write(move |tx| {
                store::delegation::authorize(tx, digest, &clients, &authorization, request_id)
            })
            .await
    }

    /// Exchanges a code and its PKCE verifier for a delegated session; the
    /// code is consumed whether or not the exchange succeeds.
    pub async fn exchange_delegation(
        &self,
        client: String,
        code: Secret,
        code_verifier: Secret,
        redirect_uri: String,
        request_id: RequestId,
    ) -> Result<DelegatedSession, Error> {
        let clients = self.delegation.clone();
        self.handle
            .write(move |tx| {
                store::delegation::exchange(
                    tx,
                    &clients,
                    &client,
                    code.expose(),
                    code_verifier.expose(),
                    &redirect_uri,
                    request_id,
                )
            })
            .await
            .and_then(|outcome| match outcome {
                store::delegation::Exchange::Issued(issued, client) => {
                    Ok(DelegatedSession { issued, client })
                }
                store::delegation::Exchange::Rejected => Err(Error::InvalidDelegation),
            })
    }

    pub async fn register(
        &self,
        email: String,
        password: Secret,
        request_id: RequestId,
    ) -> Result<IssuedSession, Error> {
        self.register_from_client(email, password, request_id, None)
            .await
    }

    pub(crate) async fn register_from_client(
        &self,
        email: String,
        password: Secret,
        request_id: RequestId,
        client_label: Option<String>,
    ) -> Result<IssuedSession, Error> {
        if !self.options.registration_enabled {
            return Err(Error::RegistrationDisabled);
        }
        let email = email::normalize(&email)?;
        password::validate(&password)?;
        let hash = password::hash(password).await?;
        let ttl = self.options.session_ttl;
        self.handle
            .write(move |tx| {
                store::register(tx, email, hash, ttl, request_id, client_label.as_deref())
            })
            .await
    }

    pub async fn login(
        &self,
        email: String,
        password: Secret,
        request_id: RequestId,
    ) -> Result<IssuedSession, Error> {
        self.login_from_client(email, password, request_id, None, None)
            .await
    }

    /// `source` is the direct peer address (no forwarded header); failed
    /// logins of one account from one source back off (`throttle`), and an
    /// attempt while closed is `Busy` without computing the password hash.
    pub(crate) async fn login_from_client(
        &self,
        email: String,
        password: Secret,
        request_id: RequestId,
        client_label: Option<String>,
        source: Option<std::net::IpAddr>,
    ) -> Result<IssuedSession, Error> {
        let attempt = self.throttle.admit(throttle::Key::new(&email, source))?;
        let result = self
            .login_attempt(email, password, request_id, client_label)
            .await;
        match &result {
            Ok(_) => attempt.finish(false),
            Err(Error::InvalidCredentials) => attempt.finish(true),
            // Busy, storage and the rest say nothing about the credentials.
            Err(_) => drop(attempt),
        }
        result
    }

    async fn login_attempt(
        &self,
        email: String,
        password: Secret,
        request_id: RequestId,
        client_label: Option<String>,
    ) -> Result<IssuedSession, Error> {
        // Invalid identifiers and unknown users still take the bounded dummy KDF
        // path; malformed/oversize passwords are rejected before computation.
        password::validate(&password).map_err(|_| Error::InvalidCredentials)?;
        let email = email::normalize(&email).ok();
        let candidate = self
            .handle
            .read(move |tx| store::login_candidate(tx, email.as_deref()))
            .await?;
        let candidate = password::verify(password, candidate).await?;
        let ttl = self.options.session_ttl;
        self.handle
            .write(move |tx| store::login(tx, candidate, ttl, request_id, client_label.as_deref()))
            .await
    }

    pub async fn authenticate(&self, token: &str) -> Result<AuthenticatedSession, Error> {
        let digest = secret::token_digest(token)?;
        self.handle
            .read(move |tx| store::authenticate(tx, digest))
            .await
    }

    /// Invalid/expired/already revoked sessions return InvalidSession. Only the
    /// successfully authenticated current session is revoked.
    pub async fn logout(&self, token: &str, request_id: RequestId) -> Result<(), Error> {
        let digest = secret::token_digest(token)?;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::logout(tx, digest, request_id)
            })
            .await
    }
}
