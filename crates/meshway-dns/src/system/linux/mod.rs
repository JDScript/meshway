mod file;
mod resolved;
mod storage;

use super::*;
use meshway_private_storage::Directory;
use std::os::unix::fs::MetadataExt;

/// The network namespace this owner runs in: boot ID, nsfs device/inode and
/// the namespace cookie, compared by the one rule of every namespace record
/// (`meshway_netns::same`, shared with the Underlay).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Namespace {
    boot: String,
    device: u64,
    inode: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cookie: Option<u64>,
}
impl Namespace {
    /// Whether `self` and `other` name the same namespace.
    fn same(&self, other: &Self) -> bool {
        meshway_netns::same(self.recorded(), other.recorded())
    }
    /// Whether a record written as `self` belongs to a namespace that no
    /// longer exists while `current` lives: the same boot and inode (its
    /// directory) under another cookie. A cookie is never reused within a
    /// boot, so the inode was reused by `current`.
    fn replaced_by(&self, current: &Self) -> bool {
        self.boot == current.boot
            && self.device == current.device
            && self.inode == current.inode
            && self
                .cookie
                .zip(current.cookie)
                .is_some_and(|(old, new)| old != new)
    }
    fn recorded(&self) -> meshway_netns::Recorded<'_> {
        meshway_netns::Recorded {
            boot: &self.boot,
            device: self.device,
            inode: self.inode,
            cookie: self.cookie,
        }
    }
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
            cookie: meshway_netns::cookie().map_err(|_| SystemDnsError::Unsupported)?,
        })
    }
    fn verify_pid(&self, pid: u32) -> Result<(), SystemDnsFailure> {
        let source = format!("/proc/{pid}/ns/net");
        let failure = |actual: String| {
            SystemDnsFailure::new(SystemDnsError::Conflict, "resolved_namespace")
                .expected(format!("resolved (pid {pid}) in netns {self}"))
                .actual(actual)
                .source(&source)
        };
        let meta = std::fs::metadata(&source).map_err(|error| failure(error.to_string()))?;
        if meta.dev() != self.device || meta.ino() != self.inode {
            return Err(failure(format!("netns {}:{}", meta.dev(), meta.ino())));
        }
        self.check_current()
    }
    /// This process still runs in the recorded namespace.
    fn check_current(&self) -> Result<(), SystemDnsFailure> {
        let current = Self::current().at("namespace")?;
        if !self.same(&current) {
            return Err(SystemDnsFailure::new(SystemDnsError::Conflict, "namespace")
                .expected(self)
                .actual(&current)
                .source("/proc/self/ns/net, SO_NETNS_COOKIE, boot_id"));
        }
        Ok(())
    }
}
impl std::fmt::Display for Namespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.device, self.inode)?;
        if let Some(cookie) = self.cookie {
            write!(f, " cookie {cookie}")?;
        }
        write!(f, " boot {}", self.boot)
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
impl Reservation {
    /// The same owner in the same namespace ([`Namespace::same`]); a record
    /// from an earlier build without the cookie is then rewritten with it.
    fn same_owner(&self, other: &Self) -> bool {
        self.version == other.version
            && self.data_owner == other.data_owner
            && self.settings == other.settings
            && self.namespace.same(&other.namespace)
    }
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
            self.reservation.namespace.cookie,
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
    /// `(boot, device, inode, cookie)` is the caller's record of its
    /// namespace (the Underlay journal), compared by the same rule.
    pub fn verify_namespace(
        &self,
        boot: &str,
        device: u64,
        inode: u64,
        cookie: Option<u64>,
    ) -> Result<(), SystemDnsError> {
        let namespace = &self.reservation.namespace;
        let recorded = Namespace {
            boot: boot.trim().to_owned(),
            device,
            inode,
            cookie,
        };
        if !namespace.same(&recorded) || !namespace.same(&Namespace::current()?) {
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
    /// the installed link, the stub and resolved's OS server inventory
    /// permit it, or why not. Its lapse withdraws only served queries, never
    /// the owner.
    served: Result<SystemResolverSnapshot, ExitDnsUnavailableReason>,
    writes: storage::Writes,
    invalid: CancellationToken,
    /// Why the last failed operation failed; cleared by the next success.
    failure: Option<SystemDnsFailure>,
}
/// The records of one namespace's DNS ownership in its directory.
const RECORDS: [&str; 3] = ["owner.json", "resolv-conf.json", "resolved.json"];

/// Make `reservation` the owner of `directory`, held under its lock (another
/// live process holding it never gets here). A reservation of a namespace
/// that no longer exists ([`Namespace::replaced_by`]) is archived with every
/// backend record beside it, as the Underlay archives its journals, and this
/// one takes over; the same owner in the same namespace (by cookie, or by
/// inode for a record written without one) is reused and rewritten with the
/// cookie; anything else is a conflict.
fn take_over(directory: &Directory, reservation: &Reservation) -> Result<(), SystemDnsError> {
    let mut existing: Option<Reservation> = storage::read(directory, "owner.json")?;
    if let Some(old) = &existing
        && old.namespace.replaced_by(&reservation.namespace)
    {
        let names = directory.names().map_err(|_| SystemDnsError::Storage)?;
        for record in RECORDS {
            if names.iter().any(|name| name == record) {
                let archive = directory
                    .archive_stale(record, meshway_private_storage::STALE_ARCHIVES)
                    .map_err(|_| SystemDnsError::Storage)?;
                tracing::info!(record, archive, old_cookie = ?old.namespace.cookie,
                    cookie = ?reservation.namespace.cookie,
                    "Archived the DNS record of another network namespace");
            }
        }
        existing = None;
    }
    if existing.is_none() && (file::has_record(directory)? || resolved::has_record(directory)?) {
        return Err(SystemDnsError::Conflict);
    }
    if existing
        .as_ref()
        .is_some_and(|old| !old.same_owner(reservation))
    {
        return Err(SystemDnsError::Conflict);
    }
    // This accepted private write is completed by its blocking owner even if
    // prepare is cancelled; it changes no host DNS configuration.
    storage::save(directory, "owner.json", &Some(reservation))
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
                // A destroyed namespace's records: nothing of this one to undo.
                Some(reservation) if reservation.namespace.replaced_by(&namespace) => Ok(false),
                Some(reservation) => {
                    reservation.settings.validate()?;
                    if reservation.version != 1
                        || !reservation.namespace.same(&namespace)
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
        Self::prepare_reserved(reservation, anchor)
            .await
            .map_err(|failure| failure.error)
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
            take_over(&directory, &reservation)?;
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
    ) -> Result<Self, SystemDnsFailure> {
        let SystemDnsReservation {
            directory,
            reservation,
            mut writes,
        } = reserved;
        writes.settle().await.at("owner_record")?;
        let source = directory.path().join("owner.json");
        if storage::read::<Reservation>(&directory, "owner.json")
            .at("owner_record")?
            .as_ref()
            != Some(&reservation)
        {
            return Err(
                SystemDnsFailure::new(SystemDnsError::Conflict, "owner_record")
                    .expected("the reservation this owner made")
                    .actual("another or no reservation")
                    .source(source.display()),
            );
        }
        let (backend, snapshot, invalid) = Self::open(&directory, &reservation, anchor).await?;
        Ok(Self {
            backend,
            directory,
            reservation,
            snapshot,
            served: Err(ExitDnsUnavailableReason::Unobserved),
            writes,
            invalid,
            failure: None,
        })
    }
    /// Why the last failed observation, recovery, installation or down
    /// failed, until the next one succeeds.
    pub fn failure(&self) -> Option<&SystemDnsFailure> {
        self.failure.as_ref()
    }
    fn outcome(&mut self, result: Result<(), SystemDnsFailure>) -> Result<(), SystemDnsError> {
        match result {
            Ok(()) => {
                self.failure = None;
                Ok(())
            }
            Err(failure) => {
                let error = failure.error;
                self.failure = Some(failure);
                Err(error)
            }
        }
    }

    async fn open(
        directory: &Arc<Directory>,
        reservation: &Reservation,
        anchor: Option<Arc<dyn OwnedDnsLink>>,
    ) -> Result<(Backend, Option<SystemResolverSnapshot>, CancellationToken), SystemDnsFailure>
    {
        let settings = &reservation.settings;
        let invalid = CancellationToken::new();
        let (backend, snapshot) = match &settings.backend {
            SystemDnsBackend::OwnedResolvConf { path } => {
                let dir = directory.clone();
                let path = path.clone();
                let bind = settings.bind;
                let source = path.display().to_string();
                let file = async {
                    let backend = tokio::task::spawn_blocking(move || {
                        file::Backend::prepare(dir, path, bind)
                    })
                    .await
                    .map_err(|_| SystemDnsError::Storage)??;
                    Ok::<_, SystemDnsError>((
                        backend.snapshot_id()?,
                        backend.upstreams()?,
                        backend.transport()?,
                        backend,
                    ))
                };
                let (id, upstreams, primary, backend) = file
                    .await
                    .at("resolv_conf")
                    .map_err(|failure| failure.source(source))?;
                let snapshot = SystemResolverSnapshot {
                    id,
                    upstreams: Arc::new(upstreams),
                    primary,
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
                        anchor.ok_or(SystemDnsError::Settings).at("anchor")?,
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
        let result = self.reprove().await;
        self.outcome(result)
    }
    async fn reprove(&mut self) -> Result<(), SystemDnsFailure> {
        self.invalid.cancel();
        self.writes.settle().await.at("record_write")?;
        let anchor = match &mut self.backend {
            Backend::File(file) => {
                file.settle().await.at("record_write")?;
                None
            }
            Backend::Resolved(resolved) => {
                resolved.settle().await.at("record_write")?;
                Some(resolved.anchor())
            }
        };
        self.reservation.namespace.check_current()?;
        self.check_reservation()?;
        let (backend, snapshot, invalid) =
            Self::open(&self.directory, &self.reservation, anchor).await?;
        self.backend = backend;
        self.snapshot = snapshot;
        self.served = Err(ExitDnsUnavailableReason::Unobserved);
        self.invalid = invalid;
        Ok(())
    }
    /// The namespace's ownership record is still this owner's.
    fn check_reservation(&self) -> Result<(), SystemDnsFailure> {
        let source = self.directory.path().join("owner.json");
        let recorded = storage::read::<Reservation>(&self.directory, "owner.json")
            .at("owner_record")
            .map_err(|failure| failure.source(source.display()))?;
        if recorded.as_ref() != Some(&self.reservation) {
            return Err(
                SystemDnsFailure::new(SystemDnsError::Conflict, "owner_record")
                    .expected(format!(
                        "this owner ({}) in netns {}",
                        self.reservation.data_owner.display(),
                        self.reservation.namespace
                    ))
                    .actual(match &recorded {
                        None => "no reservation".to_owned(),
                        Some(other) => format!(
                            "owner {} in netns {}",
                            other.data_owner.display(),
                            other.namespace
                        ),
                    })
                    .source(source.display()),
            );
        }
        Ok(())
    }

    /// OS upstreams for this Node's own unmanaged names (file backend only;
    /// resolved routes those itself).
    pub fn system_resolvers(&self) -> Option<SystemResolverSnapshot> {
        self.snapshot.clone()
    }

    /// OS upstreams for an exit resolver's served queries: the recorded
    /// original nameservers, or resolved's proxy stub while it is usable;
    /// otherwise why not. [`crate::exit_resolvers`] decides them on the netmap.
    pub fn served_resolvers(&self) -> Result<SystemResolverSnapshot, ExitDnsUnavailableReason> {
        let served = match &self.backend {
            Backend::File(_) => self
                .snapshot
                .clone()
                .ok_or(ExitDnsUnavailableReason::Unobserved),
            Backend::Resolved(_) => self.served.clone(),
        }?;
        served
            .ensure_current()
            .map_err(|_| ExitDnsUnavailableReason::Unobserved)?;
        Ok(served)
    }

    pub async fn install(&mut self, plan: &SystemDnsPlan) -> Result<(), SystemDnsError> {
        let result = async {
            plan.validate().at("plan")?;
            self.checked_observe().await?;
            let result = match &mut self.backend {
                Backend::File(file) => file.install(plan).await.at("resolv_conf"),
                Backend::Resolved(resolved) => resolved.install(plan).await,
            };
            if result.is_err() {
                self.invalid.cancel();
            }
            result?;
            self.checked_observe().await
        }
        .await;
        self.outcome(result)
    }

    /// Keep, replace or withdraw the resolved proxy snapshot after a
    /// successful observation. The same inventory keeps the same snapshot.
    fn refresh_served(&mut self, proxy: Result<resolved::Proxy, ExitDnsUnavailableReason>) {
        let current = self.served.as_ref().ok().filter(|served| {
            served.ensure_current().is_ok()
                && match (&served.provenance, &proxy) {
                    (Provenance::ResolvedProxy { routes, servers }, Ok(proxy)) => {
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
        if let Ok(old) = &self.served {
            old.invalid.cancel();
        }
        let proxy = match proxy {
            Ok(proxy) => proxy,
            Err(reason) => {
                if self.served.as_ref().err() != Some(&reason) {
                    tracing::debug!(
                        ?reason,
                        "resolved proxy unavailable for served exit queries"
                    );
                }
                self.served = Err(reason);
                return;
            }
        };
        tracing::debug!(
            servers = proxy.servers.len(),
            routes = proxy.routes.len(),
            "resolved proxy serves exit queries"
        );
        let mut nonce = [0u8; 8];
        if getrandom::fill(&mut nonce).is_err() {
            self.served = Err(ExitDnsUnavailableReason::Unobserved);
            return;
        }
        self.served = Ok(SystemResolverSnapshot {
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
        let result = self.checked_observe().await;
        self.outcome(result)
    }
    async fn checked_observe(&mut self) -> Result<(), SystemDnsFailure> {
        let result = async {
            self.writes.settle().await.at("record_write")?;
            if self.invalid.is_cancelled() {
                return Err(
                    SystemDnsFailure::new(SystemDnsError::Conflict, "owner_fenced")
                        .expected("no failed operation since the last re-proof")
                        .actual("an earlier failure fenced this owner until it recovers"),
                );
            }
            self.reservation.namespace.check_current()?;
            if let Some(snapshot) = &self.snapshot {
                snapshot.ensure_current().map_err(|error| {
                    SystemDnsFailure::new(error, "resolver_snapshot")
                        .expected("the recorded OS resolvers re-observed within 2 s")
                        .actual("expired or fenced")
                })?;
            }
            match &mut self.backend {
                Backend::File(file) => file.observe().await.at("resolv_conf").map(|()| None),
                Backend::Resolved(resolved) => {
                    resolved.observe().await?;
                    // An unusable inventory withdraws only served queries.
                    Ok(Some(resolved.proxy().await))
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
            self.served = Err(ExitDnsUnavailableReason::Unobserved);
        } else if let Some(snapshot) = &self.snapshot {
            snapshot.ensure_current().at("resolver_snapshot")?;
            *snapshot.verified_until.lock().map_err(|_| {
                SystemDnsFailure::new(SystemDnsError::Conflict, "resolver_snapshot")
            })? = Instant::now() + std::time::Duration::from_secs(2);
        }
        result
    }

    pub async fn down(&mut self) -> Result<(), SystemDnsError> {
        self.invalid.cancel();
        let result = match &mut self.backend {
            Backend::File(file) => file.down().await.at("resolv_conf"),
            Backend::Resolved(resolved) => resolved.down().await,
        };
        self.outcome(result)?;
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

#[cfg(test)]
mod namespace_tests {
    use super::{Directory, Namespace, Reservation, SystemDnsError, storage, take_over};

    fn reservation(inode: u64, cookie: Option<u64>) -> Reservation {
        Reservation {
            version: 1,
            namespace: Namespace {
                boot: "b".into(),
                device: 5,
                inode,
                cookie,
            },
            data_owner: "/var/lib/meshway".into(),
            settings: Default::default(),
        }
    }

    fn owner(directory: &Directory) -> Option<Reservation> {
        storage::read(directory, "owner.json").unwrap()
    }

    fn archives(directory: &Directory, record: &str) -> usize {
        let prefix = format!("{record}.stale-");
        directory
            .names()
            .unwrap()
            .iter()
            .filter(|name| name.starts_with(&prefix))
            .count()
    }

    /// The same inode under another cookie is a destroyed namespace: its
    /// reservation and backend records are archived (the newest three kept,
    /// as the Underlay keeps its journals) and the current one takes over.
    #[test]
    fn a_destroyed_namespaces_records_are_archived_and_taken_over() {
        let temp = tempfile::tempdir().unwrap();
        let directory = Directory::open(&temp.path().join("dns"), "dns.lock").unwrap();
        storage::save(&directory, "owner.json", &Some(&reservation(7, Some(8)))).unwrap();
        directory.replace("resolved.json", b"{}").unwrap();
        take_over(&directory, &reservation(7, Some(9))).unwrap();
        assert!(owner(&directory) == Some(reservation(7, Some(9))));
        assert!(
            !directory
                .names()
                .unwrap()
                .iter()
                .any(|name| name == "resolved.json")
        );
        assert_eq!(archives(&directory, "owner.json"), 1);
        assert_eq!(archives(&directory, "resolved.json"), 1);
        for cookie in 10..15 {
            directory.replace("resolv-conf.json", b"{}").unwrap();
            take_over(&directory, &reservation(7, Some(cookie))).unwrap();
            assert!(owner(&directory) == Some(reservation(7, Some(cookie))));
        }
        assert_eq!(archives(&directory, "owner.json"), 3);
        assert_eq!(archives(&directory, "resolv-conf.json"), 3);
    }

    /// Only the same namespace is the same owner: the same cookie with
    /// another owner is a conflict, and so is a live process holding the
    /// namespace's lock (it never reaches the records).
    #[test]
    fn the_same_namespace_with_another_owner_or_a_live_holder_conflicts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dns");
        let directory = Directory::open(&path, "dns.lock").unwrap();
        storage::save(&directory, "owner.json", &Some(&reservation(7, Some(9)))).unwrap();
        take_over(&directory, &reservation(7, Some(9))).unwrap();
        let mut other = reservation(7, Some(9));
        other.data_owner = "/srv/other".into();
        assert!(matches!(
            take_over(&directory, &other),
            Err(SystemDnsError::Conflict)
        ));
        assert!(owner(&directory) == Some(reservation(7, Some(9))));
        assert_eq!(archives(&directory, "owner.json"), 0);
        assert!(
            Directory::open(&path, "dns.lock").is_err(),
            "the live holder's lock"
        );
        // A backend record without any reservation is never adopted.
        let bare = Directory::open(&temp.path().join("bare"), "dns.lock").unwrap();
        bare.replace("resolved.json", b"{}").unwrap();
        assert!(take_over(&bare, &reservation(7, Some(9))).is_err());
        assert!(owner(&bare).is_none());
    }

    /// A reservation an earlier build wrote without a cookie keeps the inode
    /// rule: the same inode is taken over and rewritten with the cookie.
    #[test]
    fn a_reservation_without_a_cookie_follows_the_inode() {
        let temp = tempfile::tempdir().unwrap();
        let directory = Directory::open(&temp.path().join("dns"), "dns.lock").unwrap();
        storage::save(&directory, "owner.json", &Some(&reservation(7, None))).unwrap();
        take_over(&directory, &reservation(7, Some(9))).unwrap();
        assert!(owner(&directory) == Some(reservation(7, Some(9))));
        assert_eq!(archives(&directory, "owner.json"), 0);
        storage::save(&directory, "owner.json", &Some(&reservation(6, None))).unwrap();
        assert!(matches!(
            take_over(&directory, &reservation(7, Some(9))),
            Err(SystemDnsError::Conflict)
        ));
    }

    /// A reused inode with another cookie is another namespace; a record of
    /// an earlier build (no cookie) or a kernel without cookies is compared
    /// by the inode.
    #[test]
    fn namespaces_are_compared_by_cookie_when_both_carry_one() {
        let at = |inode, cookie| Namespace {
            boot: "b".into(),
            device: 5,
            inode,
            cookie,
        };
        assert!(at(7, Some(9)).same(&at(7, Some(9))));
        assert!(!at(7, Some(9)).same(&at(7, Some(8))), "reused inode");
        assert!(!at(7, Some(9)).same(&at(8, Some(9))));
        assert!(at(7, None).same(&at(7, Some(9))));
        assert!(at(7, Some(9)).same(&at(7, None)));
        assert!(!at(7, None).same(&at(8, None)));
        // An earlier build's record decodes without the field and writes none.
        let old: Namespace = serde_json::from_str(r#"{"boot":"b","device":5,"inode":7}"#).unwrap();
        assert_eq!(old, at(7, None));
        assert!(!serde_json::to_string(&old).unwrap().contains("cookie"));
        let current = Namespace::current().unwrap();
        assert!(current.cookie.is_some(), "Linux 5.7 or later");
        assert!(current.same(&Namespace::current().unwrap()));
    }
}
