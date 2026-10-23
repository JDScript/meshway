//! Per-owned-link resolve1 integration. No global settings or ambient resolver
//! fallback are changed. A nonce UDP->TCP probe verifies local link DNS behavior.
//!
//! Ownership is proven by the namespace, the Kernel-owned anchor link and the
//! exact installed link settings, which carry a random installation marker. The
//! resolved process (its D-Bus unique name) is not part of the proof: every
//! operation re-checks that the current name owner runs in this namespace, so
//! a restarted resolved that kept the settings is adopted, and one that lost
//! them leaves the link at its original settings for a fresh installation.
use super::{
    DnsLinkIdentity, ExitDnsUnavailableReason, ForeignRoute, LocalDnsDomain, Namespace,
    OwnedDnsLink, SystemDnsError, SystemDnsFailure, SystemDnsPlan, storage,
};
use crate::system::At;
use meshway_private_storage::Directory;
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use zbus::zvariant::{DynamicDeserialize, DynamicType};

mod bus;
mod probe;
#[cfg(test)]
mod tests;
use bus::{attach, property, raw, read_settings, verify_os_resolver, verify_owner};
use probe::serve_probe;

const PATH: &str = "/org/freedesktop/resolve1";
const IFACE: &str = "org.freedesktop.resolve1.Manager";
const LINK: &str = "org.freedesktop.resolve1.Link";
const SERVICE: &str = "org.freedesktop.resolve1";
const TIMEOUT: Duration = Duration::from_secs(5);
const VERSION: u32 = 2;
const MARKER_SUFFIX: &str = "meshway-installation.invalid";
const RECORD: &str = "resolved.json";
/// The systemd-resolved releases that deliver a query to a link's loopback
/// DNS server (systemd commit 5f9041af, see [`Backend::probe`]).
const LOOPBACK_SERVER_RELEASES: &str =
    "systemd-resolved 253 or later, or stable 249.15, 250.10, 251.11, 252.5 or later";
type DnsServer = (i32, Vec<u8>, u16, String);
type GlobalDomains = Vec<(i32, String, bool)>;
/// resolved's manager-wide server lists: (ifindex, family, address, port, name).
type GlobalServers = Vec<(i32, i32, Vec<u8>, u16, String)>;
type Checked<T> = Result<T, SystemDnsFailure>;
/// resolved's proxy stub (systemd ≥ 248, present with `DNSStubListener=yes`):
/// queries are forwarded by resolved's per-link routing without local
/// synthesis, LLMNR or mDNS. Exit resolvers serve sources through it.
pub(super) const PROXY_STUB: SocketAddr =
    SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 54)), 53);

/// What resolved's proxy stub currently forwards to.
pub(super) struct Proxy {
    /// Names under these domains reach the owned link (this Node's owner).
    pub routes: Vec<String>,
    /// Every OS server outside the owned link, global and fallback included.
    pub servers: Vec<IpAddr>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkSettings {
    dns: Vec<DnsServer>,
    domains: Vec<(String, bool)>,
    default_route: bool,
    llmnr: String,
    mdns: String,
    tls: String,
    dnssec: String,
}
impl LinkSettings {
    /// The fields where `self` (expected) and `actual` differ, each side
    /// rendered as resolvectl shows it.
    fn differences(&self, actual: &Self) -> (String, String) {
        let mut expected = Vec::new();
        let mut found = Vec::new();
        let mut field = |name: &str, same: bool, e: String, a: String| {
            if !same {
                expected.push(format!("{name}={e}"));
                found.push(format!("{name}={a}"));
            }
        };
        let servers = |dns: &[DnsServer]| {
            let shown: Vec<_> = dns
                .iter()
                .map(|(family, bytes, port, _)| {
                    let ip = match (family, bytes.as_slice()) {
                        (2, b) => <[u8; 4]>::try_from(b).ok().map(IpAddr::from),
                        (10, b) => <[u8; 16]>::try_from(b).ok().map(IpAddr::from),
                        _ => None,
                    };
                    match ip {
                        Some(ip) if *port != 0 => SocketAddr::new(ip, *port).to_string(),
                        Some(ip) => ip.to_string(),
                        None => format!("family {family} ({} bytes)", bytes.len()),
                    }
                })
                .collect();
            format!("[{}]", shown.join(", "))
        };
        let domains = |domains: &[(String, bool)]| {
            let shown: Vec<_> = domains
                .iter()
                .map(|(name, route)| format!("{}{}", if *route { "~" } else { "" }, shown(name)))
                .collect();
            format!("[{}]", shown.join(", "))
        };
        field(
            "dns",
            self.dns == actual.dns,
            servers(&self.dns),
            servers(&actual.dns),
        );
        field(
            "domains",
            self.domains == actual.domains,
            domains(&self.domains),
            domains(&actual.domains),
        );
        field(
            "defaultRoute",
            self.default_route == actual.default_route,
            self.default_route.to_string(),
            actual.default_route.to_string(),
        );
        for (name, e, a) in [
            ("llmnr", &self.llmnr, &actual.llmnr),
            ("mdns", &self.mdns, &actual.mdns),
            ("dnsOverTls", &self.tls, &actual.tls),
            ("dnssec", &self.dnssec, &actual.dnssec),
        ] {
            field(name, e == a, e.clone(), a.clone());
        }
        (expected.join(" "), found.join(" "))
    }
}
/// A domain as logged: the random installation marker shortened.
fn shown(name: &str) -> String {
    match name.strip_suffix(MARKER_SUFFIX) {
        Some(marker) if marker.len() > 10 => format!("{}….{MARKER_SUFFIX}", &marker[..9]),
        _ => name.to_owned(),
    }
}
fn link_identity(identity: &DnsLinkIdentity) -> String {
    format!(
        "{} (index {}, {}, netns {}:{})",
        identity.name,
        identity.index,
        identity.address,
        identity.namespace_device,
        identity.namespace_inode
    )
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    /// Version 1 pinned the resolved process; read for upgrade only.
    #[serde(default, skip_serializing)]
    service_owner: Option<String>,
    /// Random route-only marker domain inside every installed setting.
    /// Absent in version 1 records until their next installation.
    #[serde(default)]
    installation: Option<String>,
    link: DnsLinkIdentity,
    bind: SocketAddr,
    original: LinkSettings,
    installed: Option<LinkSettings>,
    plan: Option<SystemDnsPlan>,
    pending: bool,
}
pub(super) struct Backend {
    directory: Arc<Directory>,
    connection: zbus::Connection,
    service_owner: String,
    namespace: Namespace,
    anchor: Arc<dyn OwnedDnsLink>,
    path: String,
    record: Record,
    writes: storage::Writes,
    probed: bool,
    /// The last proxy stub check: resolved's owner, when, and its result.
    stub: Option<(String, std::time::Instant, bool)>,
    /// Other links' domains that coexist with the installed plan's
    /// fallback, as the last successful observation read them.
    local: Vec<LocalDnsDomain>,
}
pub(super) fn has_record(directory: &Directory) -> Result<bool, SystemDnsError> {
    storage::read::<Record>(directory, RECORD).map(|record| record.is_some())
}
impl Backend {
    fn record_source(&self) -> String {
        self.directory.path().join(RECORD).display().to_string()
    }
    fn link_source(&self) -> String {
        format!("D-Bus {} {} {LINK}", self.service_owner, self.path)
    }
    pub async fn prepare(
        directory: Arc<Directory>,
        namespace: Namespace,
        bind: SocketAddr,
        anchor: Arc<dyn OwnedDnsLink>,
    ) -> Checked<Self> {
        let identity = anchor.verify().await?;
        if identity.namespace_device != namespace.device
            || identity.namespace_inode != namespace.inode
            || identity.boot.trim() != namespace.boot
            || identity.index == 0
            || identity.index > i32::MAX as u32
        {
            return Err(
                SystemDnsFailure::new(SystemDnsError::Conflict, "anchor_namespace")
                    .expected(format!(
                        "anchor in netns {}:{} of boot {}",
                        namespace.device, namespace.inode, namespace.boot
                    ))
                    .actual(link_identity(&identity))
                    .source("Kernel DNS anchor record"),
            );
        }
        let connection = tokio::time::timeout(TIMEOUT, zbus::Connection::system())
            .await
            .map_err(|_| "no connection within 5 s".to_owned())
            .and_then(|connection| connection.map_err(|error| error.to_string()))
            .map_err(|actual| {
                SystemDnsFailure::new(SystemDnsError::Unsupported, "system_bus")
                    .actual(actual)
                    .source("D-Bus system bus")
            })?;
        let (service_owner, path) = attach(&connection, &namespace, identity.index).await?;
        let actual = read_settings(&connection, &service_owner, &path).await?;
        let again = anchor.verify().await?;
        if again != identity {
            return Err(
                SystemDnsFailure::new(SystemDnsError::Conflict, "anchor_identity")
                    .expected(link_identity(&identity))
                    .actual(link_identity(&again))
                    .source("Kernel DNS anchor"),
            );
        }
        let dir = directory.clone();
        let previous: Option<Record> =
            tokio::task::spawn_blocking(move || storage::read(&dir, RECORD))
                .await
                .map_err(|_| SystemDnsError::Storage)
                .and_then(|read| read)
                .at("record_read")
                .map_err(|failure| failure.source(directory.path().join(RECORD).display()))?;
        let source = directory.path().join(RECORD).display().to_string();
        let link_source = format!("D-Bus {service_owner} {path} {LINK}");
        let record_failure = |check, expected: &str, actual: String| {
            SystemDnsFailure::new(SystemDnsError::Conflict, check)
                .expected(expected)
                .actual(actual)
                .source(&source)
        };
        let mut lost = false;
        let record = match previous {
            Some(mut record) => {
                if !record.original.dns.is_empty() || !record.original.domains.is_empty() {
                    let (_, found) = local_settings(bind, Vec::new()).differences(&record.original);
                    return Err(record_failure(
                        "record_original",
                        "original link settings without DNS servers or domains",
                        found,
                    ));
                }
                match (&record.installed, &record.plan) {
                    (Some(installed), Some(plan)) => {
                        plan.validate().at("record_plan")?;
                        let (expected, found) = record.settings(plan).differences(installed);
                        if !expected.is_empty() {
                            return Err(record_failure(
                                "record_plan",
                                &format!("installed settings of the recorded plan: {expected}"),
                                found,
                            ));
                        }
                    }
                    (None, None) => {}
                    (installed, plan) => {
                        return Err(record_failure(
                            "record_plan",
                            "installed settings and plan both present or both absent",
                            format!(
                                "installed {}, plan {}",
                                if installed.is_some() {
                                    "present"
                                } else {
                                    "absent"
                                },
                                if plan.is_some() { "present" } else { "absent" }
                            ),
                        ));
                    }
                }
                let valid = match record.version {
                    1 => record.service_owner.is_some() && record.installation.is_none(),
                    VERSION => record.service_owner.is_none() && record.installation.is_some(),
                    _ => false,
                };
                if !valid {
                    return Err(record_failure(
                        "record_version",
                        "version 1 with a service owner or version 2 with a marker",
                        format!("version {}", record.version),
                    ));
                }
                if record.pending {
                    return Err(record_failure(
                        "record_pending",
                        "no interrupted settings change",
                        "a recorded settings change that never completed".to_owned(),
                    ));
                }
                if record.link != identity {
                    return Err(record_failure(
                        "record_link",
                        &link_identity(&record.link),
                        format!("anchor {}", link_identity(&identity)),
                    ));
                }
                if record.bind != bind {
                    return Err(record_failure(
                        "record_bind",
                        &record.bind.to_string(),
                        bind.to_string(),
                    ));
                }
                if record.installed.as_ref().unwrap_or(&record.original) != &actual {
                    // Only the exact original settings mean resolved dropped
                    // this link's runtime configuration (a restart without its
                    // saved link state). Any other value is a foreign writer.
                    if record.installed.is_none() || actual != record.original {
                        let (expected, found) = record
                            .installed
                            .as_ref()
                            .unwrap_or(&record.original)
                            .differences(&actual);
                        return Err(SystemDnsFailure::new(
                            SystemDnsError::Conflict,
                            "link_settings",
                        )
                        .expected(expected)
                        .actual(found)
                        .source(&link_source));
                    }
                    record.installed = None;
                    record.plan = None;
                    lost = true;
                }
                if record.installation.is_none() {
                    record.installation = Some(marker().at("record_marker")?);
                }
                record.version = VERSION;
                record.service_owner = None;
                record
            }
            None => {
                if !actual.dns.is_empty() || !actual.domains.is_empty() {
                    let (_, found) = local_settings(bind, Vec::new()).differences(&actual);
                    return Err(
                        SystemDnsFailure::new(SystemDnsError::Conflict, "link_settings")
                            .expected("a fresh anchor without DNS servers or domains")
                            .actual(found)
                            .source(&link_source),
                    );
                }
                Record {
                    version: VERSION,
                    service_owner: None,
                    installation: Some(marker().at("record_marker")?),
                    link: identity,
                    bind,
                    original: actual,
                    installed: None,
                    plan: None,
                    pending: false,
                }
            }
        };
        let mut backend = Self {
            directory,
            connection,
            service_owner,
            namespace,
            anchor,
            path,
            record,
            writes: storage::Writes::default(),
            probed: false,
            stub: None,
            local: Vec::new(),
        };
        if lost {
            // The link is at its original settings: record exactly that, so
            // the next install probes this resolved and installs afresh.
            tracing::info!(
                link = backend.record.link.name,
                "resolved dropped the owned link's settings; installing afresh"
            );
            backend.persist().await?;
        }
        backend.observe().await?;
        Ok(backend)
    }
    async fn verify(&self) -> Checked<()> {
        verify_owner(&self.connection, &self.service_owner, &self.namespace).await?;
        verify_os_resolver(&self.connection, &self.service_owner).await?;
        let identity = self.anchor.verify().await?;
        if identity != self.record.link {
            return Err(
                SystemDnsFailure::new(SystemDnsError::Conflict, "anchor_identity")
                    .expected(link_identity(&self.record.link))
                    .actual(link_identity(&identity))
                    .source("Kernel DNS anchor"),
            );
        }
        Ok(())
    }
    async fn call<B, R>(&self, method: &'static str, body: &B) -> Checked<R>
    where
        B: Serialize + DynamicType,
        R: for<'d> DynamicDeserialize<'d>,
    {
        self.verify().await?;
        let result = tokio::select! { biased;
            // Ended mid-call proves nothing by itself (kernel ownership may
            // have ended unavailable); the anchor's own check judges it.
            _ = self.anchor.ended() => {
                return Err(SystemDnsFailure::new(SystemDnsError::Backend, "anchor_ended")
                    .expected("the Kernel's DNS anchor")
                    .actual(format!("anchor ended during {method}"))
                    .source("Kernel DNS anchor"));
            }
            result = raw(&self.connection, &self.service_owner, PATH, IFACE, method, body, "resolved_call") => result,
        };
        self.verify().await?;
        result
    }
    pub async fn settle(&mut self) -> Result<(), SystemDnsError> {
        self.writes.settle().await
    }
    pub fn anchor(&self) -> Arc<dyn OwnedDnsLink> {
        self.anchor.clone()
    }
    async fn persist(&mut self) -> Checked<()> {
        let source = self.record_source();
        self.writes
            .save(self.directory.clone(), RECORD, &Some(&self.record))
            .await
            .at("record_write")
            .map_err(|failure| failure.source(source))
    }
    /// Other links' routing domains against `plan`
    /// ([`SystemDnsPlan::foreign_route`]): a conflict fails; the specific
    /// domains that coexist with the plan's fallback are returned, sorted.
    async fn conflicts(&self, plan: &SystemDnsPlan) -> Checked<Vec<(u32, String)>> {
        let source = format!("D-Bus {} {PATH} {IFACE}.Domains", self.service_owner);
        let domains: GlobalDomains = property(
            &self.connection,
            &self.service_owner,
            PATH,
            IFACE,
            "Domains",
            "foreign_domain",
        )
        .await?;
        if domains.len() > 4096 {
            return Err(
                SystemDnsFailure::new(SystemDnsError::Capacity, "foreign_domain")
                    .expected("at most 4096 routing domains")
                    .actual(domains.len())
                    .source(source),
            );
        }
        let mut local = std::collections::BTreeSet::new();
        for (index, name, route) in &domains {
            if *index == self.record.link.index as i32 {
                continue;
            }
            match plan.foreign_route(name) {
                ForeignRoute::Unrelated => {}
                ForeignRoute::Local => {
                    local.insert((
                        u32::try_from(*index).unwrap_or(0),
                        name.trim_end_matches('.').to_ascii_lowercase(),
                    ));
                }
                ForeignRoute::Conflict => {
                    return Err(SystemDnsFailure::new(
                        SystemDnsError::Conflict,
                        "foreign_domain",
                    )
                    .expected(format!(
                        "no link other than {} routing a domain this plan routes explicitly, nor a second ~.",
                        self.record.link.index
                    ))
                    .actual(format!(
                        "link {index} has {}{}",
                        if *route { "~" } else { "" },
                        shown(name)
                    ))
                    .source(source));
                }
            }
        }
        Ok(local.into_iter().collect())
    }
    /// The coexisting domains of the last successful observation, with the
    /// links' names; logged once whenever they change.
    fn record_local(&mut self, local: Vec<(u32, String)>) {
        let same = self.local.len() == local.len()
            && self
                .local
                .iter()
                .zip(&local)
                .all(|(known, (index, domain))| {
                    known.link_index == *index && known.domain == *domain
                });
        if same {
            return;
        }
        let names = link_names(local.iter().map(|(index, _)| *index));
        self.local = local
            .into_iter()
            .map(|(link_index, domain)| LocalDnsDomain {
                link_name: names.get(&link_index).cloned(),
                domain,
                link_index,
            })
            .collect();
        if !self.local.is_empty() {
            let shown: Vec<_> = self
                .local
                .iter()
                .map(|local| match &local.link_name {
                    Some(name) => format!("{} ({name})", local.domain),
                    None => format!("{} (link {})", local.domain, local.link_index),
                })
                .collect();
            tracing::info!(
                domains = shown.join(", "),
                "Other links resolve these domains; every other name takes this Node's DNS fallback"
            );
        }
    }
    pub fn local_domains(&self) -> &[LocalDnsDomain] {
        &self.local
    }
    pub async fn observe(&mut self) -> Checked<()> {
        let source = self.record_source();
        self.settle()
            .await
            .at("record_write")
            .map_err(|failure| failure.source(&source))?;
        if self.record.pending {
            return Err(
                SystemDnsFailure::new(SystemDnsError::Conflict, "record_pending")
                    .expected("no interrupted settings change")
                    .actual("a recorded settings change that never completed")
                    .source(source),
            );
        }
        let (owner, path) =
            attach(&self.connection, &self.namespace, self.record.link.index).await?;
        if owner != self.service_owner {
            // A restarted resolved is adopted only if it kept these exact
            // settings; the comparison below is the ownership proof.
            tracing::info!(
                previous = self.service_owner,
                current = owner,
                "resolved has a new bus owner; re-proving the owned link"
            );
            self.service_owner = owner;
            self.path = path;
        }
        self.verify().await?;
        let actual = read_settings(&self.connection, &self.service_owner, &self.path).await?;
        let wanted = self
            .record
            .installed
            .as_ref()
            .unwrap_or(&self.record.original);
        if actual != *wanted {
            let (expected, found) = wanted.differences(&actual);
            return Err(
                SystemDnsFailure::new(SystemDnsError::Conflict, "link_settings")
                    .expected(expected)
                    .actual(found)
                    .source(self.link_source()),
            );
        }
        let local = match &self.record.plan {
            Some(plan) => self.conflicts(plan).await?,
            None => Vec::new(),
        };
        self.verify().await?;
        self.record_local(local);
        Ok(())
    }
    /// The proxy stub's routing while settings are installed, or why it
    /// cannot serve: no installed link or unreadable inventory, no OS
    /// server, an OS server that is a local stub (loopback or unspecified),
    /// a multicast address or this Node's anchor/bind, or no proxy stub
    /// listening. An unknown local stub may recurse into Meshway, and no
    /// public resolver substitutes for the OS's own. The netmap's decision
    /// on each server (Mesh, exit-routed) is [`crate::exit_resolvers`]'s.
    pub async fn proxy(&mut self) -> Result<Proxy, ExitDnsUnavailableReason> {
        let Some(installed) = &self.record.installed else {
            return Err(ExitDnsUnavailableReason::Unobserved);
        };
        let mut own_seen = false;
        let mut servers = Vec::new();
        for list in ["DNSEx", "FallbackDNSEx"] {
            let entries: GlobalServers = property(
                &self.connection,
                &self.service_owner,
                PATH,
                IFACE,
                list,
                "proxy_servers",
            )
            .await
            .map_err(|_| ExitDnsUnavailableReason::Unobserved)?;
            if entries.len() > 256 {
                return Err(ExitDnsUnavailableReason::Unobserved);
            }
            for (_, family, address, port, _) in entries {
                let ip = match (family, address.as_slice()) {
                    (2, bytes) => <[u8; 4]>::try_from(bytes).ok().map(IpAddr::from),
                    (10, bytes) => <[u8; 16]>::try_from(bytes).ok().map(IpAddr::from),
                    _ => None,
                }
                .ok_or(ExitDnsUnavailableReason::LocalServer)?
                .to_canonical();
                // resolved reports a loopback server under the loopback
                // index, not its link's: the owned link's one server is the
                // bind, and any second entry for it is a foreign link.
                if SocketAddr::new(ip, if port == 0 { 53 } else { port }) == self.record.bind
                    && !own_seen
                {
                    own_seen = true;
                    continue;
                }
                if ip == IpAddr::V4(self.record.link.address) {
                    return Err(ExitDnsUnavailableReason::AnchorServer);
                }
                if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
                    return Err(ExitDnsUnavailableReason::LocalServer);
                }
                if !servers.contains(&ip) {
                    servers.push(ip);
                }
            }
        }
        if servers.is_empty() {
            return Err(ExitDnsUnavailableReason::NoServers);
        }
        let routes = installed
            .domains
            .iter()
            .map(|(name, _)| name.trim_end_matches('.').to_ascii_lowercase())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if !self.proxy_stub_listening().await {
            return Err(ExitDnsUnavailableReason::ProxyStubMissing);
        }
        servers.sort();
        Ok(Proxy { routes, servers })
    }
    /// Whether this namespace has a UDP socket bound and a TCP socket
    /// listening at exactly the proxy stub, re-read at most every two
    /// seconds and whenever resolved's owner changes. systemd < 248 and
    /// `DNSStubListener=no` have neither.
    async fn proxy_stub_listening(&mut self) -> bool {
        if let Some((owner, at, listening)) = &self.stub
            && *owner == self.service_owner
            && at.elapsed() < Duration::from_secs(2)
        {
            return *listening;
        }
        let (udp, tcp) = tokio::join!(
            tokio::fs::read_to_string("/proc/net/udp"),
            tokio::fs::read_to_string("/proc/net/tcp")
        );
        let listening = match (udp, tcp) {
            (Ok(udp), Ok(tcp)) => proxy_stub_in(&udp, &tcp),
            _ => false,
        };
        self.stub = Some((
            self.service_owner.clone(),
            std::time::Instant::now(),
            listening,
        ));
        listening
    }
    async fn settings(&self, previous: &LinkSettings, settings: &LinkSettings) -> Checked<()> {
        let index = self.record.link.index as i32;
        let mut current = previous.clone();
        let changed = |expected: &LinkSettings, actual: &LinkSettings, check| {
            let (expected, found) = expected.differences(actual);
            SystemDnsFailure::new(SystemDnsError::Conflict, check)
                .expected(expected)
                .actual(found)
                .source(self.link_source())
        };
        macro_rules! set {
            ($field:ident, $method:literal, $value:expr) => {{
                // Effective defaults are not configured overrides. In
                // particular an implicit DefaultRoute=false becomes true
                // when DNS servers appear: explicitly set it before DNS.
                self.verify().await?;
                let actual =
                    read_settings(&self.connection, &self.service_owner, &self.path).await?;
                if actual != current {
                    return Err(changed(&current, &actual, "link_settings"));
                }
                let mut next = current.clone();
                next.$field = $value;
                self.call::<_, ()>($method, &(index, &next.$field)).await?;
                let actual =
                    read_settings(&self.connection, &self.service_owner, &self.path).await?;
                if actual != next {
                    return Err(changed(&next, &actual, "link_settings_applied"));
                }
                current = next;
            }};
        }
        set!(default_route, "SetLinkDefaultRoute", settings.default_route);
        set!(llmnr, "SetLinkLLMNR", settings.llmnr.clone());
        set!(mdns, "SetLinkMulticastDNS", settings.mdns.clone());
        set!(tls, "SetLinkDNSOverTLS", settings.tls.clone());
        set!(dnssec, "SetLinkDNSSEC", settings.dnssec.clone());
        set!(dns, "SetLinkDNSEx", settings.dns.clone());
        set!(domains, "SetLinkDomains", settings.domains.clone());
        if current != *settings {
            return Err(changed(settings, &current, "link_settings_applied"));
        }
        let actual = read_settings(&self.connection, &self.service_owner, &self.path).await?;
        if actual != *settings {
            return Err(changed(settings, &actual, "link_settings_applied"));
        }
        self.verify().await
    }
    pub async fn install(&mut self, plan: &SystemDnsPlan) -> Checked<()> {
        self.observe().await?;
        self.conflicts(plan).await?;
        if !self.probed {
            // Never probe by replacing an already kept forced suffix route:
            // restart must keep those suffixes directed at the real stub.
            if self.record.installed.is_none() {
                self.probe().await?;
            } else {
                // The prior record proves that the same namespace/link completed
                // the probe, and the current resolved still serves exactly the
                // installed settings. Lost settings clear `installed` instead.
                self.probed = true;
            }
        }
        let wanted = self.record.settings(plan);
        if self.record.installed.as_ref() == Some(&wanted) {
            return Ok(());
        }
        self.record.pending = true;
        self.persist().await?;
        self.settings(
            self.record
                .installed
                .as_ref()
                .unwrap_or(&self.record.original),
            &wanted,
        )
        .await?;
        self.conflicts(plan).await?;
        self.record.installed = Some(wanted);
        self.record.plan = Some(plan.clone());
        self.record.pending = false;
        self.persist().await?;
        self.observe().await
    }
    async fn probe(&mut self) -> Checked<()> {
        let probe_failure = |error, actual: &str| {
            SystemDnsFailure::new(error, "probe")
                .expected("resolved forwards a nonce query to the owned link over UDP then TCP")
                .actual(actual)
        };
        let udp = tokio::net::UdpSocket::bind(SocketAddr::new(self.record.bind.ip(), 0))
            .await
            .map_err(|error| probe_failure(SystemDnsError::Backend, &error.to_string()))?;
        let address = udp
            .local_addr()
            .map_err(|error| probe_failure(SystemDnsError::Backend, &error.to_string()))?;
        let tcp = tokio::net::TcpListener::bind(address)
            .await
            .map_err(|error| probe_failure(SystemDnsError::Backend, &error.to_string()))?;
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce)
            .map_err(|_| probe_failure(SystemDnsError::Backend, "no randomness"))?;
        let name = format!("p{:032x}.meshway-probe.test", u128::from_be_bytes(nonce));
        let probe = local_settings(address, vec![(name.clone(), true)]);
        self.record.pending = true;
        self.persist().await?;
        self.settings(&self.record.original, &probe).await?;
        let index = self.record.link.index as i32;
        type Records = (Vec<(i32, u16, u16, Vec<u8>)>, u64);
        let arguments = (index, &name, 1u16, 1u16, 1u64);
        let lookup = self.call::<_, Records>("ResolveRecord", &arguments);
        let reached = std::sync::atomic::AtomicBool::new(false);
        let (lookup, served) = tokio::join!(
            lookup,
            tokio::time::timeout(TIMEOUT, serve_probe(udp, tcp, &name, &reached))
        );
        // Restore even after a known probe failure, but only if our exact
        // temporary settings remain. An uncertain mutation retains the journal.
        let actual = read_settings(&self.connection, &self.service_owner, &self.path).await?;
        if actual != probe {
            let (expected, found) = probe.differences(&actual);
            return Err(
                SystemDnsFailure::new(SystemDnsError::Conflict, "link_settings")
                    .expected(expected)
                    .actual(found)
                    .source(self.link_source()),
            );
        }
        self.settings(&probe, &self.record.original).await?;
        self.record.pending = false;
        self.persist().await?;
        if !reached.load(std::sync::atomic::Ordering::Relaxed) && !matches!(served, Ok(Ok(()))) {
            // Without systemd commit 5f9041af (v253, and the stable
            // releases it was backported to) resolved binds its query socket
            // to the link (SO_BINDTOIFINDEX) even for a loopback server, so
            // the query to the bind never arrives: Ubuntu 22.04's 249.11.
            return Err(SystemDnsFailure::new(SystemDnsError::Unsupported, "probe")
                .expected(format!(
                    "resolved sends a nonce query to the owned link's loopback DNS server over UDP then TCP ({LOOPBACK_SERVER_RELEASES})"
                ))
                .actual(format!(
                    "no query reached {address} within 5 s; {}",
                    self.systemd_version().await
                )));
        }
        match (lookup, served) {
            (Ok(_), Ok(Ok(()))) => {}
            (Err(failure), _) => {
                return Err(probe_failure(
                    SystemDnsError::Unsupported,
                    &format!("lookup failed: {failure}"),
                ));
            }
            (_, Ok(Err(error))) => {
                return Err(probe_failure(
                    SystemDnsError::Unsupported,
                    &format!("probe server: {error}"),
                ));
            }
            (_, Err(_)) => {
                return Err(probe_failure(
                    SystemDnsError::Unsupported,
                    "no UDP and TCP query within 5 s",
                ));
            }
        }
        self.probed = true;
        Ok(())
    }
    /// The service manager's version, for a failed probe's report only: it
    /// proves nothing (resolved is the probe's subject).
    async fn systemd_version(&self) -> String {
        let version: Checked<String> = property(
            &self.connection,
            "org.freedesktop.systemd1",
            "/org/freedesktop/systemd1",
            "org.freedesktop.systemd1.Manager",
            "Version",
            "probe",
        )
        .await;
        match version {
            Ok(version) => format!("systemd {version}"),
            Err(_) => "systemd version unknown".to_owned(),
        }
    }
    pub async fn down(&mut self) -> Checked<()> {
        self.observe().await?;
        if let Some(installed) = self.record.installed.clone() {
            self.record.pending = true;
            self.persist().await?;
            let original = self.record.original.clone();
            self.settings(&installed, &original).await?;
        }
        let source = self.record_source();
        self.writes
            .save(self.directory.clone(), RECORD, &Option::<Record>::None)
            .await
            .at("record_write")
            .map_err(|failure| failure.source(source))?;
        self.record.pending = false;
        self.record.installed = None;
        self.record.plan = None;
        self.local.clear();
        Ok(())
    }
}
impl Record {
    /// Installed link settings: the plan's domains plus the route-only marker.
    fn settings(&self, plan: &SystemDnsPlan) -> LinkSettings {
        let mut domains = plan.domains();
        if let Some(marker) = &self.installation {
            domains.push((format!("{marker}.{MARKER_SUFFIX}"), true));
        }
        local_settings(self.bind, domains)
    }
}
/// Whether `/proc/net/udp` has a bound socket (TCP_CLOSE, 07) and
/// `/proc/net/tcp` a listening one (0A) at exactly the proxy stub. The kernel
/// prints the IPv4 address as its in-memory u32 in hexadecimal.
fn proxy_stub_in(udp: &str, tcp: &str) -> bool {
    let IpAddr::V4(ip) = PROXY_STUB.ip() else {
        return false;
    };
    let local = format!(
        "{:08X}:{:04X}",
        u32::from_ne_bytes(ip.octets()),
        PROXY_STUB.port()
    );
    let has = |table: &str, state: &str| {
        table.lines().skip(1).any(|line| {
            let mut fields = line.split_whitespace().skip(1);
            fields.next() == Some(local.as_str()) && fields.nth(1) == Some(state)
        })
    };
    has(udp, "07") && has(tcp, "0A")
}
/// Interface names by index, as far as the kernel names them now; the
/// global settings (index 0) have none.
fn link_names(indexes: impl Iterator<Item = u32>) -> std::collections::BTreeMap<u32, String> {
    use rustix::net::{AddressFamily, SocketType};
    let Ok(socket) = rustix::net::socket(AddressFamily::INET, SocketType::DGRAM, None) else {
        return Default::default();
    };
    indexes
        .filter(|index| *index != 0)
        .filter_map(|index| {
            rustix::net::netdevice::index_to_name(&socket, index)
                .ok()
                .map(|name| (index, name))
        })
        .collect()
}
fn marker() -> Result<String, SystemDnsError> {
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| SystemDnsError::Backend)?;
    Ok(format!("i{:032x}", u128::from_be_bytes(nonce)))
}
fn local_settings(bind: SocketAddr, domains: Vec<(String, bool)>) -> LinkSettings {
    let (family, bytes) = match bind.ip() {
        IpAddr::V4(ip) => (2, ip.octets().to_vec()),
        IpAddr::V6(ip) => (10, ip.octets().to_vec()),
    };
    LinkSettings {
        dns: vec![(family, bytes, bind.port(), String::new())],
        domains,
        default_route: false,
        llmnr: "no".into(),
        mdns: "no".into(),
        tls: "no".into(),
        dnssec: "no".into(),
    }
}
