//! Authenticated network use cases and their transaction inputs.
//!
//! Membership is checked with the caller's current session for every operation.
//! Configuration remains a versioned value, never a second set of entity fields.

use chrono::{DateTime, Utc};
pub use meshway_model::{MemberSummary, NetworkSummary};
use meshway_model::{NetworkId, RequestId};
use std::sync::Arc;

pub(crate) mod configuration;
pub(crate) mod management;
pub(crate) mod pagination;

use crate::{
    accounts::token_digest,
    secrets::Sealer,
    store::{Handle, networks as store},
};

#[derive(Clone)]
pub struct Networks {
    handle: Handle,
    sealer: Arc<Sealer>,
    configuration_cache: Arc<crate::configuration::SnapshotCache>,
    publisher: crate::state::service::CommitPublisher,
}
pub(crate) struct NameChange {
    pub(crate) name: String,
    pub(crate) expected: String,
    pub(crate) request: RequestId,
}

impl Networks {
    pub(crate) fn new(
        handle: Handle,
        sealer: Arc<Sealer>,
        configuration_cache: Arc<crate::configuration::SnapshotCache>,
        publisher: crate::state::service::CommitPublisher,
    ) -> Self {
        Self {
            handle,
            sealer,
            configuration_cache,
            publisher,
        }
    }

    pub async fn create(
        &self,
        token: &str,
        name: String,
        request_id: RequestId,
    ) -> Result<NetworkSummary, NetworkError> {
        let digest = token_digest(token)?;
        let name = normalize_name(&name)?;
        // Cheap authentication before template preparation; the creating
        // transaction still checks the current session again after admission.
        self.handle
            .execute(move |connection| store::authenticate(connection, digest))
            .await??;
        let prepared = configuration::prepare(name, &self.sealer)?;
        let sealer = Arc::clone(&self.sealer);
        let cache = Arc::clone(&self.configuration_cache);
        self.handle
            .execute(move |connection| {
                store::create(connection, digest, prepared, request_id, &sealer, &cache)
            })
            .await?
    }

    pub async fn list(
        &self,
        token: &str,
        limit: u16,
        cursor: Option<String>,
    ) -> Result<NetworkPage, NetworkError> {
        pagination::validate_limit(limit)?;
        let digest = token_digest(token)?;
        let sealer = Arc::clone(&self.sealer);
        let cache = Arc::clone(&self.configuration_cache);
        self.handle
            .execute(move |connection| {
                store::list(connection, digest, limit, cursor, &sealer, &cache)
            })
            .await?
    }

    pub async fn get(
        &self,
        token: &str,
        network_id: NetworkId,
    ) -> Result<NetworkSummary, NetworkError> {
        let digest = token_digest(token)?;
        let sealer = Arc::clone(&self.sealer);
        let cache = Arc::clone(&self.configuration_cache);
        self.handle
            .execute(move |connection| store::get(connection, digest, network_id, &sealer, &cache))
            .await?
    }

    pub async fn rename(
        &self,
        token: &str,
        network_id: NetworkId,
        name: String,
        expected_name: String,
        request_id: RequestId,
    ) -> Result<NetworkSummary, NetworkError> {
        let digest = token_digest(token)?;
        let name = normalize_name(&name)?;
        // Expected metadata must be the exact previously returned spelling.
        if normalize_name(&expected_name)? != expected_name {
            return Err(NetworkError::InvalidName);
        }
        let sealer = Arc::clone(&self.sealer);
        let cache = Arc::clone(&self.configuration_cache);
        let publisher = self.publisher.clone();
        let changed = name != expected_name;
        self.handle
            .execute(move |connection| {
                let result = store::rename(
                    connection,
                    digest,
                    network_id,
                    NameChange {
                        name,
                        expected: expected_name,
                        request: request_id,
                    },
                    &sealer,
                    &cache,
                )?;
                if changed {
                    publisher.publish_committed(
                        crate::state::stream::CommittedChange::Authorization { network_id },
                    );
                }
                Ok(result)
            })
            .await?
    }

    pub async fn members(
        &self,
        token: &str,
        network_id: NetworkId,
        limit: u16,
        cursor: Option<String>,
    ) -> Result<MemberPage, NetworkError> {
        pagination::validate_limit(limit)?;
        let digest = token_digest(token)?;
        let sealer = Arc::clone(&self.sealer);
        let cache = Arc::clone(&self.configuration_cache);
        self.handle
            .execute(move |connection| {
                store::members(
                    connection, digest, network_id, limit, cursor, &sealer, &cache,
                )
            })
            .await?
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum NetworkError {
    #[error("invalid or expired session")]
    InvalidSession,
    #[error("network is not available to this user")]
    NotFound,
    #[error("this network operation requires administrator permission")]
    PermissionDenied,
    #[error("network name must contain 1 through 128 Unicode code points")]
    InvalidName,
    #[error("request ID was already used with different input")]
    RequestConflict,
    #[error("network name changed since it was read")]
    NameConflict,
    #[error("invalid or expired pagination cursor")]
    InvalidCursor,
    #[error("page limit must be between 1 and 200")]
    InvalidLimit,
    #[error("member changed since it was read")]
    VersionConflict,
    #[error("configuration revision changed")]
    RevisionConflict,
    #[error("the last Owner cannot be removed or demoted")]
    LastOwner,
    #[error("already a network member")]
    AlreadyMember,
    #[error("invalid or no longer usable join link")]
    InvalidLink,
    #[error("join link has expired")]
    LinkExpired,
    #[error("join link has been used")]
    LinkUsed,
    #[error("join link was revoked")]
    LinkRevoked,
    #[error("join link issuer is no longer authorized")]
    LinkIssuerUnavailable,
    #[error("network storage operation failed")]
    Storage,
    #[error("network service is unavailable")]
    Unavailable,
}

impl From<rusqlite::Error> for NetworkError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

impl From<crate::store::ExecutorError> for NetworkError {
    fn from(_: crate::store::ExecutorError) -> Self {
        Self::Unavailable
    }
}

impl From<crate::accounts::AccountError> for NetworkError {
    fn from(error: crate::accounts::AccountError) -> Self {
        match error {
            crate::accounts::AccountError::InvalidSession => Self::InvalidSession,
            crate::accounts::AccountError::Storage => Self::Storage,
            _ => Self::Unavailable,
        }
    }
}

/// Creation-bounded traversal, with current metadata and permissions on each
/// page. `as_of` identifies the first page's boundary, not a frozen SQL snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkPage {
    pub items: Vec<NetworkSummary>,
    pub next_cursor: Option<String>,
    pub as_of: DateTime<Utc>,
    pub total: meshway_model::Count,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberPage {
    pub items: Vec<MemberSummary>,
    pub next_cursor: Option<String>,
    pub as_of: DateTime<Utc>,
    pub total: meshway_model::Count,
}

pub(crate) fn normalize_name(value: &str) -> Result<String, NetworkError> {
    let rules = meshway_protocol::NetworkNameRules::CURRENT;
    if value.len() > usize::from(rules.max_input_utf8_bytes)
        || (rules.forbid_control_characters && value.chars().any(char::is_control))
    {
        return Err(NetworkError::InvalidName);
    }
    let value = value.trim();
    if !(usize::from(rules.min_codepoints)..=usize::from(rules.max_codepoints))
        .contains(&value.chars().count())
    {
        return Err(NetworkError::InvalidName);
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests;
