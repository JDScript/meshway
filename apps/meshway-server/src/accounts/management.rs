use super::{Accounts, Secret, email, password, secret};
use crate::error::Error;
use crate::store::accounts::management as store;
use chrono::{DateTime, Utc};
use meshway_model::{RequestId, SessionId};
use rusqlite::TransactionBehavior;

#[derive(Clone, Debug)]
pub struct AccountSession {
    pub id: SessionId,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub client_label: Option<String>,
    pub current: bool,
}

pub struct AccountSessionPage {
    pub items: Vec<AccountSession>,
    pub next_cursor: Option<String>,
    pub as_of: DateTime<Utc>,
}

#[derive(Clone, Copy)]
pub enum RevokeSessions {
    One(SessionId),
    Others,
    All,
}

#[derive(Clone, Debug)]
pub struct ResetInspection {
    pub expires_at: DateTime<Utc>,
}

pub struct IssuedReset {
    pub token: Secret,
    pub expires_at: DateTime<Utc>,
}

impl Accounts {
    pub(crate) async fn change_email(
        &self,
        token: &str,
        current: Secret,
        email: String,
        request: RequestId,
        configurations: crate::configuration::Configurations,
    ) -> Result<super::User, Error> {
        let digest = secret::token_digest(token)?;
        let email = email::normalize(&email)?;
        let candidate = self
            .handle
            .read(move |tx| store::candidate(tx, digest))
            .await?;
        password::validate(&current).map_err(|_| Error::InvalidCurrentPassword)?;
        let verified = password::verify(current, Some(candidate))
            .await
            .map_err(current_password_error)?;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::change_email(tx, digest, verified, email, request, &configurations)
            })
            .await
    }
    pub(crate) async fn sessions(
        &self,
        token: &str,
        limit: u16,
        cursor: Option<String>,
        sealer: std::sync::Arc<crate::secrets::Sealer>,
    ) -> Result<AccountSessionPage, Error> {
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidRequest);
        }
        let digest = secret::token_digest(token)?;
        self.handle
            .read(move |tx| store::sessions(tx, digest, limit, cursor, &sealer))
            .await
    }

    pub async fn revoke_sessions(
        &self,
        token: &str,
        target: RevokeSessions,
        request: RequestId,
    ) -> Result<(), Error> {
        let digest = secret::token_digest(token)?;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::revoke(tx, digest, target, request)
            })
            .await
    }

    pub async fn change_password(
        &self,
        token: &str,
        current: Secret,
        replacement: Secret,
        request: RequestId,
    ) -> Result<(), Error> {
        let digest = secret::token_digest(token)?;
        let candidate = self
            .handle
            .read(move |tx| store::candidate(tx, digest))
            .await?;
        password::validate(&current).map_err(|_| Error::InvalidCurrentPassword)?;
        password::validate(&replacement)?;
        let verified = password::verify(current, Some(candidate))
            .await
            .map_err(current_password_error)?;
        let hash = password::hash(replacement).await?;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::change_password(tx, digest, verified, hash, request)
            })
            .await
    }

    /// Host-maintenance entry point. No HTTP route exposes this operation.
    /// Uses the App's exclusive directory/SQLite ownership, including offline CLI use.
    pub async fn issue_password_reset(
        &self,
        email: String,
        expires_in_seconds: u32,
        request: RequestId,
    ) -> Result<IssuedReset, Error> {
        if !(60..=86400).contains(&expires_in_seconds) {
            return Err(Error::InvalidRequest);
        }
        let email = email::normalize(&email)?;
        self.handle
            .write(move |tx| store::issue_reset(tx, email, expires_in_seconds, request))
            .await
    }

    pub async fn inspect_password_reset(&self, token: &str) -> Result<ResetInspection, Error> {
        let digest = secret::one_time_digest("mwr_", token)?;
        self.handle
            .read(move |tx| store::inspect_reset(tx, digest))
            .await
    }

    pub async fn redeem_password_reset(
        &self,
        token: &str,
        password: Secret,
        request: RequestId,
    ) -> Result<(), Error> {
        let digest = secret::one_time_digest("mwr_", token)?;
        password::validate(&password)?;
        self.handle
            .read(move |tx| store::inspect_reset(tx, digest))
            .await?;
        let hash = password::hash(password).await?;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::redeem_reset(tx, digest, hash, request)
            })
            .await
    }
}

/// A failed verification of the caller's own current password is a distinct
/// outcome from a failed login.
fn current_password_error(error: Error) -> Error {
    match error {
        Error::InvalidCredentials => Error::InvalidCurrentPassword,
        other => other,
    }
}
