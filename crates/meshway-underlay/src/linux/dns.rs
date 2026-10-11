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

#[derive(Clone)]
pub(super) struct Resolver {
    inner: Option<HickoryResolver<Provider>>,
    hosts: Arc<HostsFile>,
    overrides: Arc<BTreeMap<String, Vec<IpAddr>>>,
    shared: Arc<Shared>,
    scope: ProtectionScope,
    config: Option<ResolverConfig>,
    resolver_options: ResolverOpts,
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
        let mut resolver_options = hickory_resolver::config::ResolverOpts::default();
        let config = if let Some(upstreams) = &options.bootstrap_nameservers {
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
            Some(ResolverConfig::from_parts(None, Vec::new(), servers))
        } else {
            let mut parsed = match read_bounded("/etc/resolv.conf").await {
                Ok(bytes) => hickory_resolver::system_conf::parse_resolv_conf(&bytes).ok(),
                Err(_) => None,
            };
            if parsed
                .as_ref()
                .is_some_and(|(cfg, _)| cfg.name_servers().iter().any(|s| s.ip.is_loopback()))
            {
                // Only the known systemd stub mapping has a discoverable real
                // upstream file. Docker and arbitrary local stubs stay unsupported.
                let known = tokio::fs::canonicalize("/etc/resolv.conf")
                    .await
                    .ok()
                    .is_some_and(|path| {
                        path == std::path::Path::new("/run/systemd/resolve/stub-resolv.conf")
                    });
                parsed = if known {
                    match read_bounded("/run/systemd/resolve/resolv.conf").await {
                        Ok(bytes) => hickory_resolver::system_conf::parse_resolv_conf(&bytes).ok(),
                        Err(_) => None,
                    }
                } else {
                    None
                };
            }
            match parsed {
                Some((cfg, opts))
                    if !cfg.name_servers().is_empty()
                        && cfg.name_servers().len() <= 8
                        && cfg.name_servers().iter().all(|s| {
                            !s.ip.is_loopback() && !s.ip.is_unspecified() && !s.ip.is_multicast()
                        }) =>
                {
                    resolver_options = opts;
                    Some(cfg)
                }
                _ => None,
            }
        };
        resolver_options.timeout = options.connect_timeout;
        resolver_options.attempts = 2;
        resolver_options.cache_size = 64;
        resolver_options.num_concurrent_reqs = 2;
        let hosts = Arc::new(hosts);
        let scope = shared.bootstrap();
        let inner = build(
            config.clone(),
            resolver_options.clone(),
            scope.clone(),
            options.connect_timeout,
        )?;
        Ok(Self {
            inner,
            hosts,
            overrides: Arc::new(overrides),
            shared,
            scope,
            config,
            resolver_options,
            timeout: options.connect_timeout,
        })
    }

    /// Candidate name resolution has that candidate's execution lifetime. It
    /// cannot borrow the Server bootstrap exemption after publication expiry.
    pub fn scoped(&self, scope: ProtectionScope) -> Result<Self> {
        scope.check()?;
        let inner = build(
            self.config.clone(),
            self.resolver_options.clone(),
            scope.clone(),
            self.timeout,
        )?;
        Ok(Self {
            inner,
            scope,
            ..self.clone()
        })
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
            let resolver = self.inner.as_ref().ok_or(Error::Resolver)?;
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

fn build(
    config: Option<ResolverConfig>,
    options: ResolverOpts,
    scope: ProtectionScope,
    timeout: Duration,
) -> Result<Option<HickoryResolver<Provider>>> {
    config
        .map(|config| {
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
            // System hosts are answered above from the current file only;
            // the upstream resolver never holds a copy that could go stale.
            resolver.set_hosts(Arc::new(Hosts::default()));
            Ok(resolver)
        })
        .transpose()
}

const HOSTS: &str = "/etc/hosts";

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
        use std::os::unix::fs::MetadataExt;
        let stamp = match tokio::fs::metadata(&self.path).await {
            Ok(meta) => Some(HostsStamp {
                device: meta.dev(),
                inode: meta.ino(),
                size: meta.size(),
                modified: (meta.mtime(), meta.mtime_nsec()),
                changed: (meta.ctime(), meta.ctime_nsec()),
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
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
