use axum::http::StatusCode;
use meshway_protocol::local;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid or missing bearer credential")]
    Unauthorized,
    #[error("node credential has been revoked")]
    Forbidden,
    #[error("{0}")]
    Invalid(&'static str),
    #[error("configuration revision/digest conflict or credential already bound")]
    Conflict,
    #[error("{0}")]
    Unprocessable(&'static str),
    #[error("{0}")]
    Capacity(&'static str),
    #[error("resource not found in the selected network")]
    NotFound,
    #[error("{0}")]
    Configuration(&'static str),
    #[error("controller storage error")]
    Storage(#[from] rusqlite::Error),
    #[error("controller file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Write(#[from] local::AtomicWriteError),
    #[error("controller storage worker unavailable")]
    Worker,
    #[error("{1}")]
    Body(StatusCode, &'static str),
}

impl From<crate::accounts::AccountError> for Error {
    fn from(value: crate::accounts::AccountError) -> Self {
        use crate::accounts::AccountError;
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
