//! Local installation resources. Their lifetime is deliberately independent of
//! an execution lease: kept routing may reject queries, never authorize them.
use crate::DnsFuture;
use meshway_configuration::{DnsConfiguration, DnsTransport, netmap::DnsPlan};
pub use meshway_model::exit::ExitDnsUnavailableReason;
use meshway_protocol::ipc::LocalDnsDomain;
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

/// Which ownership check a system DNS operation stopped at, what it expected
/// and found, and where it read that: one record answers which invariant
/// failed and what broke it. It never carries a secret (the installation
/// marker is shortened) and grants nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemDnsFailure {
    pub error: SystemDnsError,
    /// Stable snake_case name of the check, listed in the runtime reference.
    pub check: &'static str,
    pub expected: Option<String>,
    pub actual: Option<String>,
    /// The file, D-Bus property/method or netlink dump the check read.
    pub source: Option<String>,
}
impl SystemDnsFailure {
    pub fn new(error: SystemDnsError, check: &'static str) -> Self {
        Self {
            error,
            check,
            expected: None,
            actual: None,
            source: None,
        }
    }
    pub fn expected(mut self, value: impl std::fmt::Display) -> Self {
        self.expected = Some(bounded(value.to_string()));
        self
    }
    pub fn actual(mut self, value: impl std::fmt::Display) -> Self {
        self.actual = Some(bounded(value.to_string()));
        self
    }
    pub fn source(mut self, value: impl std::fmt::Display) -> Self {
        self.source = Some(bounded(value.to_string()));
        self
    }
    /// The owner's judgement of this failure: whether the check read the
    /// system back and found it different from what this owner installed
    /// (the "cannot recover by itself" column of the runtime reference: the
    /// anchor deleted or changed, link settings written by someone else, an
    /// interrupted settings change, a reservation of another owner, another
    /// namespace). Anything else (a failed or abandoned read, resolved
    /// restarting, a transient foreign domain route, the end of kernel
    /// ownership that the Host re-proves) proves nothing.
    pub fn proven(&self) -> bool {
        self.error == SystemDnsError::Conflict
            && match self.check {
                "namespace" | "owner_record" | "link_settings" | "anchor_ended" => true,
                "anchor_netlink" | "anchor_reader" | "anchor_create" => false,
                check if check.starts_with("anchor_") => true,
                check if check.starts_with("record_") => {
                    !matches!(check, "record_read" | "record_write")
                }
                _ => false,
            }
    }
    /// Whether the backend's installation probe found the system DNS
    /// unable to serve this backend (`probe`, `Unsupported`): the owner's
    /// re-proof does not run the probe, so it holds and the next install
    /// fails the same way. Neither the timeout nor a version proves the host
    /// unsupported for good, so it stays unknown; its owner retries it with
    /// a growing pause (`Recovery::observe_repeated`).
    pub fn repeats(&self) -> bool {
        self.error == SystemDnsError::Unsupported && self.check == "probe"
    }
    /// Whether the failure is the system's (an installation, a read or the
    /// backend), which the owner recovers from; otherwise the netmap's plan
    /// or the local settings were refused, and only another netmap changes
    /// that.
    pub fn system(&self) -> bool {
        match self.error {
            SystemDnsError::Settings | SystemDnsError::ExitPlan | SystemDnsError::ResolverPlan => {
                false
            }
            // A backend that reports an unsupported mode names its check; an
            // unsupported configuration has none.
            SystemDnsError::Unsupported => self.check != "unrecorded",
            _ => true,
        }
    }
}
/// One log line and one Status field: keep each part bounded.
fn bounded(mut value: String) -> String {
    const LIMIT: usize = 512;
    if value.len() > LIMIT {
        let mut end = LIMIT;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
        value.push('…');
    }
    value
}
impl std::fmt::Display for SystemDnsFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DNS {} check failed ({})", self.check, self.error)?;
        if let Some(expected) = &self.expected {
            write!(f, ": expected {expected}")?;
        }
        if let Some(actual) = &self.actual {
            write!(
                f,
                "{} found {actual}",
                if self.expected.is_some() { ";" } else { ":" }
            )?;
        }
        if let Some(source) = &self.source {
            write!(f, " [{source}]")?;
        }
        Ok(())
    }
}
/// Attach the failed check to a plain error.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) trait At<T> {
    fn at(self, check: &'static str) -> Result<T, SystemDnsFailure>;
}
impl<T> At<T> for Result<T, SystemDnsError> {
    fn at(self, check: &'static str) -> Result<T, SystemDnsFailure> {
        self.map_err(|error| SystemDnsFailure::new(error, check))
    }
}

/// A host adapter must verify the kernel-owned link before AND after external
/// waits. Public metadata is not a grant and cannot substitute for that adapter.
/// A failed or interrupted verification proves nothing either way: it fails
/// this call only. The link ends (`ended`) only when its owner removes it or
/// the Kernel's ownership ends.
pub trait OwnedDnsLink: Send + Sync + 'static {
    fn verify(&self) -> DnsFuture<'_, Result<DnsLinkIdentity, SystemDnsFailure>>;
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
    /// How a routing domain of another link (a search domain or `~domain`)
    /// meets this plan under resolved's most specific match. The plan's
    /// explicit domains (MagicDNS, split routes, search domains) and its
    /// fallback `~.` are judged apart: the fallback yields every specific
    /// domain to the link that routes it.
    pub fn foreign_route(&self, foreign: &str) -> ForeignRoute {
        let foreign = foreign.trim_end_matches('.').to_ascii_lowercase();
        if foreign.is_empty() {
            return if self.root {
                ForeignRoute::Conflict
            } else {
                ForeignRoute::Unrelated
            };
        }
        let explicit = self.domains.keys().any(|own| {
            let own = own.trim_end_matches('.');
            !own.is_empty()
                && (foreign == own || foreign.strip_suffix(own).is_some_and(|s| s.ends_with('.')))
        });
        if explicit {
            ForeignRoute::Conflict
        } else if self.root {
            ForeignRoute::Local
        } else {
            ForeignRoute::Unrelated
        }
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
/// [`SystemDnsPlan::foreign_route`]: the one judgement of another link's
/// routing domain against an installed plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForeignRoute {
    /// It routes no name this plan covers.
    Unrelated,
    /// It routes names this plan covers only through its fallback `~.`:
    /// that link's resolver answers them, every other name still takes the
    /// fallback. Not a conflict; reported as a local DNS domain.
    Local,
    /// Equal to or more specific than one of the plan's explicit domains,
    /// or a second `~.` beside the plan's own.
    Conflict,
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

/// What an exit resolver resolves its sources' queries with, decided on the
/// installed netmap. `Ok(None)`: `dns.overrideLocal` makes the configured
/// default resolvers the upstream, with their own netmap routes and no OS
/// snapshot. Otherwise the observed served snapshot, while every address it
/// may reach has a local decision; the error is why sources get SERVFAIL.
/// The Node's own DNS installation never depends on this result.
pub fn exit_resolvers(
    desired: &meshway_engine::Desired,
    observed: Result<SystemResolverSnapshot, ExitDnsUnavailableReason>,
) -> Result<Option<SystemResolverSnapshot>, ExitDnsUnavailableReason> {
    if desired.netmap().projection.configuration.dns.override_local {
        return Ok(None);
    }
    let snapshot = observed?;
    crate::route::served_decisions(desired, &snapshot)?;
    Ok(Some(snapshot))
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
    pub fn served_resolvers(&self) -> Result<SystemResolverSnapshot, ExitDnsUnavailableReason> {
        Err(ExitDnsUnavailableReason::Unobserved)
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
    pub fn failure(&self) -> Option<&SystemDnsFailure> {
        None
    }
    pub fn local_domains(&self) -> &[LocalDnsDomain] {
        &[]
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
