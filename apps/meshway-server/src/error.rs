//! The one outcome type for every Server use case.
//!
//! Variants are HTTP-independent categories; `http::error` is the only place
//! that maps them to wire Problem codes. Infrastructure variants carry a
//! non-secret cause so a failure stays diagnosable without exposing SQL text,
//! paths or credentials. Converting one of these errors must never change its
//! category: a storage failure is not an invalid session.

use meshway_configuration::{ConfigurationError, ConfigurationIssue, SerializationError};
use meshway_policy::compile::CompileError;

use crate::accounts::PasswordRules;
use crate::secrets::SealError;
use crate::store::ExecutorError;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    // Authentication
    #[error("invalid or expired session")]
    InvalidSession,
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("Node identity is no longer eligible")]
    NodeIdentityRejected,
    #[error("the Server holds no netmap for this authenticated Node")]
    NetmapWithdrawn,

    // Authorization
    #[error("permission denied")]
    PermissionDenied,
    #[error("registration is disabled")]
    RegistrationDisabled,

    // Lookup
    #[error("resource not found")]
    NotFound,

    // Concurrency and lifecycle conflicts
    #[error("configuration revision changed")]
    RevisionConflict,
    #[error("request ID was already used with different input")]
    RequestConflict,
    #[error("name changed since it was read")]
    NameConflict,
    #[error("version changed since it was read")]
    VersionConflict,
    #[error("request conflicts with the current generation or binding")]
    BindingConflict,
    #[error("receipt instance is no longer current")]
    ReceiptInstanceMismatch,
    #[error("receipt target was never held by this node")]
    ReceiptTargetMismatch,
    #[error("applied receipt names a superseded netmap")]
    ReceiptNetmapSuperseded,
    #[error("receipt timing or saved prerequisite is not satisfied")]
    ReceiptPrecondition,
    #[error("receipt content conflicts with its immutable sequence")]
    ReceiptContentConflict,
    #[error("lifecycle state does not allow this operation")]
    StateConflict,
    #[error("state report execution scope changed")]
    StateScopeChanged,
    #[error("the last Owner cannot be removed or demoted")]
    LastOwner,
    #[error("already a network member")]
    AlreadyMember,

    // Invalid input
    #[error("invalid request")]
    InvalidRequest,
    #[error("invalid email")]
    InvalidEmail,
    #[error("password does not meet the published rules")]
    InvalidPassword(PasswordRules),
    #[error("current password is incorrect")]
    InvalidCurrentPassword,
    #[error("email is already registered")]
    EmailTaken,
    #[error("name must contain 1 through 128 Unicode code points")]
    InvalidName,
    #[error("page limit must be between 1 and 200")]
    InvalidLimit,
    #[error("pagination cursor is invalid or expired")]
    InvalidCursor,
    #[error("configuration input is invalid")]
    InvalidConfiguration(ConfigurationError),

    // Password recovery credential lifecycle
    #[error("invalid or consumed recovery credential")]
    InvalidReset,
    /// A delegation code is unknown, expired, consumed, or does not match
    /// its client, redirect target or PKCE verifier.
    #[error("delegation code is invalid or no longer usable")]
    InvalidDelegation,
    #[error("recovery credential has already been used")]
    ResetUsed,
    #[error("recovery credential has expired")]
    ResetExpired,
    #[error("recovery credential has been superseded")]
    ResetReplaced,

    // Join link lifecycle
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

    // Budgets
    #[error("computation budget is busy")]
    Busy,
    #[error("state resource budget exceeded")]
    StateLimit,
    #[error("observation budget exceeded")]
    ObservationLimit,

    // Infrastructure
    #[error("a required dependency is unavailable")]
    DependencyUnavailable,
    #[error("service is unavailable: {0}")]
    Unavailable(Unavailability),
    #[error("storage failed: {0}")]
    Storage(StorageCause),
    #[error("internal invariant violated: {0}")]
    Internal(&'static str),
}

/// Why the service could not serve an otherwise valid request right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Unavailability {
    #[error("service is shutting down")]
    Closed,
    #[error("in-memory runtime is unavailable")]
    Runtime,
    #[error("operating system entropy is unavailable")]
    Entropy,
    #[error("the Node's netmap is unavailable")]
    Netmap,
}

/// Why a storage operation failed. Labels name a table/column or operation,
/// never a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StorageCause {
    #[error("sqlite extended code {code}")]
    Sqlite { code: i32 },
    #[error("row conversion failed")]
    Row,
    #[error("stored value could not be decoded: {0}")]
    Decode(&'static str),
    #[error("stored value could not be encoded: {0}")]
    Encode(&'static str),
    #[error("private storage cryptography failed")]
    Seal,
    #[error("database worker is unavailable")]
    Executor,
}

impl Error {
    pub const CLOSED: Self = Self::Unavailable(Unavailability::Closed);
    pub const RUNTIME: Self = Self::Unavailable(Unavailability::Runtime);
    pub const ENTROPY: Self = Self::Unavailable(Unavailability::Entropy);

    /// A persisted value failed to parse into its domain type.
    pub const fn decode(label: &'static str) -> Self {
        Self::Storage(StorageCause::Decode(label))
    }
    /// A domain value could not be serialized for persistence.
    pub const fn encode(label: &'static str) -> Self {
        Self::Storage(StorageCause::Encode(label))
    }
    pub fn invalid_configuration(issues: Vec<ConfigurationIssue>) -> Self {
        Self::InvalidConfiguration(ConfigurationError::Invalid(issues))
    }
    pub const fn is_storage(&self) -> bool {
        matches!(self, Self::Storage(_))
    }
    pub const fn is_unavailable(&self) -> bool {
        matches!(
            self,
            Self::Storage(_)
                | Self::Unavailable(_)
                | Self::DependencyUnavailable
                | Self::Internal(_)
        )
    }
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        match error {
            rusqlite::Error::SqliteFailure(failure, _) => Self::Storage(StorageCause::Sqlite {
                code: failure.extended_code,
            }),
            rusqlite::Error::FromSqlConversionFailure(..)
            | rusqlite::Error::IntegralValueOutOfRange(..)
            | rusqlite::Error::InvalidColumnIndex(_)
            | rusqlite::Error::InvalidColumnName(_)
            | rusqlite::Error::InvalidColumnType(..)
            | rusqlite::Error::InvalidQuery
            | rusqlite::Error::QueryReturnedNoRows
            | rusqlite::Error::QueryReturnedMoreThanOneRow => Self::Storage(StorageCause::Row),
            _ => Self::Storage(StorageCause::Sqlite { code: -1 }),
        }
    }
}

impl From<ExecutorError> for Error {
    fn from(error: ExecutorError) -> Self {
        match error {
            ExecutorError::Closed => Self::CLOSED,
            _ => Self::Storage(StorageCause::Executor),
        }
    }
}

impl From<SealError> for Error {
    fn from(_: SealError) -> Self {
        Self::Storage(StorageCause::Seal)
    }
}

impl From<ConfigurationError> for Error {
    fn from(error: ConfigurationError) -> Self {
        Self::InvalidConfiguration(error)
    }
}

impl From<CompileError> for Error {
    fn from(error: CompileError) -> Self {
        match error {
            CompileError::Budget => Self::Busy,
            // Demand fields are part of the compiler input, so an inconsistent
            // input may still originate from the requesting Node.
            CompileError::InvalidInput => Self::InvalidRequest,
            CompileError::EdgeTransportUnavailable { rule, a, b } => {
                Self::InvalidConfiguration(ConfigurationError::Invalid(vec![
                    meshway_configuration::ConfigurationIssue::new(
                        format!("/topology/{rule}/transports"),
                        meshway_configuration::IssueCode::InvalidTransport,
                    )
                    .with("reason", "edge_transport_unavailable")
                    .with("a", a)
                    .with("b", b),
                ]))
            }
            CompileError::Material | CompileError::UnavailableDependency => {
                Self::DependencyUnavailable
            }
            CompileError::Serialization => Self::encode("compiled netmap"),
        }
    }
}

impl From<SerializationError> for Error {
    fn from(_: SerializationError) -> Self {
        Self::encode("compiled netmap")
    }
}
