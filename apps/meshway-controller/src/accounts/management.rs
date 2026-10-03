use super::{AccountError, Accounts, Secret, email, password, secret};
use crate::store::accounts::management as store;
use chrono::{DateTime, Utc};
use meshway_model::{RequestId, SessionId};

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
    ) -> Result<super::User, AccountError> {
        let digest = secret::token_digest(token)?;
        let email = email::normalize(&email)?;
        password::validate(&current).map_err(|_| AccountError::InvalidCredentials)?;
        let candidate = self
            .handle
            .execute(move |db| store::candidate(db, digest))
            .await??;
        let verified = password::verify(current, Some(candidate)).await?;
        self.handle
            .execute(move |db| {
                store::change_email(db, digest, verified, email, request, &configurations)
            })
            .await?
    }
    pub(crate) async fn sessions(
        &self,
        token: &str,
        limit: u16,
        cursor: Option<String>,
        sealer: std::sync::Arc<crate::secrets::Sealer>,
    ) -> Result<AccountSessionPage, AccountError> {
        if !(1..=200).contains(&limit) {
            return Err(AccountError::InvalidRequest);
        }
        let digest = secret::token_digest(token)?;
        self.handle
            .execute(move |db| store::sessions(db, digest, limit, cursor, &sealer))
            .await?
    }

    pub async fn revoke_sessions(
        &self,
        token: &str,
        target: RevokeSessions,
        request: RequestId,
    ) -> Result<(), AccountError> {
        let digest = secret::token_digest(token)?;
        self.handle
            .execute(move |db| store::revoke(db, digest, target, request))
            .await?
    }

    pub async fn change_password(
        &self,
        token: &str,
        current: Secret,
        replacement: Secret,
        request: RequestId,
    ) -> Result<(), AccountError> {
        let digest = secret::token_digest(token)?;
        password::validate(&current).map_err(|_| AccountError::InvalidCredentials)?;
        password::validate(&replacement)?;
        let candidate = self
            .handle
            .execute(move |db| store::candidate(db, digest))
            .await??;
        let verified = password::verify(current, Some(candidate)).await?;
        let hash = password::hash(replacement).await?;
        self.handle
            .execute(move |db| store::change_password(db, digest, verified, hash, request))
            .await?
    }

    /// Host-maintenance entry point. No HTTP route exposes this operation.
    /// Uses the App's exclusive directory/SQLite ownership, including offline CLI use.
    pub async fn issue_password_reset(
        &self,
        email: String,
        expires_in_seconds: u32,
        request: RequestId,
    ) -> Result<IssuedReset, AccountError> {
        if !(60..=86400).contains(&expires_in_seconds) {
            return Err(AccountError::InvalidRequest);
        }
        let email = email::normalize(&email)?;
        self.handle
            .execute(move |db| store::issue_reset(db, email, expires_in_seconds, request))
            .await?
    }

    pub async fn inspect_password_reset(
        &self,
        token: &str,
    ) -> Result<ResetInspection, AccountError> {
        let digest = secret::one_time_digest("mwr_", token)?;
        self.handle
            .execute(move |db| store::inspect_reset(db, digest))
            .await?
    }

    pub async fn redeem_password_reset(
        &self,
        token: &str,
        password: Secret,
        request: RequestId,
    ) -> Result<(), AccountError> {
        let digest = secret::one_time_digest("mwr_", token)?;
        password::validate(&password)?;
        self.handle
            .execute(move |db| store::inspect_reset(db, digest))
            .await??;
        let hash = password::hash(password).await?;
        self.handle
            .execute(move |db| store::redeem_reset(db, digest, hash, request))
            .await?
    }
}
