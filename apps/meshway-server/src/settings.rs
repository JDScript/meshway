use std::path::PathBuf;
use std::time::Duration;

/// Explicit application storage location and bounded SQLite work budget.
///
/// The data directory must be private to the current user. Its parent must
/// already exist. Ancestors must belong to root or the current effective user;
/// group/other-writable ancestors require sticky protection. Root and the
/// current UID are trusted to preserve this namespace during SQLite access.
/// No main-workspace or machine-wide location is inferred.
pub struct Settings {
    pub data_dir: PathBuf,
    pub database_queue_capacity: usize,
    pub database_busy_timeout: Duration,
    pub registration_enabled: bool,
    /// Whole seconds, from one minute through thirty days.
    pub session_ttl: Duration,
}

impl Settings {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            database_queue_capacity: 64,
            database_busy_timeout: Duration::from_secs(5),
            registration_enabled: true,
            session_ttl: Duration::from_secs(24 * 60 * 60),
        }
    }
}
