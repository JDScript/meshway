//! Explicitly enabled adjacency application. Packet grants require a separately
//! validated coherent bundle and the managed-dataplane opt-in.

use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::PathBuf,
    time::Duration,
};

use futures_util::{StreamExt, stream};
use meshway_agent_connections::{controller_candidate, core_config};
use meshway_dataplane::PrivateKey;
#[cfg(test)]
use meshway_protocol::ConnectionTransport;
use meshway_protocol::{
    AgentConnectionsSnapshot, ConnectionRuntimeReason as Reason, ConnectionRuntimeState as Phase,
    NodeConnections,
};
use tokio::{
    sync::watch,
    task::JoinHandle,
    time::{self, Instant},
};

use crate::{
    AgentError,
    managed_dataplane::{PacketPlan, TunOptions, tun::Tun},
};

mod discovery;
#[cfg(unix)]
mod latency_history;
#[cfg(unix)]
mod latency_lifecycle;
#[cfg(test)]
mod managed_material_tests;
pub(crate) use meshway_agent_connections::{Failure, Observation};
use meshway_agent_connections::{session, sockets};
#[cfg(unix)]
mod probe;
mod profiles;
#[cfg(unix)]
pub(crate) use probe::Prober;
#[cfg(test)]
mod packet_tests;
#[cfg(all(test, unix))]
mod probe_tests;
#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
impl Manager {
    pub(crate) fn mock_tun(&mut self, tun: Tun) {
        self.tun = Some(tun);
    }
}

struct Running {
    /// Revision actually installed in this generation, independent of saved ACKs.
    revision: u32,
    deadline: watch::Sender<Option<Instant>>,
    observed: watch::Receiver<Observation>,
    task: JoinHandle<()>,
    discovery: watch::Receiver<discovery::Publication>,
    discovery_task: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub(crate) struct ReportGuard {
    deadline: watch::Receiver<Option<Instant>>,
    observed: watch::Receiver<Observation>,
    observation_expiry: Option<Instant>,
}

impl ReportGuard {
    /// Probe publications may be recaptured, but never a revoked, expired,
    /// renewed or replaced generation. The watch channels belong to one session.
    pub fn generation_current(&self) -> bool {
        self.deadline
            .borrow()
            .is_some_and(|deadline| deadline > Instant::now())
            && self.observed.borrow().state == Phase::Applied
            && self.observed.has_changed().is_ok()
            && !self.deadline.has_changed().unwrap_or(true)
    }

    pub fn current(&self) -> bool {
        self.generation_current()
            && !self.observed.has_changed().unwrap_or(true)
            && self
                .observation_expiry
                .is_none_or(|expiry| expiry > Instant::now())
    }

    pub async fn changed(&mut self) {
        if !self.current() {
            return;
        }
        let deadline = (*self.deadline.borrow()).expect("checked active deadline");
        let deadline = self
            .observation_expiry
            .map_or(deadline, |expiry| expiry.min(deadline));
        // Waiting must not mark the original proof witnesses seen. In
        // particular a renewal (including same-value notification / ABA) must
        // permanently fence this guard even after the waiter wakes.
        let mut deadlines = self.deadline.clone();
        let mut observations = self.observed.clone();
        tokio::select! {
            _ = time::sleep_until(deadline) => {},
            _ = deadlines.changed() => {},
            _ = observations.changed() => {},
        }
    }
}

impl Running {
    fn active(&self) -> bool {
        !self.task.is_finished()
            && self
                .deadline
                .borrow()
                .is_some_and(|deadline| deadline > Instant::now())
            && self.observed.borrow().state == Phase::Applied
    }

    async fn stop(&mut self) {
        let _ = self.deadline.send(None);
        if let Some(task) = &mut self.discovery_task {
            task.abort();
            let _ = task.await;
        }
        self.discovery_task = None;
        let _ = (&mut self.task).await;
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        // The independently owned watchdog completes socket/task shutdown even
        // when the controlling HTTP future is cancelled or returns an error.
        if let Some(running) = &self.running {
            let _ = running.deadline.send(None);
        }
    }
}

pub(crate) struct Manager {
    events: Option<tokio::sync::mpsc::Sender<crate::daemon::SessionEvent>>,
    #[cfg(target_os = "linux")]
    underlay_owner:
        Option<std::sync::Arc<tokio::sync::Mutex<meshway_agent_underlay::LinuxUnderlay>>>,
    #[cfg(target_os = "linux")]
    underlay_dns: Option<meshway_agent_underlay::SystemDnsSnapshot>,
    underlay_generation: Option<meshway_agent_underlay::PreparedGeneration>,
    socket_protection: std::sync::Arc<dyn sockets::Protection>,
    profiles: Option<PathBuf>,
    managed_materials: Vec<meshway_protocol::ManagedVlessMaterial>,
    lease: Duration,
    running: Option<Running>,
    semantic: Option<NodeConnections>,
    material: Option<Material>,
    observation: Observation,
    pub snapshot: Option<AgentConnectionsSnapshot>,
    dataplane: Option<DataplaneSettings>,
    packet_digest: Option<String>,
    tun: Option<Tun>,
    discovery_options: crate::EndpointDiscoveryOptions,
    discovery_generation: u32,
    discovery_timestamps: std::sync::Arc<discovery::TimestampGate>,
    #[cfg(unix)]
    probes: probe::Publisher,
    #[cfg(unix)]
    latency: latency_lifecycle::Owner,
}

struct DataplaneSettings {
    options: TunOptions,
    #[cfg(any(not(target_os = "linux"), test))]
    controller: String,
    #[cfg(any(not(target_os = "linux"), test))]
    may_reuse_guard: bool,
}

#[derive(PartialEq, Eq)]
struct Material {
    profiles: [u8; 32],
    targets: BTreeMap<String, Vec<SocketAddr>>,
}

impl Manager {
    pub fn new(profiles: Option<PathBuf>, lease: Duration) -> Self {
        Self {
            events: None,
            #[cfg(target_os = "linux")]
            underlay_owner: None,
            #[cfg(target_os = "linux")]
            underlay_dns: None,
            underlay_generation: None,
            socket_protection: std::sync::Arc::new(sockets::SystemUnderlay),
            profiles,
            managed_materials: Vec::new(),
            lease,
            running: None,
            semantic: None,
            material: None,
            observation: Observation::empty(Phase::Withdrawn, Reason::None),
            snapshot: None,
            dataplane: None,
            packet_digest: None,
            tun: None,
            discovery_options: crate::EndpointDiscoveryOptions::default(),
            discovery_generation: 0,
            discovery_timestamps: std::sync::Arc::default(),
            #[cfg(unix)]
            probes: probe::Publisher::new(),
            #[cfg(unix)]
            latency: latency_lifecycle::Owner::default(),
        }
    }

    pub fn new_dataplane(
        profiles: Option<PathBuf>,
        lease: Duration,
        options: TunOptions,
        controller: String,
        may_reuse_guard: bool,
    ) -> Self {
        let mut manager = Self::new(profiles, lease);
        #[cfg(all(target_os = "linux", not(test)))]
        let _ = (controller, may_reuse_guard);
        manager.dataplane = Some(DataplaneSettings {
            options,
            #[cfg(any(not(target_os = "linux"), test))]
            controller,
            #[cfg(any(not(target_os = "linux"), test))]
            may_reuse_guard,
        });
        manager
    }

    pub fn enable_discovery(&mut self, options: crate::EndpointDiscoveryOptions) {
        self.discovery_options = options;
    }

    pub(crate) fn set_events(
        &mut self,
        events: tokio::sync::mpsc::Sender<crate::daemon::SessionEvent>,
    ) {
        self.events = Some(events);
    }

    /// Routing owner supplies an already prepared dual-stack capability. No
    /// capture is activated here. Withdraw/join preserves the business sink.
    #[allow(dead_code)]
    pub(crate) async fn install_prepared_underlay(
        &mut self,
        generation: meshway_agent_underlay::PreparedGeneration,
        upstreams: meshway_agent_underlay::SystemDnsSnapshot,
    ) {
        self.withdraw(Reason::ShuttingDown).await;
        self.socket_protection = std::sync::Arc::new(
            crate::control::transport::ManagedProtection::new(generation.factory(), upstreams),
        );
        // Drop retires the former generation only after its session joined.
        self.underlay_generation = Some(generation);
    }

    #[cfg(target_os = "linux")]
    pub(crate) async fn install_linux_underlay(
        &mut self,
        owner: std::sync::Arc<tokio::sync::Mutex<meshway_agent_underlay::LinuxUnderlay>>,
    ) -> Result<(), AgentError> {
        let (generation, upstreams) = {
            let owner = owner.lock().await;
            (owner.socket_generation().await?, owner.system_dns()?)
        };
        self.underlay_dns = Some(upstreams.clone());
        self.install_prepared_underlay(generation, upstreams).await;
        self.underlay_owner = Some(owner);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub(crate) async fn refresh_underlay(&mut self) -> Result<(), AgentError> {
        if let Some(owner) = self.underlay_owner.clone() {
            let upstreams = owner.lock().await.system_dns()?;
            if self.underlay_dns.as_ref() != Some(&upstreams)
                || self.socket_protection.check_active().is_err()
            {
                self.install_linux_underlay(owner).await?;
            }
        }
        Ok(())
    }

    pub fn discovery_snapshot(
        &self,
    ) -> (
        meshway_protocol::EndpointDiscoveryState,
        Vec<meshway_protocol::UdpSocketDiscovery>,
        Option<ReportGuard>,
    ) {
        use meshway_protocol::EndpointDiscoveryState as State;
        if self.discovery_options.servers.is_empty() {
            return (State::Disabled, Vec::new(), self.report_snapshot().1);
        }
        let Some(running) = self.running.as_ref().filter(|r| r.active()) else {
            return (State::Withdrawn, Vec::new(), None);
        };
        let mut publication = running.discovery.borrow().clone();
        if !self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.payload.revision == publication.source.revision
                && snapshot.digest == publication.source.digest
        }) {
            return (State::Withdrawn, Vec::new(), None);
        }
        publication.prune();
        let (_, mut guard) = self.report_snapshot();
        if let Some(guard) = &mut guard {
            guard.observation_expiry = publication.earliest_expiry();
        }
        (publication.state, publication.sockets, guard)
    }

    /// Complete generation teardown immediately after validation, before disk
    /// persistence or any further HTTP request can stall. Runtime shutdown also
    /// destroys all established endpoint sessions and queued plaintext delivery.
    pub async fn invalidate_dataplane(
        &mut self,
        snapshot: &meshway_protocol::AgentDataplaneSnapshot,
    ) {
        if self.packet_digest.as_ref() != Some(&snapshot.digest)
            || !snapshot.payload.access_valid
            || !snapshot.payload.configuration.policy_valid
            || !snapshot.payload.connections.valid
        {
            self.withdraw(Reason::InvalidConfiguration).await;
        }
    }

    /// A validated publication can prove this generation stale before the next
    /// coherent GET completes. It never renews a lease or relies on disk receipts.
    pub async fn invalidate_dataplane_revision(&mut self, revision: u32) {
        if self
            .running
            .as_ref()
            .is_some_and(|running| revision > running.revision)
        {
            self.withdraw(Reason::InvalidConfiguration).await;
        }
    }

    pub async fn reconcile_dataplane(
        &mut self,
        snapshot: meshway_protocol::AgentDataplaneSnapshot,
        key: PrivateKey,
        fresh_at: Instant,
    ) -> Result<(), AgentError> {
        self.invalidate_dataplane(&snapshot).await;
        self.prepare_dataplane_sink(&snapshot).await?;
        self.packet_digest = Some(snapshot.digest);
        let payload = snapshot.payload;
        let connections = AgentConnectionsSnapshot {
            digest: meshway_protocol::connection_digest(&payload.connections)
                .map_err(|_| AgentError::Invalid("cannot hash coherent adjacency projection"))?,
            payload: payload.connections.clone(),
        };
        if !payload.access_valid
            || !payload.configuration.policy_valid
            || !payload.connections.valid
        {
            self.snapshot = Some(connections);
            self.withdraw(Reason::InvalidConfiguration).await;
            self.observation.state = Phase::Error;
            return Ok(());
        }
        let options = &self
            .dataplane
            .as_ref()
            .ok_or(AgentError::Invalid("dataplane was not explicitly enabled"))?
            .options;
        let plan = PacketPlan::compile(&payload, options.mtu)?;
        self.reconcile_inner(connections, key, fresh_at, Some(plan))
            .await
    }

    /// Install the drop sink before saved ACK HTTP waits, even when a structurally
    /// valid bundle publishes invalid/withdrawn source intent. It authorizes no I/O.
    pub async fn prepare_dataplane_sink(
        &mut self,
        snapshot: &meshway_protocol::AgentDataplaneSnapshot,
    ) -> Result<(), AgentError> {
        let result = self.prepare_dataplane_sink_inner(snapshot).await;
        if let Err(error) = &result {
            let reason = if matches!(error, AgentError::DataplaneResolveFailed) {
                Reason::ResolveFailed
            } else {
                Reason::InvalidConfiguration
            };
            // Preparation never authorizes renewal. Retain the owned route sink,
            // but close all old channels/packet tasks before returning to retry.
            self.withdraw(reason).await;
        }
        result
    }

    async fn prepare_dataplane_sink_inner(
        &mut self,
        snapshot: &meshway_protocol::AgentDataplaneSnapshot,
    ) -> Result<(), AgentError> {
        let settings = self
            .dataplane
            .as_ref()
            .ok_or(AgentError::Invalid("dataplane was not explicitly enabled"))?;
        let plan = PacketPlan::sink(&snapshot.payload, settings.options.mtu)?;
        self.prepare_packet_sink(plan, &snapshot.payload.connections)
            .await
    }

    async fn prepare_packet_sink(
        &mut self,
        plan: PacketPlan,
        connections: &NodeConnections,
    ) -> Result<(), AgentError> {
        self.dataplane
            .as_ref()
            .ok_or(AgentError::Invalid("dataplane was not explicitly enabled"))?;
        for listener in &connections.listeners {
            let address: SocketAddr = listener
                .bind
                .parse()
                .map_err(|_| AgentError::Invalid("invalid transport listener"))?;
            if crate::managed_dataplane::in_prefix(address.ip(), plan.prefix) {
                return Err(AgentError::Invalid(
                    "transport listener overlaps mesh IPv4 pool",
                ));
            }
        }
        if self.tun.as_ref().is_some_and(|tun| !tun.matches(&plan)) {
            return Err(AgentError::Invalid("managed TUN binding changed"));
        }
        let targets = match time::timeout(
            Duration::from_secs(5),
            resolve_targets(connections, self.socket_protection.as_ref()),
        )
        .await
        {
            Ok(Ok(targets)) => targets,
            Ok(Err(Failure(Reason::ResolveFailed))) | Err(_) => {
                return Err(AgentError::DataplaneResolveFailed);
            }
            Ok(Err(_)) => {
                return Err(AgentError::Invalid(
                    "invalid transport endpoint preparation",
                ));
            }
        };
        plan.check_targets(&targets)?;
        if self.tun.is_some() {
            return Ok(());
        }
        self.tun = Some(self.open_tun(&plan).await?);
        Ok(())
    }

    async fn open_tun(&self, plan: &PacketPlan) -> Result<Tun, AgentError> {
        let settings = self
            .dataplane
            .as_ref()
            .ok_or(AgentError::Invalid("dataplane not enabled"))?;
        #[cfg(target_os = "linux")]
        if let Some(owner) = &self.underlay_owner {
            return Tun::open_with_underlay(&settings.options, plan, &mut *owner.lock().await)
                .await;
        }
        #[cfg(all(target_os = "linux", not(test)))]
        return Err(AgentError::Invalid(
            "dataplane requires prepared namespace ownership",
        ));
        #[cfg(any(not(target_os = "linux"), test))]
        Tun::open(
            &settings.options,
            plan,
            &settings.controller,
            settings.may_reuse_guard,
        )
        .await
    }

    pub async fn invalidate_mesh_execution(
        &mut self,
        snapshot: &meshway_protocol::MeshExecutionSnapshot,
    ) {
        if self.packet_digest.as_ref() != Some(&snapshot.digest)
            || !snapshot.payload.valid
            || !snapshot.payload.connections.valid
        {
            self.withdraw(Reason::InvalidConfiguration).await;
        }
    }

    pub async fn prepare_mesh_sink(
        &mut self,
        snapshot: &meshway_protocol::MeshExecutionSnapshot,
    ) -> Result<(), AgentError> {
        let mtu = self
            .dataplane
            .as_ref()
            .ok_or(AgentError::Invalid("dataplane was not explicitly enabled"))?
            .options
            .mtu;
        let plan = PacketPlan::sink_mesh(&snapshot.payload, mtu)?;
        if snapshot.payload.valid && snapshot.payload.connections.valid {
            // ACK is saved/validated, not applied. Still reject an impossible
            // exact-path MTU/admission plan before issuing that saved receipt.
            PacketPlan::compile_mesh(&snapshot.payload, mtu)?;
        }
        let result = self
            .prepare_packet_sink(plan, &snapshot.payload.connections)
            .await;
        if result.is_err() {
            self.withdraw(Reason::InvalidConfiguration).await;
        }
        result
    }

    pub async fn reconcile_mesh_execution(
        &mut self,
        snapshot: meshway_protocol::MeshExecutionSnapshot,
        key: PrivateKey,
        fresh_at: Instant,
    ) -> Result<(), AgentError> {
        self.invalidate_mesh_execution(&snapshot).await;
        self.prepare_mesh_sink(&snapshot).await?;
        self.packet_digest = Some(snapshot.digest);
        let payload = snapshot.payload;
        let connections = AgentConnectionsSnapshot {
            digest: meshway_protocol::connection_digest(&payload.connections)
                .map_err(|_| AgentError::Invalid("cannot hash Mesh adjacency"))?,
            payload: payload.connections.clone(),
        };
        if !payload.valid || !payload.connections.valid {
            self.snapshot = Some(connections);
            self.withdraw(Reason::InvalidConfiguration).await;
            self.observation.state = Phase::Error;
            return Ok(());
        }
        let plan = PacketPlan::compile_mesh(
            &payload,
            self.dataplane
                .as_ref()
                .expect("explicit dataplane")
                .options
                .mtu,
        )?;
        self.reconcile_inner(connections, key, fresh_at, Some(plan))
            .await
    }

    pub fn observation(&self) -> Observation {
        self.running
            .as_ref()
            .map(|running| {
                let observed = running.observed.borrow().clone();
                if running.active() || observed.state != Phase::Applied {
                    observed
                } else {
                    let reason = match *running.deadline.borrow() {
                        None => Reason::ShuttingDown,
                        Some(deadline) if deadline <= Instant::now() => Reason::LeaseExpired,
                        _ => Reason::InternalError,
                    };
                    Observation::empty(Phase::Withdrawn, reason)
                }
            })
            .unwrap_or_else(|| self.observation.clone())
    }

    /// Per-generation watch channels are never shared with a replacement. This
    /// captures the actual proof snapshot and watch version in one read, fences
    /// the HTTP wait against later probe outcomes/withdrawal, and never renews.
    /// A caller may recapture after a probe update only while the old guard's
    /// generation remains current, within its original bounded HTTP budget.
    pub fn report_snapshot(&self) -> (Observation, Option<ReportGuard>) {
        if let Some(running) = &self.running
            && running.active()
        {
            let mut observed = running.observed.clone();
            let value = observed.borrow_and_update().clone();
            let deadline = running.deadline.subscribe();
            (
                value,
                Some(ReportGuard {
                    deadline,
                    observed,
                    observation_expiry: None,
                }),
            )
        } else {
            (self.observation(), None)
        }
    }

    /// Called immediately after persist/validation, before the saved ACK HTTP
    /// request can stall. Newly removed grants cannot survive that request.
    pub fn invalidate_if_changed(&self, snapshot: &AgentConnectionsSnapshot) {
        let mut semantic = snapshot.payload.clone();
        semantic.revision = 0;
        if (!semantic.valid
            || self.semantic.as_ref().is_some_and(|old| old != &semantic)
            || (!self.discovery_options.servers.is_empty()
                && self.snapshot.as_ref().is_some_and(|old| {
                    old.payload.revision != snapshot.payload.revision
                        || old.digest != snapshot.digest
                })))
            && let Some(running) = &self.running
        {
            let _ = running.deadline.send(None);
        }
    }

    pub async fn withdraw(&mut self, reason: Reason) {
        // Fence packet/carrier work before any await. Keep the exact generation
        // and its join handles in Manager until both joins complete; cancelling
        // this future must leave a re-entrant teardown, never an empty slot.
        if let Some(running) = &self.running {
            let _ = running.deadline.send(None);
        }
        #[cfg(unix)]
        self.probes.withdraw();
        #[cfg(unix)]
        self.stop_latency_history().await;
        if let Some(running) = &mut self.running {
            running.stop().await;
        }
        self.running = None;
        #[cfg(target_os = "linux")]
        if let Some(owner) = &self.underlay_owner
            && let Err(error) = owner.lock().await.withdraw_tun().await
        {
            // No fallback or foreign cleanup. The retained sink/device still
            // reject; the journal blocks a future activation until reconciled.
            eprintln!("underlay TUN withdrawal failed; resources retained: {error}");
        }
        self.material = None;
        self.observation = Observation::empty(Phase::Withdrawn, reason);
    }

    pub fn validate_materials(
        &self,
        intent: &NodeConnections,
        materials: &[meshway_protocol::ManagedVlessMaterial],
    ) -> Result<(), AgentError> {
        profiles::load_combined(self.profiles.as_deref(), intent, materials)
            .map(|_| ())
            .map_err(|_| AgentError::Invalid("invalid managed carrier materials"))
    }

    pub fn install_materials(&mut self, materials: Vec<meshway_protocol::ManagedVlessMaterial>) {
        self.managed_materials = materials;
    }

    /// Only called after a fresh validated sync and repeated disk/binding/fsync
    /// checks under the Agent state lock. Reports never call this method.
    pub async fn reconcile(
        &mut self,
        snapshot: AgentConnectionsSnapshot,
        key: PrivateKey,
        fresh_at: Instant,
    ) {
        // This entry point always has allowed_paths=[], even in feature builds.
        let _ = self.reconcile_inner(snapshot, key, fresh_at, None).await;
    }

    async fn reconcile_inner(
        &mut self,
        snapshot: AgentConnectionsSnapshot,
        key: PrivateKey,
        fresh_at: Instant,
        packet: Option<PacketPlan>,
    ) -> Result<(), AgentError> {
        let deadline = fresh_at + self.lease;
        let revision = snapshot.payload.revision;
        let mut semantic = snapshot.payload.clone();
        semantic.revision = 0;
        if !self.discovery_options.servers.is_empty()
            && self.snapshot.as_ref().is_some_and(|old| {
                old.payload.revision != snapshot.payload.revision || old.digest != snapshot.digest
            })
        {
            self.withdraw(Reason::None).await;
        }
        self.snapshot = Some(snapshot);
        if deadline <= Instant::now() {
            self.withdraw(Reason::LeaseExpired).await;
            return Ok(());
        }
        if self.semantic.as_ref() != Some(&semantic) {
            self.withdraw(Reason::None).await;
            self.semantic = Some(semantic.clone());
        }
        if !semantic.valid {
            self.withdraw(Reason::InvalidConfiguration).await;
            self.observation.state = Phase::Error;
            return Ok(());
        }
        let prepared = time::timeout_at(
            deadline.min(Instant::now() + Duration::from_secs(5)),
            async {
                let profile_path = self.profiles.clone();
                let profile_intent = semantic.clone();
                let materials = self.managed_materials.clone();
                let profiles = tokio::task::spawn_blocking(move || {
                    profiles::load_combined(profile_path.as_deref(), &profile_intent, &materials)
                })
                .await
                .map_err(|_| Failure(Reason::InternalError))??;
                let targets = resolve_targets(&semantic, self.socket_protection.as_ref()).await?;
                Ok::<_, Failure>((profiles, targets))
            },
        )
        .await
        .unwrap_or(Err(Failure(Reason::ResolveFailed)));
        let (profiles, targets) = match prepared {
            Ok(value) => value,
            Err(Failure(reason)) => {
                self.withdraw(reason).await;
                self.observation.state = Phase::Error;
                return Ok(());
            }
        };
        let material = Material {
            profiles: profiles.fingerprint,
            targets: targets.clone(),
        };
        if let Some(plan) = &packet
            && let Err(error) = plan.check_targets(&targets)
        {
            self.withdraw(Reason::InvalidConfiguration).await;
            return Err(error.into());
        }
        if self.material.as_ref() == Some(&material)
            && self.running.as_ref().is_some_and(Running::active)
        {
            // Only completed, safe preparation may renew. An already-expired
            // generation cannot be revived by publication or a DNS retry.
            let _ = self
                .running
                .as_ref()
                .expect("active generation")
                .deadline
                .send(Some(deadline));
            #[cfg(unix)]
            self.probes.refresh_source_revision(revision);
            return Ok(());
        }
        self.withdraw(Reason::None).await;
        if let Some(plan) = &packet {
            if self.tun.as_ref().is_some_and(|tun| !tun.matches(plan)) {
                return Err(AgentError::Invalid(
                    "managed TUN address, prefix or MTU changed",
                ));
            }
            if self.tun.is_none() {
                self.tun = Some(self.open_tun(plan).await?);
            }
            #[cfg(target_os = "linux")]
            if let Some(owner) = &self.underlay_owner {
                let binding = self
                    .tun
                    .as_ref()
                    .expect("configured TUN")
                    .underlay_binding()?;
                owner
                    .lock()
                    .await
                    .activate_tun(&[meshway_agent_underlay::CapturePrefix {
                        prefix: plan.prefix.into(),
                        tun: Some(&binding),
                    }])
                    .await?;
            }
            // Flush the bounded kernel queue while no generation owns the fd.
            meshway_dataplane::packet_io::Device::drain(
                self.tun.as_ref().expect("configured TUN").device.as_ref(),
            )?;
        }
        if deadline <= Instant::now() {
            self.withdraw(Reason::LeaseExpired).await;
            return Ok(());
        }
        let result = time::timeout_at(deadline, async {
            let mut runtime_config = core_config(&semantic, key, &targets)?;
            if let Some(plan) = &packet {
                plan.augment(&mut runtime_config)
                    .map_err(|_| Failure(Reason::InvalidConfiguration))?;
            }
            session::Session::start_prepared(
                &semantic,
                profiles,
                &targets,
                runtime_config,
                self.socket_protection.clone(),
            )
            .await
        })
        .await;
        match result {
            Ok(Ok(mut session)) => {
                if deadline <= Instant::now() {
                    session.shutdown().await;
                    self.observation = Observation::empty(Phase::Withdrawn, Reason::LeaseExpired);
                    return Ok(());
                }
                let initial = session.initial_observation(&semantic);
                let (observations, observed) = watch::channel(initial);
                let (renew, deadlines) = watch::channel(Some(deadline));
                let source = discovery::Source {
                    revision,
                    digest: self
                        .snapshot
                        .as_ref()
                        .expect("current source")
                        .digest
                        .clone(),
                };
                let enabled = !self.discovery_options.servers.is_empty();
                let (discovered, discovery) = watch::channel(discovery::Publication::new(
                    source.clone(),
                    if enabled {
                        meshway_protocol::EndpointDiscoveryState::Probing
                    } else {
                        meshway_protocol::EndpointDiscoveryState::Disabled
                    },
                ));
                let discovery_task = if enabled {
                    self.discovery_generation =
                        self.discovery_generation
                            .checked_add(1)
                            .ok_or(AgentError::Invalid(
                                "endpoint discovery generation exhausted",
                            ))?;
                    let runtime = std::sync::Arc::downgrade(&session.runtime);
                    let intent = semantic.clone();
                    let servers = self.discovery_options.servers.clone();
                    let generation = self.discovery_generation;
                    let deadlines = renew.subscribe();
                    let observed = observed.clone();
                    let timestamps = self.discovery_timestamps.clone();
                    Some(tokio::spawn(async move {
                        discovery::run(
                            discovery::Carrier {
                                runtime,
                                intent,
                                generation,
                                source,
                                timestamps,
                            },
                            servers,
                            deadlines,
                            observed,
                            discovered,
                        )
                        .await;
                    }))
                } else {
                    None
                };
                #[cfg(unix)]
                self.probes
                    .publish(&session.runtime, &semantic, &renew, &observed, revision);
                let packet_io = packet.map(|plan| {
                    let tun = self.tun.as_ref().expect("configured TUN");
                    (tun.device.clone(), tun.name.clone(), plan)
                });
                #[cfg(target_os = "linux")]
                let underlay = self.underlay_owner.clone();
                let events = self.events.clone();
                let packet_runtime = session.runtime.clone();
                let packet_deadline = deadlines.clone();
                let observed_owner = observations.clone();
                let task = tokio::spawn(async move {
                    let reason = {
                        let connection = session.run(semantic, deadlines, observations);
                        let packet = async {
                            match &packet_io {
                                Some((device, name, plan)) => {
                                    crate::managed_dataplane::packet::run(
                                        device.clone(),
                                        packet_runtime,
                                        plan.clone(),
                                        packet_deadline,
                                        name,
                                        events.as_ref(),
                                    )
                                    .await
                                }
                                None => std::future::pending().await,
                            }
                        };
                        tokio::pin!(connection, packet);
                        tokio::select! {
                            biased;
                            reason = &mut connection => reason,
                            _ = &mut packet => Reason::InternalError,
                        }
                    };
                    observed_owner.send_replace(Observation::empty(Phase::Withdrawn, reason));
                    if let Some((_, name, plan)) = &packet_io {
                        crate::managed_dataplane::tun::proof(
                            name,
                            plan,
                            false,
                            match reason {
                                Reason::LeaseExpired => "lease_expired",
                                Reason::ShuttingDown => "generation_withdrawn",
                                _ => "runtime_or_device_failure",
                            },
                            events.as_ref(),
                        );
                    }
                    session.shutdown().await;
                    // Session owns packet/carrier teardown. Even cancellation of
                    // the controlling HTTP future cannot skip this joined fence.
                    #[cfg(target_os = "linux")]
                    if let Some(owner) = underlay
                        && let Err(error) = owner.lock().await.withdraw_tun().await
                    {
                        eprintln!(
                            "underlay watchdog withdrawal failed; resources retained: {error}"
                        );
                    }
                });
                self.running = Some(Running {
                    revision,
                    deadline: renew,
                    observed,
                    task,
                    discovery,
                    discovery_task,
                });
                self.material = Some(material);
            }
            error => {
                let reason = match error {
                    Ok(Err(Failure(reason))) => reason,
                    _ => Reason::LeaseExpired,
                };
                self.withdraw(reason).await;
                self.observation = Observation::empty(Phase::Error, reason);
            }
        }
        Ok(())
    }
}

async fn resolve_targets(
    intent: &NodeConnections,
    protection: &dyn sockets::Protection,
) -> Result<BTreeMap<String, Vec<SocketAddr>>, Failure> {
    stream::iter(intent.outgoing.iter().map(|candidate| async move {
        let resolved = time::timeout(
            Duration::from_secs(2),
            protection.resolve(candidate.endpoint.host.as_str(), candidate.endpoint.port),
        )
        .await
        .map_err(|_| Failure(Reason::ResolveFailed))?
        .map_err(|_| Failure(Reason::ResolveFailed))?;
        let mut addresses = BTreeSet::new();
        for address in resolved.into_iter().take(32) {
            let ip = match address.ip() {
                std::net::IpAddr::V6(ip) => ip
                    .to_ipv4_mapped()
                    .map(std::net::IpAddr::V4)
                    .unwrap_or(ip.into()),
                ip => ip,
            };
            if address.port() != 0
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !matches!(ip, std::net::IpAddr::V4(ip) if ip.is_broadcast())
            {
                addresses.insert(SocketAddr::new(ip, address.port()));
            }
        }
        if addresses.is_empty() {
            return Err(Failure(Reason::ResolveFailed));
        }
        Ok((
            candidate.id.clone(),
            addresses.into_iter().take(8).collect(),
        ))
    }))
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await
    .into_iter()
    .collect()
}
