//! Authenticated network use cases and their transaction inputs.
//!
//! Membership is checked with the caller's current session for every operation.
//! Configuration remains a versioned value, never a second set of entity fields.

use chrono::{DateTime, Utc};
use meshway_model::{MembershipId, MembershipRole, NetworkId, Revision, UserId, Version};

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
    #[error("invalid or expired pagination cursor")]
    InvalidCursor,
    #[error("page limit must be between 1 and 200")]
    InvalidLimit,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkView {
    pub id: NetworkId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub current_revision: Revision,
    pub membership_id: MembershipId,
    pub role: MembershipRole,
    pub actions: NetworkActions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkActions {
    pub rename: bool,
    pub edit_configuration: bool,
    pub manage_members: bool,
}

impl NetworkActions {
    pub(crate) fn for_role(role: MembershipRole) -> Self {
        let manage = matches!(role, MembershipRole::Owner | MembershipRole::Admin);
        Self {
            rename: manage,
            edit_configuration: manage,
            manage_members: manage,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberView {
    pub id: MembershipId,
    pub user_id: UserId,
    pub email: String,
    pub role: MembershipRole,
    pub version: Version,
    pub joined_at: DateTime<Utc>,
}

pub(crate) fn normalize_name(value: &str) -> Result<String, NetworkError> {
    if value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(NetworkError::InvalidName);
    }
    let value = value.trim();
    if !(1..=128).contains(&value.chars().count()) {
        return Err(NetworkError::InvalidName);
    }
    Ok(value.to_owned())
}
