pub use reqwest::Url;
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
    /// Where browsers reach the Console. Links the Server hands to devices
    /// (device authorization) are built on it; without it they are null.
    pub console_origin: Option<Url>,
}

impl Settings {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            database_queue_capacity: 64,
            database_busy_timeout: Duration::from_secs(5),
            registration_enabled: true,
            session_ttl: Duration::from_secs(24 * 60 * 60),
            console_origin: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "expected an https Console origin (http is accepted on loopback only) without path, query or credentials"
)]
pub struct InvalidConsoleOrigin;

/// A Console origin is `scheme://host[:port]/`: HTTPS anywhere, plain HTTP on
/// loopback for development. The same rule applies to every command that
/// emits a browser link.
pub fn console_origin(value: &str) -> Result<Url, InvalidConsoleOrigin> {
    let origin = Url::parse(value).map_err(|_| InvalidConsoleOrigin)?;
    let loopback = origin.scheme() == "http"
        && matches!(origin.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(origin.scheme() == "https" || loopback)
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
        || origin.path() != "/"
    {
        return Err(InvalidConsoleOrigin);
    }
    Ok(origin)
}

#[cfg(test)]
mod tests {
    use super::console_origin;

    #[test]
    fn console_origins_are_secure_bare_origins() {
        for ok in [
            "https://console.example.com",
            "https://console.example.com:8443/",
            "http://localhost:5173",
            "http://127.0.0.1:8420/",
            "http://[::1]:8420",
        ] {
            assert_eq!(console_origin(ok).unwrap().path(), "/", "{ok}");
        }
        for bad in [
            "http://console.example.com",
            "https://user@console.example.com",
            "https://console.example.com/app",
            "https://console.example.com/?x=1",
            "https://console.example.com/#a",
            "console.example.com",
        ] {
            assert!(console_origin(bad).is_err(), "{bad}");
        }
    }
}
