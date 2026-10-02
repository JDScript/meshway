//! Normal session orchestration. Persistence belongs to state; watchdogs never wait on HTTP.

use std::{future::Future, path::PathBuf, task::Poll, time::Duration};

use meshway_protocol::{
    BeginConnectionRuntimeRequest, BeginConnectionRuntimeResponse,
    ConnectionRuntimeReason as Reason, GetConnectionRuntimeEpochResponse,
    ReportConnectionRuntimeRequest, ReportConnectionRuntimeResponse,
    ReportEndpointDiscoveryRequest, ReportEndpointDiscoveryResponse,
};
use tokio::time::{self, Instant};

use crate::managed_connections::Manager;
use crate::state::connections::{
    ConnectionSyncReceipt, ControlSyncReceipts, cache_path, convert, dataplane, materials,
    mesh_execution, validation,
};
use crate::state::{Agent, AgentError, connection_key, local};
use meshway_control_client::client::HttpClient;

mod cli;

/// The library receives cancellation and observations; terminal policy belongs
/// to the CLI adapter. Neither reporting nor an observer renews authorization.
#[derive(Clone)]
pub struct SessionControl {
    pub cancellation: tokio_util::sync::CancellationToken,
    pub events: tokio::sync::mpsc::Sender<SessionEvent>,
}

pub enum SessionEvent {
    Receipt(String),
    Retry { message: String, delay: Duration },
}

pub struct ManagedSession {
    pub profiles: Option<PathBuf>,
    pub interval: Duration,
    pub lease: Duration,
    pub tun: Option<crate::TunOptions>,
    pub discovery: crate::EndpointDiscoveryOptions,
}

struct RuntimeLease {
    events: Option<tokio::sync::mpsc::Sender<SessionEvent>>,
    instance_id: String,
    expected_epoch: Option<u32>,
    epoch: Option<u32>,
    sequence: u32,
    discovery_sequence: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiscoveryReportMode {
    Normal,
    Shutdown,
}

impl RuntimeLease {
    fn new() -> Result<Self, AgentError> {
        Ok(Self {
            events: None,
            instance_id: local::credential()?,
            expected_epoch: None,
            epoch: None,
            sequence: 0,
            discovery_sequence: 0,
        })
    }

    fn receipt(&self, encoded: String) {
        if let Some(events) = &self.events {
            let _ = events.try_send(SessionEvent::Receipt(encoded));
        }
    }
}

impl Agent {
    /// IPv4-only coherent policy application. Never activates a cached grant at startup.
    pub async fn run_managed_dataplane(
        &mut self,
        profiles: Option<PathBuf>,
        interval: Duration,
        lease: Duration,
        options: crate::TunOptions,
    ) -> Result<(), AgentError> {
        options.validate()?;
        if !cfg!(target_os = "linux") {
            return Err(AgentError::Invalid("--apply-dataplane requires Linux"));
        }
        self.run_managed(
            profiles,
            interval,
            lease,
            Some(options),
            crate::EndpointDiscoveryOptions::default(),
        )
        .await
    }

    /// Opt-in managed adjacencies. The ordinary saved-only sync API is unchanged.
    pub async fn run_managed_connections(
        &mut self,
        profiles: Option<PathBuf>,
        interval: Duration,
        lease: Duration,
    ) -> Result<(), AgentError> {
        self.run_managed(
            profiles,
            interval,
            lease,
            None,
            crate::EndpointDiscoveryOptions::default(),
        )
        .await
    }

    pub async fn run_managed_connections_with_discovery(
        &mut self,
        profiles: Option<PathBuf>,
        interval: Duration,
        lease: Duration,
        discovery: crate::EndpointDiscoveryOptions,
    ) -> Result<(), AgentError> {
        self.run_managed(profiles, interval, lease, None, discovery)
            .await
    }

    pub async fn run_managed_dataplane_with_discovery(
        &mut self,
        profiles: Option<PathBuf>,
        interval: Duration,
        lease: Duration,
        options: crate::TunOptions,
        discovery: crate::EndpointDiscoveryOptions,
    ) -> Result<(), AgentError> {
        options.validate()?;
        if !cfg!(target_os = "linux") {
            return Err(AgentError::Invalid("--apply-dataplane requires Linux"));
        }
        self.run_managed(profiles, interval, lease, Some(options), discovery)
            .await
    }

    async fn run_managed(
        &mut self,
        profiles: Option<PathBuf>,
        interval: Duration,
        lease: Duration,
        options: Option<crate::TunOptions>,
        discovery: crate::EndpointDiscoveryOptions,
    ) -> Result<(), AgentError> {
        let cli = cli::Context::new();
        self.run_managed_session(
            ManagedSession {
                profiles,
                interval,
                lease,
                tun: options,
                discovery,
            },
            cli.control.clone(),
        )
        .await
    }

    pub async fn run_managed_session(
        &mut self,
        session: ManagedSession,
        control: SessionControl,
    ) -> Result<(), AgentError> {
        let ManagedSession {
            profiles,
            interval,
            lease,
            tun: options,
            discovery,
        } = session;
        if let Some(options) = &options {
            options.validate()?;
            if !cfg!(target_os = "linux") {
                return Err(AgentError::Invalid("managed IPv4 TUN requires Linux"));
            }
        }
        if !(Duration::from_secs(10)..=Duration::from_secs(300)).contains(&lease)
            || interval.is_zero()
            || interval >= lease
        {
            return Err(AgentError::Invalid(
                "connection lease must be 10–300 seconds and exceed the polling interval",
            ));
        }
        tokio::select! {
            biased;
            _ = control.cancellation.cancelled() => return Ok(()),
            result = self.prepare_control_plane() => result?,
        }
        let mut dataplane = options
            .as_ref()
            .map(|_| dataplane::State::load(self))
            .transpose()?;
        let mut manager = if let Some(options) = options {
            Manager::new_dataplane(
                profiles,
                lease,
                options,
                self.identity.controller.clone(),
                dataplane.as_ref().is_some_and(dataplane::State::has_cache),
            )
        } else {
            Manager::new(profiles, lease)
        };
        #[cfg(target_os = "linux")]
        if let Some(control) = &self.underlay {
            manager
                .install_linux_underlay(control.owner.clone())
                .await?;
        }
        manager.enable_discovery(discovery);
        manager.set_events(control.events.clone());
        let mut runtime_lease = RuntimeLease::new()?;
        runtime_lease.events = Some(control.events);
        #[cfg(unix)]
        let diagnostics = crate::diagnostics::Server::bind(&self.path, &self._lock).await?;
        #[cfg(unix)]
        let prober = manager.prober();
        #[cfg(unix)]
        let dns_guard = self.dns_guard.clone();
        #[cfg(unix)]
        let result = tokio::select! {
            result = self.managed_loop_with_lease(&mut manager, interval, &mut dataplane, &mut runtime_lease) => result,
            result = diagnostics.serve_with_dns(Some(prober), dns_guard) => result.map_err(AgentError::from),
            _ = control.cancellation.cancelled() => Ok(()),
        };
        #[cfg(not(unix))]
        let result = tokio::select! {
            result = self.managed_loop_with_lease(&mut manager, interval, &mut dataplane, &mut runtime_lease) => result,
            _ = control.cancellation.cancelled() => Ok(()),
        };
        if let Some(guard) = &self.dns_guard {
            guard.shutdown();
        }
        manager.withdraw(Reason::ShuttingDown).await;
        if runtime_lease.epoch.is_some() {
            // No lease renewal and no failure can hold the control loop open.
            let _ = self
                .report_discovery(
                    &mut manager,
                    &mut runtime_lease,
                    DiscoveryReportMode::Shutdown,
                    |_| {},
                )
                .await;
        }
        result
    }

    #[cfg(test)]
    pub(super) async fn managed_loop(
        &mut self,
        manager: &mut Manager,
        interval: Duration,
        dataplane: &mut Option<dataplane::State>,
    ) -> Result<(), AgentError> {
        self.managed_loop_with_lease(manager, interval, dataplane, &mut RuntimeLease::new()?)
            .await
    }

    async fn managed_loop_with_lease(
        &mut self,
        manager: &mut Manager,
        interval: Duration,
        dataplane: &mut Option<dataplane::State>,
        lease: &mut RuntimeLease,
    ) -> Result<(), AgentError> {
        let mut backoff = 1_u64;
        loop {
            let mut control_synced = false;
            let result = self
                .managed_tick(manager, lease, &mut control_synced, dataplane)
                .await;
            // Recovery belongs to the validated, durable configuration sync,
            // not to the later best-effort observation report. The first probe
            // of a replacement can legitimately fence that report. Retaining an
            // outage backoff longer than the lease would expire every new
            // generation before its next tick and trap recovery indefinitely.
            if control_synced {
                backoff = 1;
            }
            let delay = match result {
                Ok(()) => {
                    backoff = 1;
                    interval
                }
                Err(AgentError::Http {
                    status: 422,
                    retryable: true,
                }) => {
                    if let Some(guard) = &self.dns_guard
                        && let Some(snapshot) = &manager.snapshot
                    {
                        guard.conflict(snapshot.payload.revision);
                    }
                    // Revision/digest changed during report. Withdraw and pull;
                    // never label the old channels current or renew by reporting.
                    manager.withdraw(Reason::InvalidConfiguration).await;
                    Duration::from_secs(1)
                }
                Err(error) if error.retryable() => {
                    let delay = Duration::from_secs(backoff);
                    backoff = (backoff * 2).min(30);
                    if let Some(events) = &lease.events {
                        let _ = events.try_send(SessionEvent::Retry {
                            message: error.to_string(),
                            delay,
                        });
                    }
                    delay
                }
                Err(error) => {
                    if let Some(guard) = &self.dns_guard {
                        guard.shutdown();
                    }
                    manager.withdraw(Reason::InternalError).await;
                    return Err(error);
                }
            };
            time::sleep(delay).await;
        }
    }

    async fn managed_tick(
        &mut self,
        manager: &mut Manager,
        lease: &mut RuntimeLease,
        control_synced: &mut bool,
        dataplane: &mut Option<dataplane::State>,
    ) -> Result<(), AgentError> {
        #[cfg(target_os = "linux")]
        manager.refresh_underlay().await?;
        let heartbeat = self.heartbeat().await?;
        // A trusted response is stale-generation evidence before any subsequent
        // HTTP can block. This never renews a lease or installs a lower revision.
        let revision = u32::try_from(heartbeat.node.configuration_revision)
            .ok()
            .filter(|revision| *revision != 0)
            .ok_or(AgentError::Invalid(
                "invalid heartbeat configuration revision",
            ))?;
        if let Some(guard) = &self.dns_guard {
            guard.observe_revision(revision);
        }
        manager.invalidate_dataplane_revision(revision).await;
        let published = materials::publish(self).await?;
        if let Some(guard) = &self.dns_guard {
            guard.observe_revision(published);
        }
        manager.invalidate_dataplane_revision(published).await;
        let (mut receipt, mut packet_snapshot, mut fresh_at) =
            self.managed_sync_mode(manager, dataplane, lease).await?;
        if lease.epoch.is_none() {
            if lease.expected_epoch.is_none() {
                let response: GetConnectionRuntimeEpochResponse =
                    convert(self.runtime_client().get_connection_runtime_epoch().await?)?;
                // Pin once, together with the process nonce. A lost begin reply
                // must never refresh this CAS and steal a newer process's lease.
                lease.expected_epoch = Some(response.epoch);
            }
            let response = self
                .runtime_client()
                .begin_connection_runtime(convert(BeginConnectionRuntimeRequest {
                    instance_id: lease.instance_id.clone(),
                    expected_epoch: lease.expected_epoch.expect("startup epoch pinned"),
                })?)
                .await;
            let response = match response {
                Err(error) if error.api().is_some_and(|error| error.status == 409) => {
                    manager.withdraw(Reason::LeaseSuperseded).await;
                    return Err(AgentError::Invalid(
                        "runtime lease superseded; refusing to reacquire",
                    ));
                }
                Err(error) => return Err(error.into()),
                Ok(response) => response,
            };
            let response: BeginConnectionRuntimeResponse = convert(response)?;
            if lease.expected_epoch.and_then(|epoch| epoch.checked_add(1)) != Some(response.epoch) {
                return Err(AgentError::Invalid("invalid runtime lease response"));
            }
            lease.epoch = Some(response.epoch);
            // Pull while holding the runtime lease, not merely before begin.
            (receipt, packet_snapshot, fresh_at) =
                self.managed_sync_mode(manager, dataplane, lease).await?;
        }
        let state = self
            .connections
            .as_ref()
            .expect("freshly synced connections");
        state.check(&self.path, &self.identity)?;
        local::ensure_durable(&cache_path(&self.path))?;
        let key = state.keys.application_key(&self.path, &self.identity)?;
        let snapshot = state
            .cache
            .as_ref()
            .expect("fresh saved cache")
            .snapshot
            .clone();
        *control_synced = true;
        if let Some((snapshot, fresh_at)) = packet_snapshot {
            match snapshot {
                mesh_execution::Execution::Legacy(snapshot) => {
                    manager.reconcile_dataplane(snapshot, key, fresh_at).await?
                }
                mesh_execution::Execution::Mesh(snapshot) => {
                    manager
                        .reconcile_mesh_execution(snapshot, key, fresh_at)
                        .await?
                }
            }
        } else {
            manager.reconcile(snapshot, key, fresh_at).await;
        }
        lease.receipt(
            serde_json::to_string(&receipt)
                .map_err(|_| AgentError::Invalid("cannot encode saved receipt"))?,
        );
        let client = self.runtime_client();
        let (request, response) = self.report_runtime(manager, lease, &client).await?;
        #[cfg(unix)]
        manager
            .latency_report_accepted(request.epoch, request.state, client.clone())
            .await;
        #[derive(serde::Serialize)]
        struct RuntimeReceipt<'a> {
            runtime: &'a ReportConnectionRuntimeRequest,
            reported_at: chrono::DateTime<chrono::Utc>,
        }
        lease.receipt(
            serde_json::to_string(&RuntimeReceipt {
                runtime: &request,
                reported_at: response.reported_at,
            })
            .map_err(|_| AgentError::Invalid("cannot encode runtime receipt"))?,
        );
        let events = lease.events.clone();
        self.report_discovery(manager, lease, DiscoveryReportMode::Normal, |receipt| {
            if let Some(events) = &events {
                let _ = events.try_send(SessionEvent::Receipt(receipt.to_owned()));
            }
        })
        .await?;
        Ok(())
    }

    async fn report_runtime(
        &self,
        manager: &mut Manager,
        lease: &mut RuntimeLease,
        client: &HttpClient,
    ) -> Result<
        (
            ReportConnectionRuntimeRequest,
            ReportConnectionRuntimeResponse,
        ),
        AgentError,
    > {
        // Match the generated client's original ten-second request budget across
        // all attempts. Never perform another sync, renew a lease or restart the
        // Manager merely because its ordinary five-second probe published.
        let http_deadline = Instant::now() + Duration::from_secs(10);
        for _ in 0..3 {
            lease.sequence = lease
                .sequence
                .checked_add(1)
                .ok_or(AgentError::Invalid("runtime report sequence exhausted"))?;
            let snapshot = manager.snapshot.as_ref().expect("reconciled snapshot");
            let (observed, guard) = manager.report_snapshot();
            let request = ReportConnectionRuntimeRequest {
                epoch: lease.epoch.expect("runtime lease acquired"),
                sequence: lease.sequence,
                revision: snapshot.payload.revision,
                digest: snapshot.digest.clone(),
                state: observed.state,
                reason: observed.reason,
                listeners: observed.listeners,
                peers: observed.peers,
            };
            let body = convert(&request)?;
            if Instant::now() >= http_deadline {
                return Err(AgentError::Transport { retryable: true });
            }
            if guard.as_ref().is_some_and(|guard| !guard.current()) {
                if guard
                    .as_ref()
                    .is_some_and(|guard| guard.generation_current())
                {
                    continue;
                }
                return Err(AgentError::RuntimeObservationChanged);
            }
            let response = {
                let mut waiter = guard.clone();
                let send = client.report_connection_runtime(body);
                tokio::pin!(send);
                let admitted = std::future::poll_fn(|cx| {
                    // An entire generated HTTP poll can advance outbound I/O,
                    // not merely inspect an already received status. Admit every
                    // poll against the original token and original HTTP budget.
                    if Instant::now() >= http_deadline {
                        return Poll::Ready(Err(AgentError::Transport { retryable: true }));
                    }
                    if guard.as_ref().is_some_and(|guard| !guard.current()) {
                        return Poll::Ready(Err(AgentError::RuntimeObservationChanged));
                    }
                    // A terminal response received during this valid poll may
                    // carry 403/409 even if that same poll also publishes a
                    // fence. Preserve it; never poll invalid I/O to seek status.
                    send.as_mut().poll(cx).map(Ok)
                });
                tokio::select! {
                    biased;
                    response = admitted => match response {
                        Ok(response) => Some(response),
                        Err(AgentError::RuntimeObservationChanged) => None,
                        Err(error) => return Err(error),
                    },
                    _ = time::sleep_until(http_deadline) => return Err(AgentError::Transport { retryable: true }),
                    _ = async { match &mut waiter { Some(guard) => guard.changed().await, None => std::future::pending().await } } => None,
                }
            };
            let Some(response) = response else {
                // Drop the old HTTP future/body even on peer failure. Only a
                // fresh snapshot (possibly negative) may be retried. Withdrawal,
                // source changes and expiry forbid recapturing a new generation.
                if guard
                    .as_ref()
                    .is_some_and(|guard| guard.generation_current())
                {
                    continue;
                }
                return Err(AgentError::RuntimeObservationChanged);
            };
            let response = match response {
                Err(error) if error.api().is_some_and(|error| error.status == 409) => {
                    manager.withdraw(Reason::LeaseSuperseded).await;
                    return Err(AgentError::Invalid(
                        "runtime lease superseded; refusing to reacquire",
                    ));
                }
                Err(error) if error.api().is_some_and(|error| error.status == 422) => {
                    return Err(AgentError::Http {
                        status: 422,
                        retryable: true,
                    });
                }
                Err(error) => return Err(error.into()),
                Ok(response) => convert::<ReportConnectionRuntimeResponse>(response)?,
            };
            if response.epoch != request.epoch || response.sequence != request.sequence {
                return Err(AgentError::Invalid("runtime report response mismatch"));
            }
            if guard.as_ref().is_some_and(|guard| !guard.current()) {
                if guard
                    .as_ref()
                    .is_some_and(|guard| guard.generation_current())
                {
                    continue;
                }
                return Err(AgentError::RuntimeObservationChanged);
            }
            if Instant::now() >= http_deadline {
                return Err(AgentError::Transport { retryable: true });
            }
            return Ok((request, response));
        }
        Err(AgentError::RuntimeObservationChanged)
    }

    async fn report_discovery(
        &self,
        manager: &mut Manager,
        lease: &mut RuntimeLease,
        mode: DiscoveryReportMode,
        mut receipt_writer: impl FnMut(&str),
    ) -> Result<(), AgentError> {
        let Some(snapshot) = manager.snapshot.as_ref() else {
            return Ok(());
        };
        let (state, sockets, mut guard) = manager.discovery_snapshot();
        lease.discovery_sequence = lease
            .discovery_sequence
            .checked_add(1)
            .ok_or(AgentError::Invalid("discovery report sequence exhausted"))?;
        let request = ReportEndpointDiscoveryRequest {
            epoch: lease.epoch.expect("runtime lease acquired"),
            sequence: lease.discovery_sequence,
            revision: snapshot.payload.revision,
            digest: snapshot.digest.clone(),
            state,
            sockets,
        };
        let body = convert(&request)?;
        let client = self.runtime_client();
        let http_deadline = Instant::now() + Duration::from_secs(2);
        let send = client.report_endpoint_discovery(body);
        let response = tokio::select! {
            biased;
            _ = async { match &mut guard { Some(guard) => guard.changed().await, None => std::future::pending().await } } => return Ok(()),
            _ = time::sleep_until(http_deadline) => return Ok(()),
            result = send => result,
        };
        let response = match response {
            Err(error) if error.api().is_some_and(|error| error.status == 409) => {
                // Diagnostic watermark conflicts are not runtime ownership loss.
                // Confirm using the generated read operation, within the same
                // HTTP budget and generation fence; never begin/reacquire.
                let epoch = tokio::select! {
                    biased;
                    _ = async { match &mut guard { Some(guard) => guard.changed().await, None => std::future::pending().await } } => return Ok(()),
                    _ = time::sleep_until(http_deadline) => return Ok(()),
                    result = client.get_connection_runtime_epoch() => result,
                };
                let epoch = match epoch {
                    Ok(epoch) => convert::<GetConnectionRuntimeEpochResponse>(epoch)?,
                    Err(error) if error.api().is_some_and(|error| error.status == 403) => {
                        return Err(error.into());
                    }
                    Err(_) => return Ok(()),
                };
                if guard.as_ref().is_some_and(|guard| !guard.current())
                    || epoch.epoch == request.epoch
                {
                    return Ok(());
                }
                manager.withdraw(Reason::LeaseSuperseded).await;
                return Err(AgentError::Invalid(
                    "runtime lease superseded; refusing to reacquire",
                ));
            }
            Err(error) if error.api().is_some_and(|error| error.status == 403) => {
                return Err(error.into());
            }
            Err(error) if error.api().is_some_and(|error| error.status == 422) => {
                // Observation time/schema rejection is not proof that the
                // installed authorization source changed. Confirm its current
                // projection without ACK, persistence, lease renewal or DNS
                // publication, within the original report HTTP budget.
                let fresh = tokio::select! {
                    biased;
                    _ = async { match &mut guard { Some(guard) => guard.changed().await, None => std::future::pending().await } } => return Ok(()),
                    _ = time::sleep_until(http_deadline) => return Ok(()),
                    result = client.get_agent_connections() => result,
                };
                let fresh = match fresh {
                    Ok(fresh) => match convert::<meshway_protocol::AgentConnectionsSnapshot>(fresh)
                    {
                        Ok(fresh) => fresh,
                        Err(_) => return Ok(()),
                    },
                    Err(error) if error.api().is_some_and(|error| error.status == 403) => {
                        return Err(error.into());
                    }
                    Err(_) => return Ok(()),
                };
                if guard.as_ref().is_some_and(|guard| !guard.current()) {
                    return Ok(());
                }
                let Some(key) = self
                    .connections
                    .as_ref()
                    .and_then(|state| state.keys.public_key())
                else {
                    return Ok(());
                };
                if validation::validate(&fresh, &self.identity, key, manager.snapshot.as_ref())
                    .is_err()
                    || (fresh.payload.revision == request.revision
                        && fresh.digest == request.digest)
                {
                    return Ok(());
                }
                manager.withdraw(Reason::InvalidConfiguration).await;
                return Err(AgentError::Http {
                    status: 422,
                    retryable: true,
                });
            }
            Err(_) => return Ok(()),
            Ok(response) => convert::<ReportEndpointDiscoveryResponse>(response)?,
        };
        if guard.as_ref().is_some_and(|guard| !guard.current()) {
            return Ok(());
        }
        if response.epoch != request.epoch || response.sequence != request.sequence {
            return Err(AgentError::Invalid("discovery report response mismatch"));
        }
        // In particular, never synchronously touch stdout after the shutdown
        // HTTP wait. A full pipe must not retain the daemon's state lock.
        if mode == DiscoveryReportMode::Shutdown {
            return Ok(());
        }
        #[derive(serde::Serialize)]
        struct Receipt<'a> {
            endpoint_discovery: &'a ReportEndpointDiscoveryRequest,
            reported_at: chrono::DateTime<chrono::Utc>,
        }
        receipt_writer(
            &serde_json::to_string(&Receipt {
                endpoint_discovery: &request,
                reported_at: response.reported_at,
            })
            .map_err(|_| AgentError::Invalid("cannot encode discovery receipt"))?,
        );
        Ok(())
    }

    fn runtime_client(&self) -> HttpClient {
        self.controller_client()
            .with_api_key(&self.identity.node_credential)
    }

    async fn managed_sync_mode(
        &mut self,
        manager: &mut Manager,
        dataplane: &mut Option<dataplane::State>,
        lease: &RuntimeLease,
    ) -> Result<
        (
            ControlSyncReceipts,
            Option<(mesh_execution::Execution, Instant)>,
            Instant,
        ),
        AgentError,
    > {
        if let Some(dataplane) = dataplane {
            if let Some(epoch) = lease.epoch {
                let runtime = meshway_protocol::MeshExecutionRequest {
                    epoch,
                    instance_id: lease.instance_id.clone(),
                    capabilities: vec![meshway_protocol::MeshExecutionCapability::Ipv4ExactPathV1],
                };
                if let Some(synced) = mesh_execution::sync(self, manager, &runtime).await? {
                    return Ok((
                        synced.receipt,
                        Some((
                            mesh_execution::Execution::Mesh(synced.snapshot),
                            synced.fresh_at,
                        )),
                        synced.fresh_at,
                    ));
                }
            }
            let synced = dataplane.sync(self, manager).await?;
            Ok((
                synced.receipt,
                Some((
                    mesh_execution::Execution::Legacy(synced.snapshot),
                    synced.fresh_at,
                )),
                synced.fresh_at,
            ))
        } else {
            let (receipt, fresh_at) = self.managed_sync(manager).await?;
            Ok((receipt, None, fresh_at))
        }
    }

    async fn managed_sync(
        &mut self,
        manager: &mut Manager,
    ) -> Result<(ControlSyncReceipts, Instant), AgentError> {
        let publication = self.publish_connection_identity().await?;
        manager
            .invalidate_dataplane_revision(publication.revision)
            .await;
        let configuration = self.sync().await?;
        #[cfg(unix)]
        manager.set_probe_inventory(
            self.cache
                .as_ref()
                .expect("freshly synced configuration")
                .public_inventory(),
        );
        let client = self.runtime_client();
        let mut saved = None;
        for _ in 0..3 {
            let bundle: meshway_protocol::AgentManagedConnectionsSnapshot =
                convert(client.get_agent_managed_connections().await?)?;
            let snapshot = &bundle.connections;
            let fresh_at = Instant::now();
            // Fence the old generation before material validation or persistence.
            #[cfg(unix)]
            manager.fence_latency_snapshot(snapshot).await;
            manager.invalidate_if_changed(snapshot);
            manager
                .invalidate_dataplane_revision(snapshot.payload.revision)
                .await;
            manager.validate_materials(&snapshot.payload, &bundle.materials)?;
            materials::persist(
                self,
                &bundle.network_id,
                snapshot.payload.revision,
                snapshot,
                &bundle.materials,
                &bundle.digest,
            )?;
            let state = self.connections.as_mut().expect("published connections");
            state.persist(&self.path, &self.identity, snapshot.clone())?;
            state.check(&self.path, &self.identity)?;
            local::ensure_durable(&cache_path(&self.path))?;
            local::ensure_durable(&connection_key::path(&self.path))?;
            let response = client
                .acknowledge_agent_connections(convert(
                    meshway_protocol::AcknowledgeAgentConnectionsRequest {
                        revision: snapshot.payload.revision,
                        digest: snapshot.digest.clone(),
                    },
                )?)
                .await;
            let ack: meshway_protocol::AcknowledgeAgentConnectionsResponse = match response {
                Err(e) if e.api().is_some_and(|e| e.status == 409) => {
                    manager.withdraw(Reason::InvalidConfiguration).await;
                    continue;
                }
                Err(e) => return Err(e.into()),
                Ok(value) => convert(value)?,
            };
            if ack.revision != snapshot.payload.revision
                || ack.digest != snapshot.digest
                || ack.status != meshway_protocol::ConnectionReceiptStatus::Saved
            {
                return Err(AgentError::Invalid("managed connection ACK mismatch"));
            }
            state.check(&self.path, &self.identity)?;
            manager.install_materials(bundle.materials);
            saved = Some((
                ConnectionSyncReceipt {
                    controller: self.identity.controller.clone(),
                    node_id: snapshot.payload.node_id.clone(),
                    stored_revision: snapshot.payload.revision,
                    digest: snapshot.digest.clone(),
                    configuration_valid: snapshot.payload.valid,
                    status: ack.status,
                    applied: false,
                },
                fresh_at,
            ));
            break;
        }
        let (connections, fresh_at) = saved.ok_or(AgentError::Http {
            status: 409,
            retryable: true,
        })?;
        Ok((
            ControlSyncReceipts {
                configuration,
                connections,
            },
            fresh_at,
        ))
    }
}

#[cfg(test)]
mod tests;
