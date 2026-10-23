use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::accounts::Accounts;
use crate::directory::{Directory, DirectoryError};
use crate::networks::Networks;
use crate::secrets::{self, KeyError, Sealer};
use crate::settings::Settings;
use crate::store::{Executor, ExecutorError, OpenError, Options, schema};

pub(crate) mod startup;

const MAX_DIRECTORY_PREPARATIONS: u32 = 4;
static DIRECTORY_PREPARATIONS: Semaphore =
    Semaphore::const_new(MAX_DIRECTORY_PREPARATIONS as usize);

/// Application lifecycle owner. Dropping closes work admission immediately;
/// the SQLite worker retains its process lock until accepted work has drained
/// and the connection has closed. Use [`Self::shutdown`] to await that boundary.
pub struct App {
    stun: crate::stun::Hosted,
    pub(crate) stun_servers:
        Arc<std::collections::BTreeMap<String, meshway_transport::spec::Endpoint>>,
    resource_scheduler: crate::resources::scheduler::Scheduler,
    observation_maintenance: std::sync::Mutex<crate::observations::service::MaintenanceOwner>,
    traversal_maintenance: crate::connectivity::maintenance::Maintenance,
    downloads: Arc<crate::resources::downloads::Downloads>,
    executor: Executor,
    accounts: Accounts,
    sealer: Arc<Sealer>,
    committer: crate::configuration::Committer,
    boot: meshway_model::InstanceId,
    state_runtime: Arc<std::sync::Mutex<crate::state::service::Runtime>>,
    /// Sole audience of committed effects; the executor holds it weakly.
    effects: Arc<Effects>,
    netmap_timing: crate::netmap::SyncTiming,
    peer_facts: Arc<dyn crate::netmap::PeerFactsSource>,
    netmap_signer: Arc<crate::netmap::signing::NetmapSigningKeys>,
    /// Process-local trusted DNS associations (see `netmap::associations`).
    dns_associations: crate::netmap::associations::Shared,
    /// The production stored-netmap reader shared by State, observations and coordination.
    netmaps: Arc<crate::store::netmap::LateNetmaps>,
    shutdown_hooks: ShutdownHooks,
    console_origin: Option<Arc<crate::settings::Url>>,
    public_origin: Option<Arc<crate::settings::Url>>,
    migrated: Option<Migrated>,
}

/// The storage migration this opening performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migrated {
    /// Schema version of the database before.
    pub from: i64,
    pub to: i64,
    /// The old database file, kept in the data directory.
    pub backup: std::path::PathBuf,
    pub elapsed: std::time::Duration,
}

/// Committed effects: netmap changes wake pending syncs, everything else
/// goes to the State runtime.
struct Effects {
    state: Arc<std::sync::Mutex<crate::state::service::Runtime>>,
    netmaps: Arc<crate::netmap::Hub>,
}
impl crate::effect::EffectSink for Effects {
    fn apply(&self, effect: crate::effect::Effect) {
        match effect {
            crate::effect::Effect::Netmap {
                network_id,
                node_ids,
            } => self.netmaps.wake(network_id, &node_ids),
            other => self.state.apply(other),
        }
    }
}

#[derive(Default)]
struct ShutdownHooks(std::sync::Mutex<Vec<Box<dyn Fn() + Send + Sync>>>);
impl ShutdownHooks {
    fn close(&self) {
        let hooks = std::mem::take(&mut *self.0.lock().unwrap_or_else(|error| error.into_inner()));
        for close in hooks {
            close();
        }
    }
}
impl Drop for ShutdownHooks {
    fn drop(&mut self) {
        self.close();
    }
}

/// Safe diagnostic categories: no filesystem paths, SQL, or credentials.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum AppError {
    #[error("invalid database budget or session setting")]
    InvalidSettings,
    #[error("data directory must be an absolute path without parent traversal")]
    InvalidDataDirectory,
    #[error("storage ownership, type, or private permissions are invalid")]
    UnsafeStorage,
    #[error("data directory is already in use")]
    AlreadyRunning,
    #[error("storage filesystem operation failed")]
    Filesystem,
    #[error("database application or schema version is unsupported")]
    UnsupportedSchema,
    #[error("database structure does not match the current schema: {0}")]
    SchemaStructure(String),
    #[error("database integrity check failed")]
    StorageIntegrity,
    #[error("database operation failed")]
    Database,
    #[error("application worker failed or stopped")]
    Worker,
    #[error("hosted STUN listener failed")]
    Stun,
    #[error("private storage is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("private master key is missing, invalid, or does not match storage")]
    MasterKey,
    #[error("private storage cryptography failed")]
    Cryptography,
    #[error(
        "database has an earlier schema version; run `meshway-server migrate` with the Server stopped first"
    )]
    MigrationRequired,
    #[error("not enough free space in the data directory to migrate the database")]
    InsufficientSpace,
    /// The backup destination, not the data directory: it exists already, its
    /// parent is not private to this user, or it cannot be written.
    #[error(
        "backup output must be a new directory in a private parent directory, with room for the database"
    )]
    BackupOutput,
}

/// `EX_CONFIG`: the exit status of a persistent storage block
/// ([`AppError::is_persistent`]); a service manager does not restart into it.
pub const PERSISTENT_STORAGE_BLOCK: u8 = 78;

impl AppError {
    /// A proven inconsistency or unsupported storage that a restart does not
    /// change: it needs an operator (exit status 78 for a service manager).
    /// Every other failure is transient, and a start again repeats the proof.
    pub fn is_persistent(&self) -> bool {
        matches!(
            self,
            Self::InvalidSettings
                | Self::InvalidDataDirectory
                | Self::UnsafeStorage
                | Self::UnsupportedSchema
                | Self::SchemaStructure(_)
                | Self::StorageIntegrity
                | Self::UnsupportedPlatform
                | Self::MasterKey
                | Self::MigrationRequired
        )
    }

    /// The process exit status of a command that stops on this error:
    /// [`PERSISTENT_STORAGE_BLOCK`] when persistent, else 1. The Server and
    /// every offline command share it.
    pub fn exit_code(&self) -> std::process::ExitCode {
        if self.is_persistent() {
            std::process::ExitCode::from(PERSISTENT_STORAGE_BLOCK)
        } else {
            std::process::ExitCode::FAILURE
        }
    }

    /// Operator-facing sentence for a failed storage opening. A safe
    /// category with, at most, schema object names.
    pub fn open_failure(&self) -> String {
        match self {
            Self::AlreadyRunning => {
                "Cannot obtain private Server storage ownership. Stop the Server first.".to_owned()
            }
            error => format!("Server storage startup failed: {error}."),
        }
    }
}

impl App {
    /// Opens a private, exclusively locked account-storage database.
    /// Existing unknown or damaged databases are rejected, never reset.
    pub async fn open(settings: Settings) -> Result<Self, AppError> {
        Self::open_with_downloads(settings, crate::resources::downloads::Downloads::new()).await
    }
    /// [`Self::open`] with another connection boundary for external resource
    /// sources, e.g. [`crate::resources::download::Offline`].
    pub async fn open_with_connector(
        settings: Settings,
        connector: Arc<dyn crate::resources::download::Connector>,
    ) -> Result<Self, AppError> {
        Self::open_with_downloads(
            settings,
            crate::resources::downloads::Downloads::with_connector(connector),
        )
        .await
    }
    async fn open_with_downloads(
        settings: Settings,
        downloads: Arc<crate::resources::downloads::Downloads>,
    ) -> Result<Self, AppError> {
        let options = Options {
            queue_capacity: settings.database_queue_capacity,
            busy_timeout: settings.database_busy_timeout,
        };
        options.validate().map_err(AppError::from)?;
        let account_options =
            crate::accounts::Options::new(settings.registration_enabled, settings.session_ttl)
                .ok_or(AppError::InvalidSettings)?;
        crate::settings::validate_stun(&settings)?;
        if !settings.netmap.valid() {
            return Err(AppError::InvalidSettings);
        }
        let stun_sockets = crate::stun::bind(&settings.stun_listen)?;
        let permit = DIRECTORY_PREPARATIONS
            .acquire()
            .await
            .map_err(|_| AppError::Worker)?;
        let migration = settings.migration;
        // The whole storage opening, migration and startup compilation, under
        // one renewal of the service manager's start timeout.
        let startup = startup::Startup::start("Opening the database");
        let progress = startup.progress();
        let (directory, migrated) = tokio::task::spawn_blocking(move || {
            // The task retains its budget through completion even if its caller
            // cancels: a started migration completes or fails as a whole. An
            // unclaimed result drops the directory and its lock; the SQLite
            // worker has not been opened at this stage.
            let _permit = permit;
            let mut directory = Directory::open(&settings.data_dir)?;
            let migrated = schema::migrate::prepare(&mut directory, migration, &progress)?;
            Ok::<_, AppError>((directory, migrated))
        })
        .await
        .map_err(|_| AppError::Worker)??;
        let mut app = Self::open_directory(
            directory,
            options,
            account_options,
            downloads,
            settings.console_origin.map(Arc::new),
            Arc::new(settings.delegation_clients),
            startup.progress(),
        )
        .await?;
        drop(startup);
        app.public_origin = settings.public_origin.map(Arc::new);
        app.migrated = migrated;
        app.stun = crate::stun::Hosted::start(stun_sockets)?;
        app.stun_servers = Arc::new(settings.stun_servers);
        app.netmap_timing = settings.netmap;
        Ok(app)
    }

    async fn open_directory(
        directory: Directory,
        options: Options,
        account_options: crate::accounts::Options,
        downloads: Arc<crate::resources::downloads::Downloads>,
        console_origin: Option<Arc<crate::settings::Url>>,
        delegation: Arc<crate::accounts::delegation::DelegationClients>,
        progress: startup::Progress,
    ) -> Result<Self, AppError> {
        progress.status("Checking the database");
        // The netmap reader needs the sealer opened with the database; it is
        // bound once the executor has opened (see `netmaps` below).
        let netmaps = Arc::new(crate::store::netmap::LateNetmaps::default());
        let state_runtime = Arc::new(std::sync::Mutex::new(
            crate::state::service::Runtime::new(netmaps.clone()).map_err(|_| AppError::Worker)?,
        ));
        let effects = Arc::new(Effects {
            state: Arc::clone(&state_runtime),
            netmaps: Arc::default(),
        });
        // No yield between taking the prepared directory and transferring it
        // to the SQLite worker. Dropping either future preserves lock ownership.
        let (executor, sealer) = Executor::open_with_effects(
            directory.database_path(),
            options,
            directory,
            Arc::downgrade(&effects) as std::sync::Weak<dyn crate::effect::EffectSink>,
            |connection, directory| {
                let sealer = schema::initialize(connection, |connection, fresh| {
                    secrets::initialize(connection, directory, fresh).map_err(AppError::from)
                })?;
                let transaction = connection
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .map_err(|_| AppError::Database)?;
                crate::netmap::signing::ensure(&transaction, &sealer)
                    .map_err(|_| AppError::StorageIntegrity)?;
                transaction.commit().map_err(|_| AppError::Database)?;
                Ok(sealer)
            },
        )
        .await
        .map_err(|error| match error {
            OpenError::Executor(error) => AppError::from(error),
            OpenError::Initialization(error) => error,
        })?;
        let accounts = Accounts::new(executor.handle(), account_options, delegation);
        let shutdown_hooks = ShutdownHooks::default();
        let shutdown_runtime = Arc::clone(&state_runtime);
        let shutdown_netmaps = Arc::clone(&effects.netmaps);
        shutdown_hooks
            .0
            .lock()
            .map_err(|_| AppError::Worker)?
            .push(Box::new(move || {
                shutdown_netmaps.close();
                if let Ok(mut runtime) = shutdown_runtime.lock() {
                    runtime.close();
                }
            }));
        let sealer = Arc::new(sealer);
        netmaps.bind(crate::store::netmap::StoredNetmaps::new(Arc::clone(
            &sealer,
        )));
        let netmap_signer = Arc::new(crate::netmap::signing::NetmapSigningKeys::new(Arc::clone(
            &sealer,
        )));
        // The one owner of configuration versions and their snapshot cache.
        let committer = crate::configuration::Committer::new(
            Arc::clone(&sealer),
            crate::configuration::SnapshotCache::shared(),
        );
        let resource_scheduler = crate::resources::scheduler::Scheduler::start(
            executor.handle(),
            crate::resources::provider::Provider {
                sealer: Arc::clone(&sealer),
                cache: Arc::clone(&committer.cache),
            },
            downloads.clone(),
        );
        let app = Self {
            traversal_maintenance: crate::connectivity::maintenance::Maintenance::start(
                executor.handle(),
                state_runtime.clone(),
            ),
            stun: crate::stun::Hosted::default(),
            stun_servers: Arc::default(),
            resource_scheduler,
            downloads,
            executor,
            accounts,
            sealer,
            committer,
            peer_facts: Arc::new(crate::state::facts::StateFacts::new(Arc::clone(
                &state_runtime,
            ))),
            state_runtime,
            effects,
            netmap_timing: crate::netmap::SyncTiming::default(),
            netmap_signer,
            dns_associations: Default::default(),
            netmaps,
            shutdown_hooks,
            console_origin,
            public_origin: None,
            migrated: None,
            observation_maintenance: std::sync::Mutex::default(),
            boot: meshway_model::InstanceId::new(
                crate::accounts::random_uuid().map_err(|_| AppError::Cryptography)?,
            )
            .map_err(|_| AppError::Cryptography)?,
        };
        // Deployment-owned IP datasets live in storage; derived State needs them
        // from the first report on, so loading is part of opening, not a task.
        app.ip_datasets()
            .restore()
            .await
            .map_err(|error| match error {
                crate::error::Error::Storage(_) => AppError::Database,
                _ => AppError::StorageIntegrity,
            })?;
        app.refresh_netmaps(&progress).await?;
        Ok(app)
    }

    /// Before any request: every stored netmap becomes this build's
    /// compilation of its current inputs (`store::netmap::refresh`), one
    /// transaction per network. A network that cannot be compiled keeps its
    /// stored netmaps and is logged; its next input change compiles again.
    async fn refresh_netmaps(&self, progress: &startup::Progress) -> Result<(), AppError> {
        let started = std::time::Instant::now();
        let handle = self.executor.handle();
        let networks = handle
            .read(crate::store::netmap::networks)
            .await
            .map_err(|_| AppError::Database)?;
        let mut changed = 0;
        for (index, network) in networks.iter().enumerate() {
            progress.status(&format!(
                "Compiling stored netmaps with this build: network {} of {}",
                index + 1,
                networks.len()
            ));
            let (network, sealer) = (*network, Arc::clone(&self.sealer));
            let result = handle
                .transact(rusqlite::TransactionBehavior::Immediate, move |tx| {
                    let now = crate::store::execution::now()?;
                    let netmap = crate::store::netmap::refresh(tx, &sealer, network, now)?;
                    let mut effects = netmap.effects();
                    if !effects.is_empty() {
                        effects.push(crate::effect::Effect::Progress {
                            network_id: network,
                        });
                    }
                    Ok((netmap.changed.len(), effects))
                })
                .await;
            match result {
                Ok(nodes) => changed += nodes,
                Err(error) => tracing::error!(
                    network_id = %network,
                    %error,
                    "Stored netmaps not compiled by this build; they stay until the next input change"
                ),
            }
        }
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let build = meshway_build::identity();
        if changed > 0 {
            tracing::info!(%build, networks = networks.len(), changed_nodes = changed, elapsed_ms,
                "Stored netmaps compiled again by this build");
        } else {
            tracing::debug!(%build, networks = networks.len(), elapsed_ms,
                "Stored netmaps checked against this build; nothing changed");
        }
        Ok(())
    }

    /// The storage migration this opening performed, if any.
    pub fn migrated(&self) -> Option<&Migrated> {
        self.migrated.as_ref()
    }

    /// An account service handle, without network or instance privileges.
    /// Handles stop accepting storage work when this App closes.
    pub fn accounts(&self) -> Accounts {
        self.accounts.clone()
    }

    pub fn networks(&self) -> Networks {
        Networks::new(self.executor.handle(), self.sealer(), self.committer())
    }

    pub(crate) fn committer(&self) -> crate::configuration::Committer {
        self.committer.clone()
    }
    pub(crate) fn configurations(&self) -> crate::configuration::Configurations {
        crate::configuration::Configurations::new(
            self.executor.handle(),
            self.committer(),
            self.downloads.clone(),
        )
    }
    pub(crate) fn history(&self) -> crate::history::History {
        crate::history::History::new(self.executor.handle(), self.sealer())
    }
    pub fn observation_service<P: crate::observations::service::ObservationProvider>(
        &self,
        provider: P,
    ) -> crate::observations::service::ObservationService<P> {
        crate::observations::service::ObservationService::new(
            self.executor.handle(),
            provider,
            Arc::clone(&self.state_runtime),
        )
    }
    pub fn observations(
        &self,
    ) -> crate::observations::service::ObservationService<crate::execution::NetmapDiagnostics> {
        let service = self.observation_service(self.netmap_diagnostics());
        let mut owner = self
            .observation_maintenance
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if matches!(*owner, crate::observations::service::MaintenanceOwner::Idle) {
            *owner = crate::observations::service::MaintenanceOwner::Running(service.start());
        }
        service
    }
    pub fn resource_service<P: crate::resources::service::ResourceProvider>(
        &self,
        provider: P,
    ) -> crate::resources::service::ResourceService<P> {
        crate::resources::service::ResourceService::new(
            self.executor.handle(),
            provider,
            self.downloads.clone(),
        )
    }
    /// The origin delegation clients should use for this Server, if configured.
    pub fn public_origin(&self) -> Option<&crate::settings::Url> {
        self.public_origin.as_deref()
    }
    pub fn ip_datasets(&self) -> crate::resources::datasets::IpDatasets {
        crate::resources::datasets::IpDatasets::new(
            self.executor.handle(),
            Arc::clone(&self.state_runtime),
        )
    }
    pub fn resources(
        &self,
    ) -> crate::resources::service::ResourceService<crate::resources::provider::Provider> {
        self.resource_service(crate::resources::provider::Provider {
            sealer: self.sealer(),
            cache: Arc::clone(&self.committer.cache),
        })
    }
    /// Peer facts for Node `sync`, scoped by each Node's netmap.
    pub fn peer_facts(&self) -> crate::state::facts::StateFacts {
        crate::state::facts::StateFacts::new(Arc::clone(&self.state_runtime))
    }
    /// Observation authority: reads the source's netmap only.
    pub fn netmap_diagnostics(&self) -> crate::execution::NetmapDiagnostics {
        crate::execution::NetmapDiagnostics::new(self.execution(), self.netmaps.clone())
    }
    #[cfg(test)]
    pub(crate) fn state_reader(&self) -> crate::state::service::StateReader {
        crate::state::service::StateReader::new(Arc::clone(&self.state_runtime))
    }

    /// An owned action for a server's 'static graceful-shutdown future. Close
    /// long-lived streams before waiting for HTTP drain; accepted SQLite jobs
    /// remain owned by the executor until `shutdown` completes.
    pub fn stream_shutdown(&self) -> impl FnOnce() + Send + 'static {
        let runtime = Arc::clone(&self.state_runtime);
        let downloads = self.downloads.clone();
        let netmaps = Arc::clone(&self.effects.netmaps);
        move || {
            netmaps.close();
            runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .close();
            downloads.close();
        }
    }

    fn state_projection(&self) -> crate::state::provider::Projection {
        crate::state::provider::Projection {
            execution: self.execution(),
            sealer: self.sealer(),
            cache: Arc::clone(&self.committer.cache),
            boot: self.boot,
        }
    }
    pub fn user_state_service(
        &self,
    ) -> Result<
        crate::state::service::StateService<crate::state::provider::UserProvider>,
        crate::error::Error,
    > {
        self.state_service(crate::state::provider::UserProvider(
            self.state_projection(),
        ))
    }
    pub fn node_state_service(
        &self,
    ) -> Result<
        crate::state::service::StateService<crate::state::provider::NodeProvider>,
        crate::error::Error,
    > {
        self.state_service(crate::state::provider::NodeProvider(
            self.state_projection(),
        ))
    }

    /// Compose State HTTP/SSE with the transactional management/Node providers.
    pub fn state_service<P: crate::state::service::StateProvider>(
        &self,
        provider: P,
    ) -> Result<crate::state::service::StateService<P>, crate::error::Error> {
        // Runtime shutdown is registered once when the App opens. Registering
        // each handle would retain its provider and replay subscriptions until
        // App shutdown, even after the last caller has dropped that service.
        crate::state::service::StateService::new(
            self.executor.handle(),
            provider,
            self.sealer(),
            Arc::clone(&self.state_runtime),
        )
    }
    /// Node netmap distribution (`sync`).
    pub fn netmaps(&self) -> crate::netmap::Netmaps {
        crate::netmap::Netmaps::new(
            self.executor.handle(),
            self.sealer(),
            self.boot,
            Arc::clone(&self.effects.netmaps),
            Arc::clone(&self.peer_facts),
            Arc::clone(&self.netmap_signer),
            self.netmap_timing,
            Arc::clone(&self.dns_associations),
        )
    }
    /// Starts a netmap signing-key rotation window (see `netmap::signing`).
    /// Rejected with `StateConflict` while a window is open.
    pub async fn rotate_netmap_key(
        &self,
        window: std::time::Duration,
    ) -> Result<crate::netmap::signing::RotatedSigningKey, crate::error::Error> {
        let sealer = self.sealer();
        self.executor
            .handle()
            .write(move |tx| crate::netmap::signing::rotate(tx, &sealer, window))
            .await
    }
    pub fn execution(&self) -> crate::execution::Execution {
        crate::execution::Execution::new(
            self.executor.handle(),
            self.sealer(),
            self.boot,
            self.netmaps.clone(),
        )
    }
    pub fn connectivity(&self) -> crate::connectivity::Connectivity {
        crate::connectivity::Connectivity::new(
            self.executor.handle(),
            self.execution(),
            self.state_runtime.clone(),
            self.stun_servers.clone(),
            self.stun.health(),
        )
    }
    #[cfg(test)]
    pub(crate) fn stun_ready(&self) -> bool {
        self.stun.health().ready()
    }
    pub fn enrollment(&self) -> crate::enrollment::Enrollment {
        crate::enrollment::Enrollment::new(
            self.executor.handle(),
            self.sealer(),
            self.committer(),
            self.console_origin.clone(),
        )
    }

    /// Runs full, read-only schema, SQLite integrity, and foreign-key checks on
    /// the owning thread against one consistent database snapshot.
    pub async fn check_storage(&self) -> Result<(), AppError> {
        let sealer = self.sealer();
        self.executor
            .handle()
            .execute(move |connection| {
                schema::check_with(connection, |connection| {
                    sealer.check_binding(connection).map_err(AppError::from)
                })
            })
            .await??;
        Ok(())
    }

    pub(crate) fn sealer(&self) -> Arc<Sealer> {
        Arc::clone(&self.sealer)
    }

    /// Stops admission, drains accepted operations, and waits for SQLite close.
    /// Cancelling this future still leaves the process lock with the worker.
    pub async fn shutdown(self) -> Result<(), AppError> {
        self.shutdown_hooks.close();
        self.downloads.close();
        let maintenance = std::mem::replace(
            &mut *self
                .observation_maintenance
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
            crate::observations::service::MaintenanceOwner::Closed,
        );
        // Each shutdown signals its producer before its first await. Poll all
        // of them together so one queued SQL reply or task join cannot delay
        // cancellation of another producer. Accepted SQL work drains below.
        let (stun_result, maintenance_result, (), traversal_result, ()) = tokio::join!(
            self.stun.shutdown(),
            async move {
                match maintenance {
                    crate::observations::service::MaintenanceOwner::Running(worker) => {
                        worker.shutdown().await.map_err(|_| AppError::Worker)
                    }
                    _ => Ok(()),
                }
            },
            self.resource_scheduler.shutdown(),
            self.traversal_maintenance.shutdown(),
            // Started conversions cannot be cancelled; they end here.
            self.downloads.join(),
        );
        self.executor.shutdown().await?;
        traversal_result.map_err(|_| AppError::Worker)?;
        maintenance_result?;
        stun_result?;
        Ok(())
    }
}

impl From<KeyError> for AppError {
    fn from(error: KeyError) -> Self {
        match error {
            KeyError::Missing | KeyError::Invalid | KeyError::Mismatch => Self::MasterKey,
            KeyError::Database => Self::Database,
            KeyError::Directory(error) => error.into(),
            KeyError::Sealing(_) => Self::Cryptography,
        }
    }
}

impl From<DirectoryError> for AppError {
    fn from(error: DirectoryError) -> Self {
        match error {
            DirectoryError::InvalidPath => Self::InvalidDataDirectory,
            DirectoryError::UnsafeStorage => Self::UnsafeStorage,
            DirectoryError::InUse => Self::AlreadyRunning,
            DirectoryError::Io => Self::Filesystem,
            #[cfg(not(unix))]
            DirectoryError::UnsupportedPlatform => Self::UnsupportedPlatform,
        }
    }
}

impl From<ExecutorError> for AppError {
    fn from(error: ExecutorError) -> Self {
        match error {
            ExecutorError::InvalidQueueCapacity | ExecutorError::InvalidBusyTimeout => {
                Self::InvalidSettings
            }
            // SQLITE_CORRUPT, SQLITE_NOTADB: see `schema::damaged`.
            ExecutorError::Sqlite { code: 11 | 26, .. } => Self::StorageIntegrity,
            ExecutorError::Sqlite { .. } | ExecutorError::ConnectionConfiguration => Self::Database,
            ExecutorError::ThreadStart
            | ExecutorError::Closed
            | ExecutorError::WorkerPanicked
            | ExecutorError::JoinFailed => Self::Worker,
        }
    }
}

impl From<schema::SchemaError> for AppError {
    fn from(error: schema::SchemaError) -> Self {
        match error {
            schema::SchemaError::Unsupported => Self::UnsupportedSchema,
            schema::SchemaError::Structure(detail) => Self::SchemaStructure(detail),
            schema::SchemaError::Integrity => Self::StorageIntegrity,
            schema::SchemaError::Sqlite => Self::Database,
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
