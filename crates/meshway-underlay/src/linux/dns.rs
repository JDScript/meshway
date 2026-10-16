//! Bootstrap resolver only. Names enter through the fixed origin or a validated
//! execution candidate, never through a public arbitrary-name resolution API.
use hickory_net::runtime::{
    DnsTcpStream, DnsUdpSocket, RuntimeProvider, TokioTime, iocompat::AsyncIoTokioAsStd,
};
use hickory_resolver::{
    Hosts, Resolver as HickoryResolver,
    config::{NameServerConfig, ResolverConfig, ResolverOpts},
};
use meshway_transport::raw::Datagram;
use std::{
    collections::BTreeMap,
    future::Future,
    io,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::AsyncReadExt;

use super::{
    datagram::ProtectedDatagram,
    protection::{ProtectionScope, Shared, io_error},
    sockets::{ProtectedTcp, connect_tcp},
    tasks::Tasks,
};
use crate::{Error, Options, Result};

/// An upstream resolver and the configuration generation it was built from.
type Built = (u64, HickoryResolver<Provider>);

#[derive(Clone)]
pub(super) struct Resolver {
    upstreams: Arc<Upstreams>,
    /// The upstream resolver built for this scope from the upstream
    /// configuration of the generation it names.
    inner: Arc<std::sync::Mutex<Option<Built>>>,
    hosts: Arc<HostsFile>,
    overrides: Arc<BTreeMap<String, Vec<IpAddr>>>,
    shared: Arc<Shared>,
    scope: ProtectionScope,
    timeout: Duration,
}

impl Resolver {
    pub async fn open(shared: Arc<Shared>, options: &Options) -> Result<Self> {
        let hosts = HostsFile::new(std::path::PathBuf::from(HOSTS));
        hosts.current().await?;
        let mut overrides = BTreeMap::new();
        for (name, addresses) in &options.bootstrap_hosts {
            let name =
                meshway_transport::spec::normalize_host(name).map_err(|_| Error::Resolver)?;
            if addresses
                .iter()
                .any(|ip| ip.is_unspecified() || ip.is_multicast())
            {
                return Err(Error::Resolver);
            }
            overrides.insert(name, addresses.clone());
        }
        let upstreams = if let Some(upstreams) = &options.bootstrap_nameservers {
            if upstreams.iter().any(|a| {
                a.port() == 0
                    || a.ip().is_loopback()
                    || a.ip().is_unspecified()
                    || a.ip().is_multicast()
            }) {
                return Err(Error::Resolver);
            }
            let servers = upstreams
                .iter()
                .map(|address| {
                    let mut config = NameServerConfig::udp_and_tcp(address.ip());
                    for connection in &mut config.connections {
                        connection.port = address.port();
                    }
                    config
                })
                .collect();
            Upstreams::fixed(
                ResolverConfig::from_parts(None, Vec::new(), servers),
                options.connect_timeout,
            )
        } else {
            Upstreams::system(SystemPaths::default(), options.connect_timeout)
        };
        let upstreams = Arc::new(upstreams);
        upstreams.current().await?;
        let scope = shared.bootstrap();
        Ok(Self {
            upstreams,
            inner: Arc::default(),
            hosts: Arc::new(hosts),
            overrides: Arc::new(overrides),
            shared,
            scope,
            timeout: options.connect_timeout,
        })
    }

    /// Candidate name resolution has that candidate's execution lifetime. It
    /// cannot borrow the Server bootstrap exemption after publication expiry.
    pub fn scoped(&self, scope: ProtectionScope) -> Result<Self> {
        scope.check()?;
        Ok(Self {
            inner: Arc::default(),
            scope,
            ..self.clone()
        })
    }

    /// The upstream resolver of the system configuration as it is now; a
    /// changed configuration (a new network's DHCP or resolved upstreams)
    /// builds a new one.
    async fn upstream(&self) -> Result<Option<HickoryResolver<Provider>>> {
        let (generation, config) = self.upstreams.current().await?;
        let mut inner = self.inner.lock().map_err(|_| Error::Worker)?;
        if let Some((built, resolver)) = inner.as_ref()
            && *built == generation
        {
            return Ok(Some(resolver.clone()));
        }
        let Some((config, options)) = config else {
            *inner = None;
            return Ok(None);
        };
        let resolver = build(config, options, self.scope.clone(), self.timeout)?;
        *inner = Some((generation, resolver.clone()));
        Ok(Some(resolver))
    }

    pub async fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>> {
        self.scope.check()?;
        let _query = self
            .shared
            .resolutions
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)?;
        let host = meshway_transport::spec::normalize_host(host).map_err(|_| Error::Resolver)?;
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, port)]);
        }
        if let Some(addresses) = self.overrides.get(&host) {
            return Ok(addresses
                .iter()
                .map(|ip| SocketAddr::new(*ip, port))
                .collect());
        }
        let name = hickory_proto::rr::Name::from_ascii(&host).map_err(|_| Error::Resolver)?;
        let hosts = self.hosts.current().await?;
        let mut addresses = Vec::new();
        for ty in [
            hickory_proto::rr::RecordType::A,
            hickory_proto::rr::RecordType::AAAA,
        ] {
            let query = hickory_proto::op::Query::query(name.clone(), ty);
            if let Some(answer) = hosts.lookup_static_host(&query) {
                for record in answer.answers() {
                    match &record.data {
                        hickory_proto::rr::RData::A(a) => {
                            addresses.push(SocketAddr::new(IpAddr::V4(a.0), port))
                        }
                        hickory_proto::rr::RData::AAAA(a) => {
                            addresses.push(SocketAddr::new(IpAddr::V6(a.0), port))
                        }
                        _ => {}
                    }
                }
            }
        }
        if addresses.is_empty() {
            let resolver = self.upstream().await?.ok_or(Error::Resolver)?;
            let scope = self.scope.clone();
            let answer = tokio::select! { biased;
                _ = scope.ended() => return Err(Error::Conflict),
                result = tokio::time::timeout(self.timeout, resolver.lookup_ip(host)) => result.map_err(|_| Error::Resolver)?.map_err(|_| Error::Resolver)?,
            };
            addresses.extend(answer.iter().take(16).map(|ip| SocketAddr::new(ip, port)));
        }
        addresses.truncate(16);
        if addresses.is_empty() {
            return Err(Error::Resolver);
        }
        self.scope.check()?;
        Ok(addresses)
    }
}

/// Where the system resolver configuration lives.
#[derive(Clone)]
struct SystemPaths {
    resolv: std::path::PathBuf,
    /// The systemd stub `resolv` may point at; only it has a discoverable
    /// real upstream file.
    stub: std::path::PathBuf,
    /// resolved's real upstreams behind that stub.
    upstream: std::path::PathBuf,
}

impl Default for SystemPaths {
    fn default() -> Self {
        Self {
            resolv: "/etc/resolv.conf".into(),
            stub: "/run/systemd/resolve/stub-resolv.conf".into(),
            upstream: "/run/systemd/resolve/resolv.conf".into(),
        }
    }
}

type UpstreamConfig = Option<(ResolverConfig, ResolverOpts)>;

/// The bootstrap resolver's upstreams: explicit ones, or the system's as its
/// files are now. Every lookup compares the files' identity and change times
/// (and where `resolv.conf` points) and parses them again when they differ,
/// so a network whose DHCP or resolved replaces the upstreams needs no
/// service restart.
struct Upstreams {
    paths: Option<SystemPaths>,
    timeout: Duration,
    current: std::sync::Mutex<UpstreamState>,
}

struct UpstreamState {
    /// `None` until read; then the files' stamps it was read from.
    stamp: Option<UpstreamStamp>,
    generation: u64,
    config: UpstreamConfig,
}

#[derive(Clone, PartialEq, Eq)]
struct UpstreamStamp {
    target: Option<std::path::PathBuf>,
    resolv: Option<HostsStamp>,
    upstream: Option<HostsStamp>,
}

impl Upstreams {
    fn fixed(config: ResolverConfig, timeout: Duration) -> Self {
        Self {
            paths: None,
            timeout,
            current: std::sync::Mutex::new(UpstreamState {
                stamp: None,
                generation: 0,
                config: Some((config, options(ResolverOpts::default(), timeout))),
            }),
        }
    }

    fn system(paths: SystemPaths, timeout: Duration) -> Self {
        Self {
            paths: Some(paths),
            timeout,
            current: std::sync::Mutex::new(UpstreamState {
                stamp: None,
                generation: 0,
                config: None,
            }),
        }
    }

    /// The configuration's generation (it changes with the configuration)
    /// and the configuration; `None` without a usable upstream.
    async fn current(&self) -> Result<(u64, UpstreamConfig)> {
        let Some(paths) = &self.paths else {
            let current = self.current.lock().map_err(|_| Error::Worker)?;
            return Ok((current.generation, current.config.clone()));
        };
        let stamp = UpstreamStamp {
            target: tokio::fs::canonicalize(&paths.resolv).await.ok(),
            // An unreadable file is no upstream, as an absent one.
            resolv: stamp(&paths.resolv).await.ok().flatten(),
            upstream: stamp(&paths.upstream).await.ok().flatten(),
        };
        {
            let current = self.current.lock().map_err(|_| Error::Worker)?;
            if current.stamp.as_ref() == Some(&stamp) {
                return Ok((current.generation, current.config.clone()));
            }
        }
        let config = system_config(paths, stamp.target.as_deref())
            .await
            .map(|(config, parsed)| (config, options(parsed, self.timeout)));
        let mut current = self.current.lock().map_err(|_| Error::Worker)?;
        if current.stamp.is_some() {
            tracing::info!(
                upstreams = config
                    .as_ref()
                    .map_or(0, |(config, _)| config.name_servers().len()),
                "Bootstrap resolver upstreams changed; using the current configuration"
            );
        }
        current.generation = current.generation.wrapping_add(1);
        current.stamp = Some(stamp);
        current.config = config;
        Ok((current.generation, current.config.clone()))
    }
}

/// This resolver's fixed limits over whatever the configuration asks for.
fn options(mut options: ResolverOpts, timeout: Duration) -> ResolverOpts {
    options.timeout = timeout;
    options.attempts = 2;
    options.cache_size = 64;
    options.num_concurrent_reqs = 2;
    options
}

/// The system's real upstreams: `resolv.conf`'s, or behind the systemd stub
/// resolved's own file. Docker and arbitrary local stubs stay unsupported.
async fn system_config(
    paths: &SystemPaths,
    target: Option<&std::path::Path>,
) -> Option<(ResolverConfig, ResolverOpts)> {
    let mut parsed = match read_bounded(&paths.resolv).await {
        Ok(bytes) => hickory_resolver::system_conf::parse_resolv_conf(&bytes).ok(),
        Err(_) => None,
    };
    if parsed
        .as_ref()
        .is_some_and(|(cfg, _)| cfg.name_servers().iter().any(|s| s.ip.is_loopback()))
    {
        parsed = if target == Some(paths.stub.as_path()) {
            match read_bounded(&paths.upstream).await {
                Ok(bytes) => hickory_resolver::system_conf::parse_resolv_conf(&bytes).ok(),
                Err(_) => None,
            }
        } else {
            None
        };
    }
    parsed.filter(|(cfg, _)| {
        !cfg.name_servers().is_empty()
            && cfg.name_servers().len() <= 8
            && cfg
                .name_servers()
                .iter()
                .all(|s| !s.ip.is_loopback() && !s.ip.is_unspecified() && !s.ip.is_multicast())
    })
}

fn build(
    config: ResolverConfig,
    options: ResolverOpts,
    scope: ProtectionScope,
    timeout: Duration,
) -> Result<HickoryResolver<Provider>> {
    let upstreams = config
        .name_servers()
        .iter()
        .flat_map(|server| {
            server
                .connections
                .iter()
                .map(move |connection| SocketAddr::new(server.ip, connection.port))
        })
        .collect();
    let provider = Provider {
        scope,
        upstreams: Arc::new(upstreams),
        timeout,
    };
    let mut builder = HickoryResolver::builder_with_config(config, provider);
    *builder.options_mut() = options;
    let mut resolver = builder.build().map_err(|_| Error::Resolver)?;
    // System hosts are answered above from the current file only; the
    // upstream resolver never holds a copy that could go stale.
    resolver.set_hosts(Arc::new(Hosts::default()));
    Ok(resolver)
}

const HOSTS: &str = "/etc/hosts";

/// A file's identity and change times as they are now; `None` when absent.
async fn stamp(path: &std::path::Path) -> Result<Option<HostsStamp>> {
    use std::os::unix::fs::MetadataExt;
    match tokio::fs::metadata(path).await {
        Ok(meta) => Ok(Some(HostsStamp {
            device: meta.dev(),
            inode: meta.ino(),
            size: meta.size(),
            modified: (meta.mtime(), meta.mtime_nsec()),
            changed: (meta.ctime(), meta.ctime_nsec()),
        })),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// The system hosts file as it is now: every lookup compares the file's
/// identity and change times and re-reads it when they differ, so editing
/// `/etc/hosts` needs no service restart.
struct HostsFile {
    path: std::path::PathBuf,
    current: std::sync::Mutex<(Option<HostsStamp>, Arc<Hosts>)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct HostsStamp {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl HostsFile {
    fn new(path: std::path::PathBuf) -> Self {
        Self {
            path,
            current: std::sync::Mutex::new((None, Arc::new(Hosts::default()))),
        }
    }

    async fn current(&self) -> Result<Arc<Hosts>> {
        let stamp = stamp(&self.path).await?;
        {
            let current = self.current.lock().map_err(|_| Error::Worker)?;
            if current.0 == stamp {
                return Ok(current.1.clone());
            }
        }
        let mut hosts = Hosts::default();
        if stamp.is_some() {
            match read_bounded(&self.path).await {
                Ok(bytes) => hosts
                    .read_hosts_conf(&bytes[..])
                    .map_err(|_| Error::Resolver)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let hosts = Arc::new(hosts);
        *self.current.lock().map_err(|_| Error::Worker)? = (stamp, hosts.clone());
        Ok(hosts)
    }
}

async fn read_bounded(path: impl AsRef<std::path::Path>) -> io::Result<Vec<u8>> {
    let file = tokio::fs::File::open(path).await?;
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes).await?;
    if bytes.len() > 65536 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}

#[derive(Clone)]
struct Provider {
    scope: ProtectionScope,
    upstreams: Arc<Vec<SocketAddr>>,
    timeout: Duration,
}

impl RuntimeProvider for Provider {
    type Handle = Tasks;
    type Timer = TokioTime;
    type Udp = DnsUdp;
    type Tcp = DnsTcp;

    fn create_handle(&self) -> Self::Handle {
        self.scope.shared.tasks.clone()
    }

    fn connect_tcp(
        &self,
        server: SocketAddr,
        bind: Option<SocketAddr>,
        timeout: Option<Duration>,
    ) -> Pin<Box<dyn Send + Future<Output = io::Result<Self::Tcp>>>> {
        let this = self.clone();
        Box::pin(async move {
            if !this.upstreams.contains(&server) {
                return Err(io_error(Error::Resolver));
            }
            connect_tcp(
                this.scope,
                server,
                bind,
                &Default::default(),
                &Default::default(),
                timeout.unwrap_or(this.timeout).min(this.timeout),
            )
            .await
            .map(|stream| DnsTcp(AsyncIoTokioAsStd(stream)))
            .map_err(io_error)
        })
    }

    fn bind_udp(
        &self,
        local: SocketAddr,
        server: SocketAddr,
    ) -> Pin<Box<dyn Send + Future<Output = io::Result<Self::Udp>>>> {
        let this = self.clone();
        Box::pin(async move {
            if !this.upstreams.contains(&server) {
                return Err(io_error(Error::Resolver));
            }
            Ok(DnsUdp {
                endpoint: ProtectedDatagram::bind(
                    this.scope,
                    local,
                    &Default::default(),
                    Some(vec![server]),
                )
                .map_err(io_error)?,
                server,
            })
        })
    }
}

struct DnsUdp {
    endpoint: ProtectedDatagram,
    server: SocketAddr,
}

impl DnsUdpSocket for DnsUdp {
    type Time = TokioTime;

    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<(usize, SocketAddr)>> {
        match self.endpoint.poll_recv_from(cx, buffer) {
            Poll::Ready(Ok(read)) if read.source == self.server && !read.truncated => {
                Poll::Ready(Ok((read.copied_len, read.source)))
            }
            Poll::Ready(Ok(_)) => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        bytes: &[u8],
        target: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        if target != self.server {
            return Poll::Ready(Err(io_error(Error::Resolver)));
        }
        self.endpoint.poll_send_to(cx, bytes, target)
    }
}

struct DnsTcp(AsyncIoTokioAsStd<ProtectedTcp>);
impl DnsTcpStream for DnsTcp {
    type Time = TokioTime;
}
impl futures_util::io::AsyncRead for DnsTcp {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        futures_util::io::AsyncRead::poll_read(Pin::new(&mut self.get_mut().0), cx, bytes)
    }
}
impl futures_util::io::AsyncWrite for DnsTcp {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        futures_util::io::AsyncWrite::poll_write(Pin::new(&mut self.get_mut().0), cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        futures_util::io::AsyncWrite::poll_flush(Pin::new(&mut self.get_mut().0), cx)
    }
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        futures_util::io::AsyncWrite::poll_close(Pin::new(&mut self.get_mut().0), cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn addresses(hosts: &HostsFile, name: &str) -> Vec<IpAddr> {
        let query = hickory_proto::op::Query::query(
            hickory_proto::rr::Name::from_ascii(name).unwrap(),
            hickory_proto::rr::RecordType::A,
        );
        hosts
            .current()
            .await
            .unwrap()
            .lookup_static_host(&query)
            .map(|answer| {
                answer
                    .answers()
                    .iter()
                    .filter_map(|record| match &record.data {
                        hickory_proto::rr::RData::A(a) => Some(IpAddr::V4(a.0)),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn servers(config: &UpstreamConfig) -> Vec<IpAddr> {
        config
            .as_ref()
            .map(|(config, _)| {
                config
                    .name_servers()
                    .iter()
                    .map(|server| server.ip)
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn bootstrap_upstreams_follow_the_system_files_as_they_are_now() {
        let root = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(root.path()).unwrap();
        let paths = SystemPaths {
            resolv: root.join("resolv.conf"),
            stub: root.join("stub-resolv.conf"),
            upstream: root.join("upstream.conf"),
        };
        let replace = |path: &std::path::Path, text: &str| {
            let next = root.join("next");
            std::fs::write(&next, text).unwrap();
            std::fs::rename(&next, path).unwrap();
        };
        let upstreams = Upstreams::system(paths.clone(), Duration::from_secs(1));
        assert!(servers(&upstreams.current().await.unwrap().1).is_empty());
        replace(&paths.resolv, "nameserver 192.0.2.1\n");
        let (first, config) = upstreams.current().await.unwrap();
        assert_eq!(servers(&config), ["192.0.2.1".parse::<IpAddr>().unwrap()]);
        assert_eq!(upstreams.current().await.unwrap().0, first);
        // A new network's DHCP client replaces the file.
        replace(&paths.resolv, "nameserver 192.0.2.2\n");
        let (second, config) = upstreams.current().await.unwrap();
        assert_ne!(second, first);
        assert_eq!(servers(&config), ["192.0.2.2".parse::<IpAddr>().unwrap()]);
        // resolved's stub: its real upstream file is read, and followed.
        replace(&paths.stub, "nameserver 127.0.0.53\n");
        replace(&paths.upstream, "nameserver 198.51.100.1\n");
        std::fs::remove_file(&paths.resolv).unwrap();
        std::os::unix::fs::symlink(&paths.stub, &paths.resolv).unwrap();
        let config = upstreams.current().await.unwrap().1;
        assert_eq!(
            servers(&config),
            ["198.51.100.1".parse::<IpAddr>().unwrap()]
        );
        replace(&paths.upstream, "nameserver 198.51.100.2\n");
        let config = upstreams.current().await.unwrap().1;
        assert_eq!(
            servers(&config),
            ["198.51.100.2".parse::<IpAddr>().unwrap()]
        );
        // Any other local stub has no discoverable upstream.
        std::fs::remove_file(&paths.resolv).unwrap();
        replace(&paths.resolv, "nameserver 127.0.0.11\n");
        assert!(servers(&upstreams.current().await.unwrap().1).is_empty());
    }

    #[tokio::test]
    async fn every_lookup_sees_the_hosts_file_as_it_is_now() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hosts");
        let hosts = HostsFile::new(path.clone());
        assert!(addresses(&hosts, "server.example").await.is_empty());
        std::fs::write(&path, "192.0.2.1 server.example\n").unwrap();
        assert_eq!(
            addresses(&hosts, "server.example").await,
            ["192.0.2.1".parse::<IpAddr>().unwrap()]
        );
        // Same size, replaced atomically: a new inode and change time.
        let next = root.path().join("hosts.next");
        std::fs::write(&next, "192.0.2.2 server.example\n").unwrap();
        std::fs::rename(&next, &path).unwrap();
        assert_eq!(
            addresses(&hosts, "server.example").await,
            ["192.0.2.2".parse::<IpAddr>().unwrap()]
        );
        std::fs::remove_file(&path).unwrap();
        assert!(addresses(&hosts, "server.example").await.is_empty());
    }
}
