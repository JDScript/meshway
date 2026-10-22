//! How the Server and its offline commands stop on an error: one message for
//! the log and the exit status a service manager acts on.
use std::process::ExitCode;

use crate::AppError;

pub struct Failure {
    pub message: String,
    pub code: ExitCode,
}

impl Failure {
    /// A failed storage opening: persistent blocks exit with
    /// [`crate::PERSISTENT_STORAGE_BLOCK`].
    pub fn open(error: AppError) -> Self {
        Self {
            code: error.exit_code(),
            message: error.open_failure(),
        }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            message,
            code: ExitCode::FAILURE,
        }
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
