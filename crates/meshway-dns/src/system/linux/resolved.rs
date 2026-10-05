//! Per-owned-link resolve1 integration. No global settings or ambient resolver
//! fallback are changed. A nonce UDP->TCP probe verifies local link DNS behavior.
use super::{DnsLinkIdentity, Namespace, OwnedDnsLink, SystemDnsError, SystemDnsPlan, storage};
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
type DnsServer = (i32, Vec<u8>, u16, String);
type GlobalDomains = Vec<(i32, String, bool)>;

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
    service_owner: String,
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
        let service_owner: String = raw(
            &connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            &(SERVICE,),
        )
        .await?;
        verify_owner(&connection, &service_owner, &namespace).await?;
        verify_os_resolver(&connection, &service_owner).await?;
        let path: OwnedObjectPath = raw(
            &connection,
            &service_owner,
            PATH,
            IFACE,
            "GetLink",
            &(identity.index as i32,),
        )
        .await?;
        let path = path.to_string();
        let actual = read_settings(&connection, &service_owner, &path).await?;
        if anchor.verify().await? != identity {
            return Err(SystemDnsError::Conflict);
        }
        let dir = directory.clone();
        let previous: Option<Record> =
            tokio::task::spawn_blocking(move || storage::read(&dir, "resolved.json"))
                .await
                .map_err(|_| SystemDnsError::Storage)??;
        let record = match previous {
            Some(record) => {
                if !record.original.dns.is_empty() || !record.original.domains.is_empty() {
                    return Err(SystemDnsError::Conflict);
                }
                match (&record.installed, &record.plan) {
                    (Some(installed), Some(plan)) => {
                        plan.validate()?;
                        if *installed != local_settings(record.bind, plan.domains()) {
                            return Err(SystemDnsError::Conflict);
                        }
                    }
                    (None, None) => {}
                    _ => return Err(SystemDnsError::Conflict),
                }
                if record.version != 1
                    || record.pending
                    || record.link != identity
                    || record.bind != bind
                    || record.service_owner != service_owner
                    || record.installed.as_ref().unwrap_or(&record.original) != &actual
                {
                    return Err(SystemDnsError::Conflict);
                }
                record
            }
            None => {
                if !actual.dns.is_empty() || !actual.domains.is_empty() {
                    return Err(SystemDnsError::Conflict);
                }
                Record {
                    version: 1,
                    service_owner: service_owner.clone(),
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
        };
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
            _ = self.anchor.invalidated() => return Err(SystemDnsError::Conflict),
            result = raw(&self.connection, &self.service_owner, PATH, IFACE, method, body) => result,
        };
        self.verify().await?;
        result
    }
    pub async fn settle(&mut self) -> Result<(), SystemDnsError> {
        self.writes.settle().await
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
            // Never probe by replacing an already retained forced suffix route:
            // restart must keep those suffixes directed at the real stub.
            if self.record.installed.is_none() {
                self.probe().await?;
            } else {
                // The prior record proves that the same namespace/link completed
                // the probe. A service restart is detected separately by owner
                // identity during this process and requires re-preparation.
                self.probed = true;
            }
        }
        let wanted = local_settings(self.record.bind, plan.domains());
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
    let uid: u32 = raw(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetConnectionUnixUser",
        &(expected,),
    )
    .await?;
    // resolved commonly runs as systemd-resolve, not root. The system bus owns
    // service-name policy; PID/netns is checked regardless of service UID.
    let _ = uid;
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
