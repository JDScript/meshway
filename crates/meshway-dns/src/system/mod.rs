//! Local installation resources. Their lifetime is deliberately independent of
//! an execution lease: kept routing may reject queries, never authorize them.
use crate::DnsFuture;
use meshway_configuration::{DnsConfiguration, DnsTransport, netmap::DnsPlan};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

/// Nullable private record envelopes use Serde's option visitor outside the
/// object-only wrapper. Applying MapOnly to Option itself rejects both its
/// object and null encodings rather than validating the contained record.
#[cfg(any(target_os = "linux", test))]
fn decode_optional_record<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<Option<T>, SystemDnsError> {
    struct Record<T>(T);
    impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for Record<T> {
        fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
            T::deserialize(meshway_model::wire::MapOnly(decoder)).map(Self)
        }
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let record =
        Option::<Record<T>>::deserialize(&mut decoder).map_err(|_| SystemDnsError::Storage)?;
    decoder.end().map_err(|_| SystemDnsError::Storage)?;
    Ok(record.map(|record| record.0))
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{SystemDnsOwner, SystemDnsReservation};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "backend", rename_all = "camelCase", deny_unknown_fields)]
pub enum SystemDnsBackend {
    Resolved,
    OwnedResolvConf { path: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SystemDnsSettings {
    pub bind: SocketAddr,
    pub backend: SystemDnsBackend,
    pub anchor_address: Ipv4Addr,
}
impl Default for SystemDnsSettings {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 5354)),
            backend: SystemDnsBackend::Resolved,
            anchor_address: Ipv4Addr::new(198, 18, 255, 254),
        }
    }
}
impl SystemDnsSettings {
    pub fn validate(&self) -> Result<(), SystemDnsError> {
        if !self.bind.ip().is_loopback()
            || self.bind.port() == 0
            || self.anchor_address.is_unspecified()
            || self.anchor_address.is_multicast()
            || self.anchor_address.is_loopback()
            || self.anchor_address.is_broadcast()
            || self.anchor_address.is_link_local()
        {
            return Err(SystemDnsError::Settings);
        }
        if let SystemDnsBackend::OwnedResolvConf { path } = &self.backend
            && (self.bind.port() != 53
                || !path.is_absolute()
                || path.components().any(|c| {
                    !matches!(
                        c,
                        std::path::Component::RootDir | std::path::Component::Normal(_)
                    )
                }))
        {
            return Err(SystemDnsError::Settings);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SystemDnsError {
    #[error("system DNS backend is unsupported")]
    Unsupported,
    #[error("invalid local DNS installation settings")]
    Settings,
    #[error("system DNS resource ownership conflict")]
    Conflict,
    #[error("system DNS storage operation failed or has an uncertain outcome")]
    Storage,
    #[error("system DNS operation failed")]
    Backend,
    #[error("system DNS capacity exceeded")]
    Capacity,
    #[error("DNS execution requires an unsupported exit plan")]
    ExitPlan,
    #[error("DNS resolver plan is incomplete")]
    ResolverPlan,
}

/// A host adapter must verify the kernel-owned link before AND after external
/// waits. Public metadata is not a grant and cannot substitute for that adapter.
pub trait OwnedDnsLink: Send + Sync + 'static {
    fn verify(&self) -> DnsFuture<'_, Result<DnsLinkIdentity, SystemDnsError>>;
    fn ended(&self) -> DnsFuture<'_, ()>;
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsLinkIdentity {
    pub boot: String,
    pub namespace_device: u64,
    pub namespace_inode: u64,
    pub installation: String,
    pub index: u32,
    pub name: String,
    pub address: Ipv4Addr,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemDnsPlan {
    /// Normalized domain -> route-only. Search entries are kept separately to
    /// preserve the Server's order when serializing the D-Bus array.
    domains: BTreeMap<String, bool>,
    search: Vec<String>,
    root: bool,
}
impl SystemDnsPlan {
    pub fn new(settings: &DnsConfiguration, egress: &DnsPlan) -> Result<Self, SystemDnsError> {
        if matches!(egress, DnsPlan::Unavailable { .. }) {
            return Err(SystemDnsError::ExitPlan);
        }
        Self::required(settings, egress)
    }

    /// Required OS capture even when execution is unavailable. An unsupported
    /// exit plan captures root negatively rather than treating it as NoExit.
    /// This plan grants no query execution; callers keep Scope=None until every
    /// real route and supported execution check has completed.
    pub fn required(settings: &DnsConfiguration, egress: &DnsPlan) -> Result<Self, SystemDnsError> {
        let mut domains = BTreeMap::new();
        let normalize = |s: &str| {
            meshway_configuration::normalize_dns_name(s).map_err(|_| SystemDnsError::Settings)
        };
        for suffix in settings.routes.keys() {
            domains.insert(normalize(suffix)?, true);
        }
        let mut search = Vec::new();
        if settings.magic_dns {
            let domain = normalize(&settings.domain)?;
            domains.insert(domain.clone(), false);
            search.push(domain);
        }
        for suffix in &settings.search_domains {
            let suffix = normalize(suffix)?;
            if !search.contains(&suffix) {
                search.push(suffix.clone());
            }
            domains.insert(suffix, false);
        }
        let root = settings.override_local
            || matches!(
                egress,
                DnsPlan::ViaExit { .. } | DnsPlan::Unavailable { .. }
            );
        if root {
            domains.insert(".".into(), true);
        }
        if domains.len() > 256 {
            return Err(SystemDnsError::Capacity);
        }
        let plan = Self {
            domains,
            search,
            root,
        };
        plan.validate()?;
        Ok(plan)
    }
    pub fn domains(&self) -> Vec<(String, bool)> {
        self.search
            .iter()
            .map(|s| (s.clone(), false))
            .chain(
                self.domains
                    .iter()
                    .filter(|(_, only)| **only)
                    .map(|(s, _)| (s.clone(), true)),
            )
            .collect()
    }
    fn validate(&self) -> Result<(), SystemDnsError> {
        if self.domains.len() > 256
            || self.search.len() > self.domains.len()
            || self.root != (self.domains.get(".") == Some(&true))
            || self.search.iter().enumerate().any(|(i, name)| {
                self.search[..i].contains(name) || self.domains.get(name) != Some(&false)
            })
            || self.domains.iter().any(|(name, only)| {
                (name == "." && !only)
                    || (name != "."
                        && meshway_configuration::normalize_dns_name(name).as_ref() != Ok(name))
                    || (!only && !self.search.contains(name))
            })
        {
            return Err(SystemDnsError::Settings);
        }
        Ok(())
    }
    pub fn conflicts(&self, foreign: &str) -> bool {
        let foreign = foreign.trim_end_matches('.').to_ascii_lowercase();
        self.domains.keys().any(|own| {
            let own = own.trim_end_matches('.');
            own.is_empty()
                || foreign == own
                || foreign.strip_suffix(own).is_some_and(|s| s.ends_with('.'))
        })
    }
    pub fn needs_system_fallback(&self, settings: &DnsConfiguration) -> bool {
        // Search domains also route DNS in resolved. Only suffixes whose entire
        // namespace is already handled locally/configured need no fallback.
        !self.root
            && self.search.iter().any(|s| {
                !(settings.magic_dns && suffix_of(&settings.domain, s))
                    && !settings.routes.keys().any(|route| suffix_of(route, s))
            })
    }
}
fn suffix_of(parent: &str, child: &str) -> bool {
    let parent = parent.trim_end_matches('.');
    let child = child.trim_end_matches('.');
    parent.eq_ignore_ascii_case(child)
        || child
            .to_ascii_lowercase()
            .strip_suffix(&parent.to_ascii_lowercase())
            .is_some_and(|s| s.ends_with('.'))
}

/// Where a system resolver snapshot's upstreams come from.
#[derive(Clone, Debug)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) enum Provenance {
    /// The original OS nameservers the owner recorded before installing.
    Original,
    /// resolved's proxy stub. It forwards by resolved's own per-link and
    /// per-suffix routing to `servers` (every OS server except the owned
    /// link's), but names under `routes` reach the owned link, i.e. this
    /// Node's own DNS owner. Only an exit resolver's served queries use it.
    ResolvedProxy {
        routes: Arc<Vec<String>>,
        servers: Arc<Vec<std::net::IpAddr>>,
    },
}

/// Only a checked system installer constructs this provenance. Its endpoints
/// still require the netmap's local-egress decision before any business I/O.
#[derive(Clone)]
pub struct SystemResolverSnapshot {
    pub(crate) id: String,
    pub(crate) upstreams: Arc<Vec<SocketAddr>>,
    pub(crate) primary: DnsTransport,
    pub(crate) provenance: Provenance,
    pub(crate) invalid: CancellationToken,
    pub(crate) verified_until: Arc<Mutex<Instant>>,
}
impl SystemResolverSnapshot {
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Whether resolving `name` through these upstreams would come back to
    /// this Node's own DNS link.
    pub(crate) fn routes_to_owner(&self, name: &str) -> bool {
        let Provenance::ResolvedProxy { routes, .. } = &self.provenance else {
            return false;
        };
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        routes.iter().any(|route| {
            let route = route.trim_end_matches('.');
            route.is_empty()
                || name == route
                || name
                    .strip_suffix(route)
                    .is_some_and(|rest| rest.ends_with('.'))
        })
    }
    /// The same observation, not merely equal endpoints.
    pub(crate) fn same(&self, other: &Self) -> bool {
        self.id == other.id && Arc::ptr_eq(&self.upstreams, &other.upstreams)
    }
    pub(crate) fn ensure_current(&self) -> Result<(), SystemDnsError> {
        if self.invalid.is_cancelled()
            || self
                .verified_until
                .lock()
                .map_or(true, |at| Instant::now() >= *at)
        {
            self.invalid.cancel();
            Err(SystemDnsError::Conflict)
        } else {
            Ok(())
        }
    }
    pub(crate) async fn ended(&self) {
        loop {
            let deadline = match self.verified_until.lock() {
                Ok(at) => *at,
                Err(_) => return,
            };
            if self.ensure_current().is_err() {
                return;
            }
            tokio::select! { biased;
                _ = self.invalid.cancelled() => return,
                _ = tokio::time::sleep_until(deadline.into()) => {},
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub struct SystemDnsOwner;
#[cfg(not(target_os = "linux"))]
impl SystemDnsOwner {
    pub async fn prepare(
        _: SystemDnsSettings,
        _: Arc<meshway_private_storage::Directory>,
        _: Option<Arc<dyn OwnedDnsLink>>,
    ) -> Result<Self, SystemDnsError> {
        Err(SystemDnsError::Unsupported)
    }
    pub fn system_resolvers(&self) -> Option<SystemResolverSnapshot> {
        None
    }
    pub fn served_resolvers(&self) -> Option<SystemResolverSnapshot> {
        None
    }
    pub async fn install(&mut self, _: &SystemDnsPlan) -> Result<(), SystemDnsError> {
        Err(SystemDnsError::Unsupported)
    }
    pub async fn observe(&mut self) -> Result<(), SystemDnsError> {
        Err(SystemDnsError::Unsupported)
    }
    pub async fn recover(&mut self) -> Result<(), SystemDnsError> {
        Err(SystemDnsError::Unsupported)
    }
    pub async fn down(&mut self) -> Result<(), SystemDnsError> {
        Err(SystemDnsError::Unsupported)
    }
    pub async fn shutdown(&mut self, _: bool) -> Result<(), SystemDnsError> {
        Err(SystemDnsError::Unsupported)
    }
}

#[cfg(test)]
mod tests;
