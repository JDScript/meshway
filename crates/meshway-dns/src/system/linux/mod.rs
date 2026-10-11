mod file;
mod resolved;
mod storage;

use super::*;
use meshway_private_storage::Directory;
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Namespace {
    boot: String,
    device: u64,
    inode: u64,
}
impl Namespace {
    fn directory(&self) -> PathBuf {
        PathBuf::from("/run/meshway")
            .join(format!("dns-{}-{}-{}", self.boot, self.device, self.inode))
    }
    fn current() -> Result<Self, SystemDnsError> {
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|_| SystemDnsError::Unsupported)?;
        let boot = boot.trim().to_owned();
        if boot.len() != 36 || !boot.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            return Err(SystemDnsError::Unsupported);
        }
        let meta =
            std::fs::metadata("/proc/self/ns/net").map_err(|_| SystemDnsError::Unsupported)?;
        Ok(Self {
            boot,
            device: meta.dev(),
            inode: meta.ino(),
        })
    }
    fn verify_pid(&self, pid: u32) -> Result<(), SystemDnsError> {
        let meta = std::fs::metadata(format!("/proc/{pid}/ns/net"))
            .map_err(|_| SystemDnsError::Conflict)?;
        if meta.dev() != self.device || meta.ino() != self.inode || *self != Self::current()? {
            return Err(SystemDnsError::Conflict);
        }
        Ok(())
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reservation {
    version: u32,
    namespace: Namespace,
    data_owner: PathBuf,
    settings: SystemDnsSettings,
}

enum Backend {
    File(file::Backend),
    Resolved(Box<resolved::Backend>),
}

/// Namespace-wide reservation acquired before the Kernel creates an anchor.
/// Dropping it releases the live lock, not the kept private ownership record.
pub struct SystemDnsReservation {
    directory: Arc<Directory>,
    reservation: Reservation,
    writes: storage::Writes,
}
impl SystemDnsReservation {
    pub fn has_backend_record(&self) -> Result<bool, SystemDnsError> {
        self.verify_namespace(
            &self.reservation.namespace.boot,
            self.reservation.namespace.device,
            self.reservation.namespace.inode,
        )?;
        Ok(file::has_record(&self.directory)? || resolved::has_record(&self.directory)?)
    }
    /// Explicit-down cleanup for preparation that never installed DNS. Parent
    /// first removes its unconfigured Kernel anchor. Any backend receipt makes
    /// this refuse cleanup; those resources require their real backend owner.
    pub async fn release_uninstalled(&mut self) -> Result<(), SystemDnsError> {
        let directory = self.directory.clone();
        self.writes
            .run(move || {
                if file::has_record(&directory)? || resolved::has_record(&directory)? {
                    return Err(SystemDnsError::Conflict);
                }
                storage::save(&directory, "owner.json", &Option::<Reservation>::None)
            })
            .await
    }
    pub fn settings(&self) -> &SystemDnsSettings {
        &self.reservation.settings
    }
    /// Read-only recovery input for the host's existing fixed-origin bootstrap
    /// resolver, before Host::open. This returns configuration, never a socket
    /// grant or a business query API. Explicit operator bootstrap settings take
    /// precedence. Resolved's DefaultRoute=false link is excluded from its
    /// generated uplink resolv.conf, so its existing reader remains applicable.
    pub async fn bootstrap_nameservers(&self) -> Result<Option<Vec<SocketAddr>>, SystemDnsError> {
        let SystemDnsBackend::OwnedResolvConf { path } = &self.reservation.settings.backend else {
            return Ok(None);
        };
        let directory = self.directory.clone();
        let path = path.clone();
        let bind = self.reservation.settings.bind;
        tokio::task::spawn_blocking(move || {
            file::Backend::prepare(directory, path, bind)
                .and_then(|backend| backend.upstreams())
                .map(Some)
        })
        .await
        .map_err(|_| SystemDnsError::Storage)?
    }
    /// The Kernel consumes the live reservation, not a caller-supplied boolean.
    /// This is only local resource ownership, never execution authority.
    pub fn verify_namespace(
        &self,
        boot: &str,
        device: u64,
        inode: u64,
    ) -> Result<(), SystemDnsError> {
        let namespace = &self.reservation.namespace;
        if namespace.boot != boot.trim()
            || namespace.device != device
            || namespace.inode != inode
            || *namespace != Namespace::current()?
        {
            return Err(SystemDnsError::Conflict);
        }
        if storage::read::<Reservation>(&self.directory, "owner.json")?.as_ref()
            != Some(&self.reservation)
        {
            return Err(SystemDnsError::Conflict);
        }
        Ok(())
    }
}

/// Exactly one owner per boot/network namespace, across all daemon data dirs.
/// Call observe periodically; failure fences captured upstream provenance.
pub struct SystemDnsOwner {
    backend: Backend,
    directory: Arc<Directory>,
    reservation: Reservation,
    snapshot: Option<SystemResolverSnapshot>,
    /// resolved's proxy stub for an exit resolver's served queries, while
    /// the installed link and resolved's OS server inventory permit it. Its
    /// lapse withdraws only served queries, never the owner.
    served: Option<SystemResolverSnapshot>,
    writes: storage::Writes,
    invalid: CancellationToken,
}
impl SystemDnsOwner {
    /// Read the namespace's existing ownership record under its existing lock.
    /// An unused DNS backend creates no /run paths as a side effect of down.
    pub async fn has_reservation(data: Arc<Directory>) -> Result<bool, SystemDnsError> {
        tokio::task::spawn_blocking(move || {
            let namespace = Namespace::current()?;
            let Some(directory) = Directory::open_existing(&namespace.directory(), "dns.lock")
                .map_err(|_| SystemDnsError::Conflict)?
            else {
                return Ok(false);
            };
            match storage::read::<Reservation>(&directory, "owner.json")? {
                Some(reservation) => {
                    reservation.settings.validate()?;
                    if reservation.version != 1
                        || reservation.namespace != namespace
                        || reservation.data_owner != data.path()
                    {
                        return Err(SystemDnsError::Conflict);
                    }
                    Ok(true)
                }
                None if file::has_record(&directory)? || resolved::has_record(&directory)? => {
                    Err(SystemDnsError::Conflict)
                }
                None => Ok(false),
            }
        })
        .await
        .map_err(|_| SystemDnsError::Storage)?
    }
    pub async fn prepare(
        settings: SystemDnsSettings,
        data: Arc<Directory>,
        anchor: Option<Arc<dyn OwnedDnsLink>>,
    ) -> Result<Self, SystemDnsError> {
        let reservation = Self::reserve(settings, data).await?;
        Self::prepare_reserved(reservation, anchor).await
    }

    pub async fn reserve(
        settings: SystemDnsSettings,
        data: Arc<Directory>,
    ) -> Result<SystemDnsReservation, SystemDnsError> {
        settings.validate()?;
        let prepared_settings = settings.clone();
        let (directory, reservation) = tokio::task::spawn_blocking(move || {
            let namespace = Namespace::current()?;
            // No configurable lock root: two data dirs cannot split ownership.
            let _root = Directory::open(std::path::Path::new("/run/meshway"), "namespaces.lock")
                .map_err(|_| SystemDnsError::Conflict)?;
            let path = namespace.directory();
            let directory =
                Arc::new(Directory::open(&path, "dns.lock").map_err(|_| SystemDnsError::Conflict)?);
            let reservation = Reservation {
                version: 1,
                namespace,
                data_owner: data.path().to_owned(),
                settings: prepared_settings,
            };
            let existing: Option<Reservation> = storage::read(&directory, "owner.json")?;
            if existing.is_none()
                && (file::has_record(&directory)? || resolved::has_record(&directory)?)
            {
                return Err(SystemDnsError::Conflict);
            }
            if existing.as_ref().is_some_and(|old| old != &reservation) {
                return Err(SystemDnsError::Conflict);
            }
            // This accepted private write is completed by its blocking owner even
            // if prepare is cancelled; it changes no host DNS configuration.
            storage::save(&directory, "owner.json", &Some(&reservation))?;
            Ok::<_, SystemDnsError>((directory, reservation))
        })
        .await
        .map_err(|_| SystemDnsError::Storage)??;
        Ok(SystemDnsReservation {
            directory,
            reservation,
            writes: storage::Writes::default(),
        })
    }

    pub async fn prepare_reserved(
        reserved: SystemDnsReservation,
        anchor: Option<Arc<dyn OwnedDnsLink>>,
    ) -> Result<Self, SystemDnsError> {
        let SystemDnsReservation {
            directory,
            reservation,
            mut writes,
        } = reserved;
        writes.settle().await?;
        if storage::read::<Reservation>(&directory, "owner.json")?.as_ref() != Some(&reservation) {
            return Err(SystemDnsError::Conflict);
        }
        let (backend, snapshot, invalid) = Self::open(&directory, &reservation, anchor).await?;
        Ok(Self {
            backend,
            directory,
            reservation,
            snapshot,
            served: None,
            writes,
            invalid,
        })
    }

    async fn open(
        directory: &Arc<Directory>,
        reservation: &Reservation,
        anchor: Option<Arc<dyn OwnedDnsLink>>,
    ) -> Result<(Backend, Option<SystemResolverSnapshot>, CancellationToken), SystemDnsError> {
        let settings = &reservation.settings;
        let invalid = CancellationToken::new();
        let (backend, snapshot) = match &settings.backend {
            SystemDnsBackend::OwnedResolvConf { path } => {
                let dir = directory.clone();
                let path = path.clone();
                let bind = settings.bind;
                let backend =
                    tokio::task::spawn_blocking(move || file::Backend::prepare(dir, path, bind))
                        .await
                        .map_err(|_| SystemDnsError::Storage)??;
                let snapshot = SystemResolverSnapshot {
                    id: backend.snapshot_id()?,
                    upstreams: Arc::new(backend.upstreams()?),
                    primary: backend.transport()?,
                    provenance: Provenance::Original,
                    invalid: invalid.clone(),
                    verified_until: Arc::new(Mutex::new(
                        Instant::now() + std::time::Duration::from_secs(2),
                    )),
                };
                (Backend::File(backend), Some(snapshot))
            }
            SystemDnsBackend::Resolved => (
                Backend::Resolved(Box::new(
                    resolved::Backend::prepare(
                        directory.clone(),
                        reservation.namespace.clone(),
                        settings.bind,
                        anchor.ok_or(SystemDnsError::Settings)?,
                    )
                    .await?,
                )),
                None,
            ),
        };
        Ok((backend, snapshot, invalid))
    }

    /// Re-prove ownership after a failed observation, against the backend as
    /// it is now (for example a restarted systemd-resolved), with the same
    /// namespace reservation and Kernel anchor. Snapshots and query provenance
    /// of the failed observation stay fenced; success yields fresh ones and
    /// leaves the backend either at its proven installed settings or at its
    /// original settings, which the caller's next install replaces.
    pub async fn recover(&mut self) -> Result<(), SystemDnsError> {
        self.invalid.cancel();
        self.writes.settle().await?;
        let anchor = match &mut self.backend {
            Backend::File(file) => {
                file.settle().await?;
                None
            }
            Backend::Resolved(resolved) => {
                resolved.settle().await?;
                Some(resolved.anchor())
            }
        };
        if Namespace::current()? != self.reservation.namespace
            || storage::read::<Reservation>(&self.directory, "owner.json")?.as_ref()
                != Some(&self.reservation)
        {
            return Err(SystemDnsError::Conflict);
        }
        let (backend, snapshot, invalid) =
            Self::open(&self.directory, &self.reservation, anchor).await?;
        self.backend = backend;
        self.snapshot = snapshot;
        self.served = None;
        self.invalid = invalid;
        Ok(())
    }

    /// OS upstreams for this Node's own unmanaged names (file backend only;
    /// resolved routes those itself).
    pub fn system_resolvers(&self) -> Option<SystemResolverSnapshot> {
        self.snapshot.clone()
    }

    /// OS upstreams for an exit resolver's served queries: the recorded
    /// original nameservers, or resolved's proxy stub while it is usable.
    pub fn served_resolvers(&self) -> Option<SystemResolverSnapshot> {
        match &self.backend {
            Backend::File(_) => self.snapshot.clone(),
            Backend::Resolved(_) => self
                .served
                .clone()
                .filter(|served| served.ensure_current().is_ok()),
        }
    }

    pub async fn install(&mut self, plan: &SystemDnsPlan) -> Result<(), SystemDnsError> {
        plan.validate()?;
        self.observe().await?;
        let result = match &mut self.backend {
            Backend::File(file) => file.install(plan).await,
            Backend::Resolved(resolved) => resolved.install(plan).await,
        };
        if result.is_err() {
            self.invalid.cancel();
        }
        result?;
        self.observe().await
    }

    /// Keep, replace or withdraw the resolved proxy snapshot after a
    /// successful observation. The same inventory keeps the same snapshot.
    fn refresh_served(&mut self, proxy: Option<resolved::Proxy>) {
        let current = self.served.as_ref().filter(|served| {
            served.ensure_current().is_ok()
                && match (&served.provenance, &proxy) {
                    (Provenance::ResolvedProxy { routes, servers }, Some(proxy)) => {
                        **routes == proxy.routes && **servers == proxy.servers
                    }
                    _ => false,
                }
        });
        if let Some(served) = current {
            if let Ok(mut until) = served.verified_until.lock() {
                *until = Instant::now() + std::time::Duration::from_secs(2);
            }
            return;
        }
        if let Some(old) = self.served.take() {
            old.invalid.cancel();
            if proxy.is_none() {
                tracing::debug!("resolved proxy withdrawn from served exit queries");
            }
        }
        let Some(proxy) = proxy else {
            return;
        };
        tracing::debug!(
            servers = proxy.servers.len(),
            routes = proxy.routes.len(),
            "resolved proxy serves exit queries"
        );
        let mut nonce = [0u8; 8];
        if getrandom::fill(&mut nonce).is_err() {
            return;
        }
        self.served = Some(SystemResolverSnapshot {
            id: format!("resolved-proxy-{:016x}", u64::from_be_bytes(nonce)),
            upstreams: Arc::new(vec![resolved::PROXY_STUB]),
            primary: DnsTransport::Udp,
            provenance: Provenance::ResolvedProxy {
                routes: Arc::new(proxy.routes),
                servers: Arc::new(proxy.servers),
            },
            invalid: self.invalid.child_token(),
            verified_until: Arc::new(Mutex::new(
                Instant::now() + std::time::Duration::from_secs(2),
            )),
        });
    }

    pub async fn observe(&mut self) -> Result<(), SystemDnsError> {
        let result = async {
            self.writes.settle().await?;
            if self.invalid.is_cancelled() || Namespace::current()? != self.reservation.namespace {
                return Err(SystemDnsError::Conflict);
            }
            if let Some(snapshot) = &self.snapshot {
                snapshot.ensure_current()?;
            }
            match &mut self.backend {
                Backend::File(file) => file.observe().await.map(|()| None),
                Backend::Resolved(resolved) => {
                    resolved.observe().await?;
                    // An unusable inventory withdraws only served queries.
                    Ok(Some(resolved.proxy().await.ok().flatten()))
                }
            }
        }
        .await;
        let result = match result {
            Ok(Some(proxy)) => {
                self.refresh_served(proxy);
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(error) => Err(error),
        };
        if result.is_err() {
            self.invalid.cancel();
            self.served = None;
        } else if let Some(snapshot) = &self.snapshot {
            snapshot.ensure_current()?;
            *snapshot
                .verified_until
                .lock()
                .map_err(|_| SystemDnsError::Conflict)? =
                Instant::now() + std::time::Duration::from_secs(2);
        }
        result
    }

    pub async fn down(&mut self) -> Result<(), SystemDnsError> {
        self.invalid.cancel();
        match &mut self.backend {
            Backend::File(file) => file.down().await?,
            Backend::Resolved(resolved) => resolved.down().await?,
        }
        self.writes
            .save(
                self.directory.clone(),
                "owner.json",
                &Option::<Reservation>::None,
            )
            .await
    }

    pub async fn shutdown(&mut self, keep: bool) -> Result<(), SystemDnsError> {
        self.invalid.cancel();
        let result = if keep {
            match &mut self.backend {
                Backend::File(file) => file.settle().await,
                Backend::Resolved(resolved) => resolved.settle().await,
            }
        } else {
            self.down().await
        };
        self.writes.settle().await?;
        result
    }
}
impl Drop for SystemDnsOwner {
    fn drop(&mut self) {
        self.invalid.cancel();
    }
}
