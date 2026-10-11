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
    DnsLinkIdentity, ExitDnsUnavailableReason, Namespace, OwnedDnsLink, SystemDnsError,
    SystemDnsPlan, storage,
};
use meshway_private_storage::Directory;
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use zbus::zvariant::{DynamicDeserialize, DynamicType, OwnedObjectPath, OwnedValue};

const PATH: &str = "/org/freedesktop/resolve1";
const IFACE: &str = "org.freedesktop.resolve1.Manager";
const SERVICE: &str = "org.freedesktop.resolve1";
const TIMEOUT: Duration = Duration::from_secs(5);
const VERSION: u32 = 2;
const MARKER_SUFFIX: &str = "meshway-installation.invalid";
type DnsServer = (i32, Vec<u8>, u16, String);
type GlobalDomains = Vec<(i32, String, bool)>;
/// resolved's manager-wide server lists: (ifindex, family, address, port, name).
type GlobalServers = Vec<(i32, i32, Vec<u8>, u16, String)>;
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
}
pub(super) fn has_record(directory: &Directory) -> Result<bool, SystemDnsError> {
    storage::read::<Record>(directory, "resolved.json").map(|record| record.is_some())
}
impl Backend {
    pub async fn prepare(
        directory: Arc<Directory>,
        namespace: Namespace,
        bind: SocketAddr,
        anchor: Arc<dyn OwnedDnsLink>,
    ) -> Result<Self, SystemDnsError> {
        let identity = anchor.verify().await?;
        if identity.namespace_device != namespace.device
            || identity.namespace_inode != namespace.inode
            || identity.boot.trim() != namespace.boot
            || identity.index == 0
            || identity.index > i32::MAX as u32
        {
            return Err(SystemDnsError::Conflict);
        }
        let connection = tokio::time::timeout(TIMEOUT, zbus::Connection::system())
            .await
            .map_err(|_| SystemDnsError::Unsupported)?
            .map_err(|_| SystemDnsError::Unsupported)?;
        let (service_owner, path) = attach(&connection, &namespace, identity.index).await?;
        let actual = read_settings(&connection, &service_owner, &path).await?;
        if anchor.verify().await? != identity {
            return Err(SystemDnsError::Conflict);
        }
        let dir = directory.clone();
        let previous: Option<Record> =
            tokio::task::spawn_blocking(move || storage::read(&dir, "resolved.json"))
                .await
                .map_err(|_| SystemDnsError::Storage)??;
        let mut lost = false;
        let record = match previous {
            Some(mut record) => {
                if !record.original.dns.is_empty() || !record.original.domains.is_empty() {
                    return Err(SystemDnsError::Conflict);
                }
                match (&record.installed, &record.plan) {
                    (Some(installed), Some(plan)) => {
                        plan.validate()?;
                        if *installed != record.settings(plan) {
                            return Err(SystemDnsError::Conflict);
                        }
                    }
                    (None, None) => {}
                    _ => return Err(SystemDnsError::Conflict),
                }
                let valid = match record.version {
                    1 => record.service_owner.is_some() && record.installation.is_none(),
                    VERSION => record.service_owner.is_none() && record.installation.is_some(),
                    _ => false,
                };
                if !valid || record.pending || record.link != identity || record.bind != bind {
                    return Err(SystemDnsError::Conflict);
                }
                if record.installed.as_ref().unwrap_or(&record.original) != &actual {
                    // Only the exact original settings mean resolved dropped
                    // this link's runtime configuration (a restart without its
                    // saved link state). Any other value is a foreign writer.
                    if record.installed.is_none() || actual != record.original {
                        return Err(SystemDnsError::Conflict);
                    }
                    record.installed = None;
                    record.plan = None;
                    lost = true;
                }
                if record.installation.is_none() {
                    record.installation = Some(marker()?);
                }
                record.version = VERSION;
                record.service_owner = None;
                record
            }
            None => {
                if !actual.dns.is_empty() || !actual.domains.is_empty() {
                    return Err(SystemDnsError::Conflict);
                }
                Record {
                    version: VERSION,
                    service_owner: None,
                    installation: Some(marker()?),
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
        };
        if lost {
            // The link is at its original settings: record exactly that, so
            // the next install probes this resolved and installs afresh.
            backend.persist().await?;
        }
        backend.observe().await?;
        Ok(backend)
    }
    async fn verify(&self) -> Result<(), SystemDnsError> {
        verify_owner(&self.connection, &self.service_owner, &self.namespace).await?;
        verify_os_resolver(&self.connection, &self.service_owner).await?;
        if self.anchor.verify().await? != self.record.link {
            return Err(SystemDnsError::Conflict);
        }
        Ok(())
    }
    async fn call<B, R>(&self, method: &str, body: &B) -> Result<R, SystemDnsError>
    where
        B: Serialize + DynamicType,
        R: for<'d> DynamicDeserialize<'d>,
    {
        self.verify().await?;
        let result = tokio::select! { biased;
            _ = self.anchor.ended() => return Err(SystemDnsError::Conflict),
            result = raw(&self.connection, &self.service_owner, PATH, IFACE, method, body) => result,
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
    async fn persist(&mut self) -> Result<(), SystemDnsError> {
        self.writes
            .save(self.directory.clone(), "resolved.json", &Some(&self.record))
            .await
    }
    async fn conflicts(&self, plan: &SystemDnsPlan) -> Result<(), SystemDnsError> {
        let domains: GlobalDomains = property(
            &self.connection,
            &self.service_owner,
            PATH,
            IFACE,
            "Domains",
        )
        .await?;
        if domains.len() > 4096 {
            return Err(SystemDnsError::Capacity);
        }
        if domains
            .iter()
            .any(|(index, name, _)| *index != self.record.link.index as i32 && plan.conflicts(name))
        {
            return Err(SystemDnsError::Conflict);
        }
        Ok(())
    }
    pub async fn observe(&mut self) -> Result<(), SystemDnsError> {
        self.settle().await?;
        if self.record.pending {
            return Err(SystemDnsError::Conflict);
        }
        let (owner, path) =
            attach(&self.connection, &self.namespace, self.record.link.index).await?;
        if owner != self.service_owner {
            // A restarted resolved is adopted only if it kept these exact
            // settings; the comparison below is the ownership proof.
            self.service_owner = owner;
            self.path = path;
        }
        self.verify().await?;
        let actual = read_settings(&self.connection, &self.service_owner, &self.path).await?;
        if actual
            != *self
                .record
                .installed
                .as_ref()
                .unwrap_or(&self.record.original)
        {
            return Err(SystemDnsError::Conflict);
        }
        if let Some(plan) = &self.record.plan {
            self.conflicts(plan).await?;
        }
        self.verify().await
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
            let entries: GlobalServers =
                property(&self.connection, &self.service_owner, PATH, IFACE, list)
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
    async fn settings(
        &self,
        previous: &LinkSettings,
        settings: &LinkSettings,
    ) -> Result<(), SystemDnsError> {
        let index = self.record.link.index as i32;
        let mut current = previous.clone();
        macro_rules! set {
            ($field:ident, $method:literal, $value:expr) => {{
                // Effective defaults are not configured overrides. In
                // particular an implicit DefaultRoute=false becomes true
                // when DNS servers appear: explicitly set it before DNS.
                self.verify().await?;
                if read_settings(&self.connection, &self.service_owner, &self.path).await?
                    != current
                {
                    return Err(SystemDnsError::Conflict);
                }
                let mut next = current.clone();
                next.$field = $value;
                self.call::<_, ()>($method, &(index, &next.$field)).await?;
                if read_settings(&self.connection, &self.service_owner, &self.path).await? != next {
                    return Err(SystemDnsError::Conflict);
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
            return Err(SystemDnsError::Conflict);
        }
        if read_settings(&self.connection, &self.service_owner, &self.path).await? != *settings {
            return Err(SystemDnsError::Conflict);
        }
        self.verify().await
    }
    pub async fn install(&mut self, plan: &SystemDnsPlan) -> Result<(), SystemDnsError> {
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
    async fn probe(&mut self) -> Result<(), SystemDnsError> {
        let udp = tokio::net::UdpSocket::bind(SocketAddr::new(self.record.bind.ip(), 0))
            .await
            .map_err(|_| SystemDnsError::Backend)?;
        let address = udp.local_addr().map_err(|_| SystemDnsError::Backend)?;
        let tcp = tokio::net::TcpListener::bind(address)
            .await
            .map_err(|_| SystemDnsError::Backend)?;
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| SystemDnsError::Backend)?;
        let name = format!("p{:032x}.meshway-probe.test", u128::from_be_bytes(nonce));
        let probe = local_settings(address, vec![(name.clone(), true)]);
        self.record.pending = true;
        self.persist().await?;
        self.settings(&self.record.original, &probe).await?;
        let index = self.record.link.index as i32;
        type Records = (Vec<(i32, u16, u16, Vec<u8>)>, u64);
        let arguments = (index, &name, 1u16, 1u16, 1u64);
        let lookup = self.call::<_, Records>("ResolveRecord", &arguments);
        let (lookup, served) = tokio::join!(
            lookup,
            tokio::time::timeout(TIMEOUT, serve_probe(udp, tcp, &name))
        );
        // Restore even after a known probe failure, but only if our exact
        // temporary settings remain. An uncertain mutation retains the journal.
        if read_settings(&self.connection, &self.service_owner, &self.path).await? != probe {
            return Err(SystemDnsError::Conflict);
        }
        self.settings(&probe, &self.record.original).await?;
        self.record.pending = false;
        self.persist().await?;
        if lookup.is_err() || !matches!(served, Ok(Ok(()))) {
            return Err(SystemDnsError::Unsupported);
        }
        self.probed = true;
        Ok(())
    }
    pub async fn down(&mut self) -> Result<(), SystemDnsError> {
        self.observe().await?;
        if self.record.installed.is_some() {
            self.record.pending = true;
            self.persist().await?;
            self.settings(
                self.record
                    .installed
                    .as_ref()
                    .ok_or(SystemDnsError::Conflict)?,
                &self.record.original,
            )
            .await?;
        }
        self.writes
            .save(
                self.directory.clone(),
                "resolved.json",
                &Option::<Record>::None,
            )
            .await?;
        self.record.pending = false;
        self.record.installed = None;
        self.record.plan = None;
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
fn marker() -> Result<String, SystemDnsError> {
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| SystemDnsError::Backend)?;
    Ok(format!("i{:032x}", u128::from_be_bytes(nonce)))
}
/// The current resolve1 owner, proven to run in this namespace as the OS
/// resolver, and the object path of the owned link on it.
async fn attach(
    connection: &zbus::Connection,
    namespace: &Namespace,
    index: u32,
) -> Result<(String, String), SystemDnsError> {
    let owner: String = raw(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetNameOwner",
        &(SERVICE,),
    )
    .await?;
    verify_owner(connection, &owner, namespace).await?;
    verify_os_resolver(connection, &owner).await?;
    let path: OwnedObjectPath =
        raw(connection, &owner, PATH, IFACE, "GetLink", &(index as i32,)).await?;
    Ok((owner, path.to_string()))
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
async fn raw<B, R>(
    connection: &zbus::Connection,
    destination: &str,
    path: &str,
    interface: &str,
    method: &str,
    body: &B,
) -> Result<R, SystemDnsError>
where
    B: Serialize + DynamicType,
    R: for<'d> DynamicDeserialize<'d>,
{
    tokio::time::timeout(TIMEOUT, async {
        let reply = connection
            .call_method(Some(destination), path, Some(interface), method, body)
            .await
            .map_err(|_| SystemDnsError::Backend)?;
        reply
            .body()
            .deserialize()
            .map_err(|_| SystemDnsError::Backend)
    })
    .await
    .map_err(|_| SystemDnsError::Backend)?
}
async fn property<T>(
    connection: &zbus::Connection,
    destination: &str,
    path: &str,
    interface: &str,
    name: &str,
) -> Result<T, SystemDnsError>
where
    T: TryFrom<OwnedValue>,
{
    let value: OwnedValue = raw(
        connection,
        destination,
        path,
        "org.freedesktop.DBus.Properties",
        "Get",
        &(interface, name),
    )
    .await?;
    value.try_into().map_err(|_| SystemDnsError::Backend)
}
async fn verify_owner(
    connection: &zbus::Connection,
    expected: &str,
    namespace: &Namespace,
) -> Result<(), SystemDnsError> {
    let current: String = raw(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetNameOwner",
        &(SERVICE,),
    )
    .await?;
    if current != expected {
        return Err(SystemDnsError::Conflict);
    }
    let pid: u32 = raw(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetConnectionUnixProcessID",
        &(expected,),
    )
    .await?;
    // resolved commonly runs as systemd-resolve, not root. The system bus owns
    // service-name policy; PID/netns is checked regardless of service UID.
    namespace.verify_pid(pid)
}

async fn verify_os_resolver(
    connection: &zbus::Connection,
    owner: &str,
) -> Result<(), SystemDnsError> {
    let mode: String = property(connection, owner, PATH, IFACE, "ResolvConfMode").await?;
    let listener: String = property(connection, owner, PATH, IFACE, "DNSStubListener").await?;
    // An active resolve1 bus is insufficient: uplink/foreign resolv.conf lets
    // libc bypass per-link routing. Initial OS support requires the actual
    // dynamically maintained resolved stub file and both UDP/TCP listeners.
    // Static mode does not publish installed search domains to libc. NSS overrides are separate
    // host integration, never inferred from D-Bus method success.
    if mode != "stub" || listener != "yes" {
        return Err(SystemDnsError::Unsupported);
    }
    Ok(())
}
async fn read_settings(
    connection: &zbus::Connection,
    owner: &str,
    path: &str,
) -> Result<LinkSettings, SystemDnsError> {
    const LINK: &str = "org.freedesktop.resolve1.Link";
    let settings = LinkSettings {
        dns: property(connection, owner, path, LINK, "DNSEx").await?,
        domains: property(connection, owner, path, LINK, "Domains").await?,
        default_route: property(connection, owner, path, LINK, "DefaultRoute").await?,
        llmnr: property(connection, owner, path, LINK, "LLMNR").await?,
        mdns: property(connection, owner, path, LINK, "MulticastDNS").await?,
        tls: property(connection, owner, path, LINK, "DNSOverTLS").await?,
        dnssec: property(connection, owner, path, LINK, "DNSSEC").await?,
    };
    if settings.dns.len() > 8 || settings.domains.len() > 256 {
        return Err(SystemDnsError::Capacity);
    }
    Ok(settings)
}
async fn serve_probe(
    udp: tokio::net::UdpSocket,
    tcp: tokio::net::TcpListener,
    expected: &str,
) -> Result<(), SystemDnsError> {
    use hickory_proto::rr::{Name, RData, Record, RecordType, rdata};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut expected = Name::from_ascii(expected).map_err(|_| SystemDnsError::Backend)?;
    expected.set_fqdn(true);
    let mut buffer = [0u8; 4096];
    let (size, peer) = udp
        .recv_from(&mut buffer)
        .await
        .map_err(|_| SystemDnsError::Backend)?;
    let query = crate::decode_query(&buffer[..size]).map_err(|_| SystemDnsError::Backend)?;
    if !peer.ip().is_loopback()
        || query.question().name() != &expected
        || query.question().query_type() != RecordType::A
    {
        return Err(SystemDnsError::Backend);
    }
    let mut response = query.response();
    response.metadata.truncation = true;
    let bytes = crate::encode_response(&query, &response, crate::ResponseTransport::Udp, 4096)
        .map_err(|_| SystemDnsError::Backend)?;
    udp.send_to(&bytes, peer)
        .await
        .map_err(|_| SystemDnsError::Backend)?;
    let (mut stream, peer) = tcp.accept().await.map_err(|_| SystemDnsError::Backend)?;
    if !peer.ip().is_loopback() {
        return Err(SystemDnsError::Backend);
    }
    let size = stream
        .read_u16()
        .await
        .map_err(|_| SystemDnsError::Backend)? as usize;
    if !(12..=buffer.len()).contains(&size) {
        return Err(SystemDnsError::Backend);
    }
    stream
        .read_exact(&mut buffer[..size])
        .await
        .map_err(|_| SystemDnsError::Backend)?;
    let query = crate::decode_query(&buffer[..size]).map_err(|_| SystemDnsError::Backend)?;
    if query.question().name() != &expected || query.question().query_type() != RecordType::A {
        return Err(SystemDnsError::Backend);
    }
    let mut response = query.response();
    response.add_answer(Record::from_rdata(
        expected,
        0,
        RData::A(rdata::A(std::net::Ipv4Addr::LOCALHOST)),
    ));
    let bytes = crate::encode_response(&query, &response, crate::ResponseTransport::Tcp, 4096)
        .map_err(|_| SystemDnsError::Backend)?;
    stream
        .write_u16(bytes.len() as u16)
        .await
        .map_err(|_| SystemDnsError::Backend)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(|_| SystemDnsError::Backend)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_proxy_stub_needs_its_exact_udp_and_tcp_sockets() {
        let header = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n";
        let row = |address: &str, state: &str| {
            format!(
                "   0: {address} 00000000:0000 {state} 00000000:00000000 00:00000000 00000000   991        0 1 2 0000000000000000 0\n"
            )
        };
        let stub = format!("{:08X}:0035", u32::from_ne_bytes([127, 0, 0, 54]));
        let main = format!("{:08X}:0035", u32::from_ne_bytes([127, 0, 0, 53]));
        let udp = format!("{header}{}{}", row(&main, "07"), row(&stub, "07"));
        let tcp = format!("{header}{}{}", row(&main, "0A"), row(&stub, "0A"));
        assert!(super::proxy_stub_in(&udp, &tcp));
        // systemd < 248: only 127.0.0.53.
        let old_udp = format!("{header}{}", row(&main, "07"));
        let old_tcp = format!("{header}{}", row(&main, "0A"));
        assert!(!super::proxy_stub_in(&old_udp, &old_tcp));
        // A connected or non-listening socket at the address is not the stub.
        assert!(!super::proxy_stub_in(
            &udp,
            &format!("{header}{}", row(&stub, "01"))
        ));
        assert!(!super::proxy_stub_in(
            &format!("{header}{}", row(&stub, "01")),
            &tcp
        ));
        assert!(!super::proxy_stub_in("", ""));
    }
}
