use std::path::PathBuf;
use std::time::Duration;

/// Explicit application storage location and bounded SQLite work budget.
///
/// The data directory must be private to the current user. Its parent must
/// already exist. No main-workspace or machine-wide location is inferred.
pub struct Settings {
    pub data_dir: PathBuf,
    pub database_queue_capacity: usize,
    pub database_busy_timeout: Duration,
}

impl Settings {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            database_queue_capacity: 64,
            database_busy_timeout: Duration::from_secs(5),
        }
    }
}
