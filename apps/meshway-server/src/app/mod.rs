use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::accounts::Accounts;
use crate::directory::{Directory, DirectoryError};
use crate::networks::Networks;
use crate::secrets::{self, KeyError, Sealer};
use crate::settings::Settings;
use crate::store::{Executor, ExecutorError, OpenError, Options, schema};

const MAX_DIRECTORY_PREPARATIONS: u32 = 4;
static DIRECTORY_PREPARATIONS: Semaphore =
    Semaphore::const_new(MAX_DIRECTORY_PREPARATIONS as usize);

/// Application lifecycle owner. Dropping closes work admission immediately;
/// the SQLite worker retains its process lock until accepted work has drained
/// and the connection has closed. Use [`Self::shutdown`] to await that boundary.
pub struct App {
    stun: crate::stun::Hosted,
    pub(crate) stun_servers:
        Arc<std::collections::BTreeMap<String, meshway_transport_spec::Endpoint>>,
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
    netmap_signer: Arc<crate::netmap::signing::NetmapSigningKey>,
    shutdown_hooks: ShutdownHooks,
    console_origin: Option<Arc<crate::settings::Url>>,
    public_origin: Option<Arc<crate::settings::Url>>,
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
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
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
}

impl App {
    /// Opens a private, exclusively locked account-storage database.
    /// Existing unknown or damaged databases are rejected, never reset.
    pub async fn open(settings: Settings) -> Result<Self, AppError> {
        Self::open_with_downloads(settings, crate::resources::downloads::Downloads::new()).await
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
        let directory = tokio::task::spawn_blocking(move || {
            // The task retains its budget through completion even if its caller
            // cancels. An unclaimed result drops the directory and its lock;
            // SQLite has not been opened at this stage.
            let _permit = permit;
            Directory::open(&settings.data_dir)
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
        )
        .await?;
        app.public_origin = settings.public_origin.map(Arc::new);
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
    ) -> Result<Self, AppError> {
        let state_runtime = Arc::new(std::sync::Mutex::new(
            crate::state::service::Runtime::new().map_err(|_| AppError::Worker)?,
        ));
        let effects = Arc::new(Effects {
            state: Arc::clone(&state_runtime),
            netmaps: Arc::default(),
        });
        // No yield between taking the prepared directory and transferring it
        // to the SQLite worker. Dropping either future preserves lock ownership.
        let (executor, (sealer, netmap_signer)) = Executor::open_with_effects(
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
                crate::store::execution::retire_obsolete(&transaction, &sealer)
                    .map_err(|_| AppError::StorageIntegrity)?;
                let signer =
                    crate::netmap::signing::NetmapSigningKey::load_or_create(&transaction, &sealer)
                        .map_err(|_| AppError::StorageIntegrity)?;
                transaction.commit().map_err(|_| AppError::Database)?;
                Ok((sealer, signer))
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
        let netmap_signer = Arc::new(netmap_signer);
        // The one owner of configuration versions and their snapshot cache.
        let committer = crate::configuration::Committer::new(Arc::clone(&sealer), Arc::default());
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
            state_runtime,
            effects,
            netmap_timing: crate::netmap::SyncTiming::default(),
            peer_facts: Arc::new(crate::netmap::NoPeerFacts),
            netmap_signer,
            shutdown_hooks,
            console_origin,
            public_origin: None,
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
        Ok(app)
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
    ) -> crate::observations::service::ObservationService<crate::execution::Execution> {
        let service = self.observation_service(self.execution());
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
    /// The netmap store owner installs its reader once; until then no Node has
    /// a netmap, so no fact is recorded or disclosed.
    pub fn install_netmaps(&self, netmaps: Arc<dyn crate::state::facts::NetmapReader>) {
        self.state_runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .install_netmaps(netmaps);
    }
    /// Netmap-chain observation authority: reads the source's netmap only.
    pub fn netmap_diagnostics(&self) -> crate::execution::NetmapDiagnostics {
        let netmaps = Arc::clone(
            &self
                .state_runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .netmaps,
        );
        crate::execution::NetmapDiagnostics::new(self.execution(), netmaps)
    }
    /// Shared core handles are available before constructing any HTTP provider.
    pub fn state_reader(&self) -> crate::state::service::StateReader {
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
        )
    }
    pub fn execution(&self) -> crate::execution::Execution {
        crate::execution::Execution::new(self.executor.handle(), self.sealer(), self.boot)
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
    pub fn stun_ready(&self) -> bool {
        self.stun.ready()
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
        let (stun_result, maintenance_result, (), traversal_result) = tokio::join!(
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
