use meshway_transport_spec::Endpoint;
pub use reqwest::Url;
use std::path::PathBuf;
use std::time::Duration;
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
};

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
    /// Browser clients allowed to obtain delegated sessions and to call the
    /// Console API cross-origin. Empty: no delegation, no CORS.
    pub delegation_clients: crate::accounts::delegation::DelegationClients,
    /// The origin at which clients reach this Server, handed to delegation
    /// clients with their session; null when the deployment has not set it.
    pub public_origin: Option<Url>,
    /// Explicit UDP service listeners. Empty means no hosted STUN socket.
    pub stun_listen: Vec<SocketAddr>,
    /// Explicit hosted/external discovery endpoints; never inferred from HTTP.
    pub stun_servers: BTreeMap<String, Endpoint>,
    /// Netmap lease validity (5 minutes through 24 hours) and nominal sync
    /// hold (1 through 54 seconds, jittered ±10%).
    pub netmap: crate::netmap::SyncTiming,
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
            delegation_clients: Default::default(),
            public_origin: None,
            stun_listen: Vec::new(),
            stun_servers: BTreeMap::new(),
            netmap: crate::netmap::SyncTiming::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("expected a unique STUN server key and an explicit unicast HOST:PORT")]
pub struct InvalidStunServer;

pub fn stun_server(value: &str) -> Result<(String, Endpoint), InvalidStunServer> {
    let (key, endpoint) = value.split_once('=').ok_or(InvalidStunServer)?;
    if key.is_empty()
        || key.len() > 64
        || !key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err(InvalidStunServer);
    }
    let url = Url::parse(&format!("stun://{endpoint}")).map_err(|_| InvalidStunServer)?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
        || endpoint.ends_with('/')
    {
        return Err(InvalidStunServer);
    }
    let host = url
        .host_str()
        .ok_or(InvalidStunServer)?
        .trim_start_matches('[')
        .trim_end_matches(']');
    if host.is_empty()
        || host.len() > 253
        || host.parse::<IpAddr>().is_ok_and(|ip| {
            ip.is_unspecified()
                || ip.is_multicast()
                || matches!(ip, IpAddr::V4(ip) if ip.is_broadcast())
        })
    {
        return Err(InvalidStunServer);
    }
    Ok((
        key.to_owned(),
        Endpoint {
            host: host.to_owned(),
            port: url
                .port()
                .ok_or(InvalidStunServer)?
                .try_into()
                .map_err(|_| InvalidStunServer)?,
        },
    ))
}

pub(crate) fn validate_stun(settings: &Settings) -> Result<(), crate::AppError> {
    if settings.stun_listen.len() > 4 || settings.stun_servers.len() > 4 {
        return Err(crate::AppError::InvalidSettings);
    }
    let mut listeners = std::collections::BTreeSet::new();
    for address in &settings.stun_listen {
        if address.port() == 0
            || address.ip().is_multicast()
            || matches!(address.ip(), IpAddr::V4(ip) if ip.is_broadcast())
            || !listeners.insert(*address)
        {
            return Err(crate::AppError::InvalidSettings);
        }
    }
    for (key, endpoint) in &settings.stun_servers {
        let host = if endpoint.host.contains(':') {
            format!("[{}]", endpoint.host)
        } else {
            endpoint.host.clone()
        };
        let (_, normalized) = stun_server(&format!("{key}={host}:{}", endpoint.port.get()))
            .map_err(|_| crate::AppError::InvalidSettings)?;
        if normalized != *endpoint {
            return Err(crate::AppError::InvalidSettings);
        }
    }
    Ok(())
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
    fn stun_metadata_is_explicit_bounded_and_never_infers_a_wildcard_advertisement() {
        use super::*;
        let mut settings = Settings::new("/unused");
        assert!(settings.stun_listen.is_empty());
        assert!(settings.stun_servers.is_empty());
        assert!(validate_stun(&settings).is_ok());
        for bad in [
            "a=0.0.0.0:3478",
            "a=[::]:3478",
            "a=224.0.0.1:3478",
            "a=stun.example",
            "a=stun.example:0",
            "a=host:3478/path",
            "a=host:3478?x=1",
            "a=user@host:3478",
            "a b=host:3478",
            "é=host:3478",
        ] {
            assert!(stun_server(bad).is_err(), "{bad}");
        }
        let (id, endpoint) = stun_server("operator_a-1=stun.example.test:3478").unwrap();
        settings.stun_servers.insert(id, endpoint);
        settings.stun_listen.push("0.0.0.0:3478".parse().unwrap());
        assert!(validate_stun(&settings).is_ok());
        settings.stun_listen.push("0.0.0.0:3478".parse().unwrap());
        assert!(validate_stun(&settings).is_err());
        settings.stun_listen.clear();
        for number in 0..4 {
            let (id, endpoint) =
                stun_server(&format!("server{number}=stun.example.test:3478")).unwrap();
            settings.stun_servers.insert(id, endpoint);
        }
        assert!(validate_stun(&settings).is_err());
    }

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
