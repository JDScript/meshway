//! The DNS owner is observed independently of packet I/O and TUN address DAD.
use super::Runtime;
use crate::host::Host;
use meshway_configuration::netmap::NodeNetmap;
use meshway_dns::{SystemDnsError, SystemDnsFailure, SystemDnsSettings};
use meshway_engine::Desired;
use meshway_model::recovery::{Health, Observation, Recovery, Step};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot, watch};

// A stalled observation withdraws query authority. Accepted backend writes
// remain owned by Runtime and are settled by its normal shutdown path.
const OBSERVE_TIMEOUT: Duration = Duration::from_secs(2);
/// A failed owner re-proves itself this often until it succeeds (fixed: the
/// resolved restart recovers within 5 s).
const RECOVERY_INTERVAL: Duration = Duration::from_secs(1);
/// A proven inconsistency is read again this often (the re-proof refuses,
/// never overwrites, settings it did not install).
const REREAD_PROVEN: Duration = Duration::from_secs(60);

/// The DNS owner's recovery (owner #9): unknown failures re-proved every
/// second without a total limit, proven ones every 60 s.
fn recovery() -> Recovery {
    Recovery::new(meshway_model::recovery::Policy {
        backoff: meshway_model::Backoff::new(RECOVERY_INTERVAL, RECOVERY_INTERVAL),
        horizon: meshway_model::recovery::Horizon::Unbounded,
        reread_blocked: Some(REREAD_PROVEN),
    })
}

/// The first wait before preparing a failed owner again: the worker's
/// reinstall of the same netmap (it doubles to 60 s).
const PREPARE_RETRY: Duration = Duration::from_secs(5);

/// Preparing the owner (owner #9 before it runs): its failures are judged
/// as the running owner's are; the worker prepares again no earlier than
/// its `recovery_at`, from 5 s doubling to 60 s, proven ones every 60 s.
fn preparation() -> Recovery {
    Recovery::new(meshway_model::recovery::Policy {
        backoff: meshway_model::Backoff::new(PREPARE_RETRY, REREAD_PROVEN),
        horizon: meshway_model::recovery::Horizon::Unbounded,
        reread_blocked: Some(REREAD_PROVEN),
    })
}

/// The DNS owner's state, as the packet plane and Status read it: one live
/// fact. It carries no authority (query authority is the published scope).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DnsFact {
    /// The fault that withdrew query authority; the plane's next hold
    /// clears it, after the owner re-proved itself (`health` healthy).
    pub fault: Option<SystemDnsError>,
    /// Counts faults: a fault with a later epoch than the plane
    /// acknowledged is a new failure for it.
    pub epoch: u64,
    /// The check the current fault (or the last failed hold or install)
    /// failed, for `lastErrorDetail`.
    pub detail: Option<SystemDnsFailure>,
    /// The owner's recovery of the fault: retrying (unknown) or blocked
    /// (proven); healthy once re-proved, or without a fault.
    pub health: Health,
    /// Other links' domains that coexist with the installed plan's fallback,
    /// as the last successful operation read them; empty while faulted.
    pub local_domains: Vec<meshway_protocol::ipc::LocalDnsDomain>,
}

/// Private I/O seam for the actor loop. Runtime is the sole production owner;
/// tests substitute observation latency, never DNS selection or installation.
trait Owner: Send + 'static {
    fn scopes(&self) -> watch::Sender<Option<meshway_dns::Scope>>;
    fn observe(&mut self) -> impl Future<Output = Result<(), SystemDnsError>> + Send;
    fn recover(&mut self) -> impl Future<Output = Result<(), SystemDnsError>> + Send;
    fn hold(
        &mut self,
        netmap: &NodeNetmap,
    ) -> impl Future<Output = Result<(), SystemDnsError>> + Send;
    fn install(
        &mut self,
        desired: Desired,
    ) -> impl Future<Output = Result<(), SystemDnsError>> + Send;
    fn shutdown(self, keep: bool) -> impl Future<Output = Result<(), SystemDnsError>> + Send;
    /// Why the last failed operation failed, if it recorded the check.
    fn failure(&self) -> Option<SystemDnsFailure> {
        None
    }
    /// The system installer's coexisting local domains.
    fn local_domains(&self) -> Vec<meshway_protocol::ipc::LocalDnsDomain> {
        Vec::new()
    }
}

/// The failure of `error` as recorded by the owner, or only its kind.
fn failure_of(owner: &impl Owner, error: SystemDnsError) -> SystemDnsFailure {
    owner
        .failure()
        .filter(|failure| failure.error == error)
        .unwrap_or_else(|| SystemDnsFailure::new(error, "unrecorded"))
}

fn warn_failure(operation: &'static str, message: &'static str, failure: &SystemDnsFailure) {
    tracing::warn!(
        error = ?failure.error,
        operation,
        check = failure.check,
        expected = failure.expected.as_deref(),
        actual = failure.actual.as_deref(),
        source = failure.source.as_deref(),
        "{message}"
    );
}

impl Owner for Runtime {
    fn scopes(&self) -> watch::Sender<Option<meshway_dns::Scope>> {
        self.scopes.clone()
    }
    async fn observe(&mut self) -> Result<(), SystemDnsError> {
        Runtime::observe(self).await
    }
    async fn recover(&mut self) -> Result<(), SystemDnsError> {
        Runtime::recover(self).await
    }
    async fn hold(&mut self, netmap: &NodeNetmap) -> Result<(), SystemDnsError> {
        Runtime::hold(self, netmap).await?;
        self.check(netmap)
    }
    async fn install(&mut self, desired: Desired) -> Result<(), SystemDnsError> {
        Runtime::install(self, desired).await
    }
    async fn shutdown(self, keep: bool) -> Result<(), SystemDnsError> {
        Runtime::shutdown(self, keep).await
    }
    fn failure(&self) -> Option<SystemDnsFailure> {
        Runtime::failure(self)
            .or_else(|| self.system.failure())
            .cloned()
    }
    fn local_domains(&self) -> Vec<meshway_protocol::ipc::LocalDnsDomain> {
        self.system.local_domains()
    }
}

enum Command {
    Hold(Box<NodeNetmap>, oneshot::Sender<Result<(), SystemDnsError>>),
    Install(Box<Desired>, oneshot::Sender<Result<(), SystemDnsError>>),
    Shutdown(bool, oneshot::Sender<Result<(), SystemDnsError>>),
}

struct Actor {
    commands: mpsc::Sender<Command>,
    scopes: watch::Sender<Option<meshway_dns::Scope>>,
    state: watch::Receiver<DnsFact>,
    task: tokio::task::JoinHandle<()>,
}

/// The actor's own recovery of its fault, published as one [`DnsFact`].
struct Faults {
    state: watch::Sender<DnsFact>,
    recovery: Recovery,
    /// When the failed owner re-proves itself next.
    recovery_at: tokio::time::Instant,
}

impl Faults {
    /// No failed preparation yet.
    fn preparing() -> Self {
        Self {
            state: watch::channel(DnsFact::default()).0,
            recovery: preparation(),
            recovery_at: tokio::time::Instant::now(),
        }
    }
    /// The owner failed `failure` now: query authority is withdrawn and the
    /// owner re-proves itself, every second, or every 60 s once proven.
    fn fail(&mut self, failure: SystemDnsFailure) {
        let now = tokio::time::Instant::now();
        let observation = if failure.proven() {
            Observation::proven(failure.check, &failure)
        } else {
            Observation::unknown(failure.check, &failure)
        };
        self.recovery_at = match self.recovery.observe(observation, now.into_std()) {
            Step::RetryAt(at) | Step::Block(Some(at)) => tokio::time::Instant::from_std(at),
            _ => now + RECOVERY_INTERVAL,
        };
        let health = self.recovery.health().clone();
        self.state.send_modify(|fact| {
            if fact.fault.is_none() || fact.health.is_healthy() {
                fact.epoch = fact.epoch.wrapping_add(1);
            }
            fact.fault = Some(failure.error);
            // Why before the fault itself: one send carries both.
            fact.detail = Some(failure);
            fact.health = health;
            fact.local_domains.clear();
        });
    }
    /// Publish the owner's coexisting local domains after a success; a
    /// send only when they changed.
    fn local(&self, owner: &impl Owner) {
        let domains = owner.local_domains();
        self.state.send_if_modified(|fact| {
            if fact.local_domains == domains {
                return false;
            }
            fact.local_domains = domains;
            true
        });
    }
    /// The failed owner re-proved its installation; the fault stays until
    /// the plane's next hold.
    fn recovered(&mut self) {
        self.recovery
            .observe(Observation::Holds, std::time::Instant::now());
        self.state.send_modify(|fact| fact.health = Health::Healthy);
    }
    /// A successful installation (it observes the system last) ended the
    /// fault.
    fn superseded(&mut self) {
        tracing::info!("DNS owner fault superseded by a successful installation");
        self.recovery
            .observe(Observation::Holds, std::time::Instant::now());
        self.state.send_modify(|fact| {
            fact.fault = None;
            fact.detail = None;
            fact.health = Health::Healthy;
        });
    }
    /// The fault that blocks a hold: one not re-proved yet.
    fn blocking(&self) -> Option<SystemDnsError> {
        let fact = self.state.borrow();
        fact.fault.filter(|_| !fact.health.is_healthy())
    }
    fn faulted(&self) -> bool {
        self.state.borrow().fault.is_some()
    }
}

impl Actor {
    fn start(mut runtime: impl Owner) -> Self {
        let (commands, mut received) = mpsc::channel(4);
        let (sender, state) = watch::channel(DnsFact::default());
        let scopes = runtime.scopes();
        let query_scopes = scopes.clone();
        let task = tokio::spawn(async move {
            let mut faults = Faults {
                state: sender,
                recovery: recovery(),
                recovery_at: tokio::time::Instant::now(),
            };
            let mut timer = tokio::time::interval(super::OBSERVE_INTERVAL);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut command_turn = true;
            loop {
                // An overdue tick and a command alternate deterministically.
                // At most one due observation separates already queued commands;
                // the four-slot FIFO therefore bounds shutdown's admission wait.
                // Commands cannot starve provenance observations either.
                let command = if command_turn {
                    match received.try_recv() {
                        Ok(command) => Some(Some(command)),
                        Err(mpsc::error::TryRecvError::Disconnected) => Some(None),
                        Err(mpsc::error::TryRecvError::Empty) => None,
                    }
                } else {
                    None
                };
                let command = match command {
                    Some(command) => Some(command),
                    None => tokio::select! { biased;
                        _ = timer.tick() => None,
                        command = received.recv() => Some(command),
                    },
                };
                if let Some(command) = command {
                    command_turn = false;
                    match command {
                        Some(Command::Hold(netmap, reply)) => {
                            let blocked = faults.blocking();
                            let result = match blocked {
                                Some(error) => Err(error),
                                None => {
                                    faults.state.send_modify(|fact| {
                                        fact.fault = None;
                                        fact.detail = None;
                                    });
                                    runtime.hold(&netmap).await
                                }
                            };
                            if result.is_ok() {
                                faults.local(&runtime);
                            }
                            if let Err(error) = result {
                                let failure = match blocked {
                                    Some(_) => faults.state.borrow().detail.clone(),
                                    None => Some(failure_of(&runtime, error)),
                                }
                                .unwrap_or_else(|| SystemDnsFailure::new(error, "unrecorded"));
                                warn_failure("hold", "DNS owner operation failed", &failure);
                                if blocked.is_none() {
                                    settle(&mut faults, &query_scopes, failure);
                                }
                            }
                            let _ = reply.send(result);
                            continue;
                        }
                        Some(Command::Install(desired, reply)) => {
                            let result = runtime.install(*desired).await;
                            if result.is_ok() {
                                faults.local(&runtime);
                            }
                            match result {
                                Err(error) => {
                                    let failure = failure_of(&runtime, error);
                                    warn_failure("install", "DNS owner operation failed", &failure);
                                    settle(&mut faults, &query_scopes, failure);
                                }
                                Ok(()) if !faults.faulted() => {
                                    faults
                                        .state
                                        .send_if_modified(|fact| fact.detail.take().is_some());
                                }
                                // The installation ended with its own
                                // observation and published query authority:
                                // it supersedes the earlier fault, as a hold
                                // does (no withdrawal for a fault the content
                                // installed since outlived).
                                Ok(()) => faults.superseded(),
                            }
                            let _ = reply.send(result);
                            continue;
                        }
                        Some(Command::Shutdown(keep, reply)) => {
                            let _ = reply.send(runtime.shutdown(keep).await);
                            return;
                        }
                        None => {
                            let _ = runtime.shutdown(true).await;
                            return;
                        }
                    }
                }
                command_turn = true;
                // A failed owner stays failed (the parent withdraws) until it
                // re-proves its installation against the backend as it is now;
                // then it waits for the parent's hold, observing nothing.
                let faulted = faults.faulted();
                if faulted && faults.blocking().is_none() {
                    continue;
                }
                let result = if faulted {
                    if tokio::time::Instant::now() < faults.recovery_at {
                        continue;
                    }
                    runtime
                        .recover()
                        .await
                        .map_err(|error| failure_of(&runtime, error))
                } else {
                    let started = tokio::time::Instant::now();
                    match tokio::time::timeout(OBSERVE_TIMEOUT, runtime.observe()).await {
                        Ok(result) => result.map_err(|error| failure_of(&runtime, error)),
                        // Abandoned mid-way: what it was waiting on is
                        // unknown; a stalled process (host-wide RTNL, CPU)
                        // shows as an elapsed time far beyond the deadline.
                        Err(_) => Err(SystemDnsFailure::new(
                            SystemDnsError::Backend,
                            "observe_deadline",
                        )
                        .expected(format!(
                            "an observation within {} ms",
                            OBSERVE_TIMEOUT.as_millis()
                        ))
                        .actual(format!(
                            "abandoned after {} ms",
                            started.elapsed().as_millis()
                        ))),
                    }
                };
                match result {
                    Err(failure) => {
                        // Each distinct failure once: a recovery that keeps
                        // failing the same check is not repeated every second.
                        if faults.state.borrow().detail.as_ref() != Some(&failure) {
                            warn_failure(
                                if faulted { "recover" } else { "observe" },
                                "DNS owner health check failed",
                                &failure,
                            );
                        }
                        meshway_dns::publish_scope(&query_scopes, None);
                        faults.fail(failure);
                    }
                    Ok(()) if faulted => {
                        tracing::info!("DNS owner recovered its system installation");
                        faults.recovered();
                    }
                    Ok(()) => faults.local(&runtime),
                }
            }
        });
        Self {
            commands,
            scopes,
            state,
            task,
        }
    }
    fn suspend(&self) {
        meshway_dns::publish_scope(&self.scopes, None);
    }
    async fn hold(&self, netmap: &NodeNetmap) -> Result<(), SystemDnsError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Hold(Box::new(netmap.clone()), reply))
            .await
            .map_err(|_| SystemDnsError::Backend)?;
        result.await.map_err(|_| SystemDnsError::Backend)?
    }
    async fn install(&self, desired: Desired) -> Result<(), SystemDnsError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Install(Box::new(desired), reply))
            .await
            .map_err(|_| SystemDnsError::Backend)?;
        result.await.map_err(|_| SystemDnsError::Backend)?
    }
    async fn shutdown(self, keep: bool) -> Result<(), SystemDnsError> {
        self.suspend();
        let (reply, result) = oneshot::channel();
        let sent = self.commands.send(Command::Shutdown(keep, reply)).await;
        let result = if sent.is_ok() {
            result.await.unwrap_or(Err(SystemDnsError::Backend))
        } else {
            Err(SystemDnsError::Backend)
        };
        self.task.await.map_err(|_| SystemDnsError::Backend)?;
        result
    }
}

/// A failed hold or installation. The system's failure (an installation,
/// a read, the backend) is the owner's fault like a failed observation: it
/// re-proves itself and the plane reinstalls once it has. A refused plan or
/// setting is the netmap's, recorded for the report only: another netmap
/// ends it, nothing here retries it.
fn settle(
    faults: &mut Faults,
    scopes: &watch::Sender<Option<meshway_dns::Scope>>,
    failure: SystemDnsFailure,
) {
    if failure.system() {
        meshway_dns::publish_scope(scopes, None);
        faults.fail(failure);
    } else {
        // The plane reads why before it sees the answer.
        faults.state.send_modify(|fact| fact.detail = Some(failure));
    }
}

#[cfg(test)]
mod tests;

pub(in crate::runtime::network) struct Attachment {
    settings: SystemDnsSettings,
    actor: Option<Actor>,
    queries: mpsc::Sender<super::mesh::Query>,
    requests: Option<mpsc::Receiver<super::mesh::Query>>,
    dispatcher: Option<Arc<meshway_dns::OwnerDispatcher>>,
    /// The DNS owner's exit resolver fact, for the plane's report.
    exit_dns: watch::Sender<Option<meshway_model::exit::ExitDns>>,
    /// Association completions of served answers, for Control.
    reports: mpsc::Sender<meshway_protocol::DnsAssociationReport>,
    report_receiver: Option<mpsc::Receiver<meshway_protocol::DnsAssociationReport>>,
    /// Preparing the owner while no actor runs: why it failed, judged and
    /// published as the running owner's faults are.
    prepare: Faults,
    /// The fault epoch the plane acknowledged: a fault of a later epoch is
    /// a new failure for it, even when the owner re-proved it before the
    /// plane looked (see `worker::lifecycle`).
    acknowledged: u64,
}
impl Attachment {
    pub fn new(settings: SystemDnsSettings) -> Self {
        let (queries, requests) = mpsc::channel(128);
        let (reports, report_receiver) = mpsc::channel(meshway_dns::MAX_PENDING_REPORTS);
        Self {
            reports,
            report_receiver: Some(report_receiver),
            settings,
            actor: None,
            queries,
            requests: Some(requests),
            dispatcher: None,
            exit_dns: watch::channel(None).0,
            prepare: Faults::preparing(),
            acknowledged: 0,
        }
    }
    /// The plane reacted to the owner's current fault: it withdrew for it,
    /// or reinstalls after its recovery. Only a later fault, or this one
    /// while it is not re-proved, is a failure for it then.
    pub fn acknowledge(&mut self) {
        if let Some(actor) = &self.actor {
            self.acknowledged = actor.state.borrow().epoch;
        }
    }
    /// The fault epoch the plane acknowledged last.
    pub fn acknowledged(&self) -> u64 {
        self.acknowledged
    }
    /// The checked reason of the current DNS fault, for the plane's report
    /// (`lastErrorDetail`).
    pub fn detail(&self) -> Option<String> {
        match &self.actor {
            Some(actor) => actor
                .state
                .borrow()
                .detail
                .as_ref()
                .map(ToString::to_string),
            None => self
                .prepare
                .state
                .borrow()
                .detail
                .as_ref()
                .map(ToString::to_string),
        }
    }
    /// The exit resolver fact of the installed content; absent unless it
    /// serves sources.
    pub fn exit_dns(&self) -> watch::Receiver<Option<meshway_model::exit::ExitDns>> {
        self.exit_dns.subscribe()
    }
    pub fn take_reports(&mut self) -> mpsc::Receiver<meshway_protocol::DnsAssociationReport> {
        self.report_receiver.take().expect("one DNS report owner")
    }
    pub fn take_queries(&mut self) -> mpsc::Receiver<super::mesh::Query> {
        self.requests.take().expect("one DNS packet owner")
    }
    pub fn dispatcher(&self) -> Option<Arc<meshway_dns::OwnerDispatcher>> {
        self.dispatcher.clone()
    }
    async fn owner(
        &mut self,
        host: &mut Box<dyn Host>,
    ) -> Result<Box<dyn crate::host::SystemDns>, SystemDnsError> {
        let prepared = host.prepare_dns().await;
        self.prepared(prepared)
    }
    /// One preparation's outcome: a failure is judged (`proven` or not)
    /// and published as the running owner's are; success ends it.
    fn prepared(
        &mut self,
        prepared: Result<Box<dyn crate::host::SystemDns>, SystemDnsFailure>,
    ) -> Result<Box<dyn crate::host::SystemDns>, SystemDnsError> {
        match prepared {
            Ok(owner) => {
                self.prepare = Faults::preparing();
                Ok(owner)
            }
            Err(failure) => {
                warn_failure("prepare", "DNS owner preparation failed", &failure);
                let error = failure.error;
                self.prepare.fail(failure);
                Err(error)
            }
        }
    }
    /// Prepare the owner if needed and hold the netmap's negative capture.
    pub async fn plan(
        &mut self,
        host: &mut Box<dyn Host>,
        netmap: &NodeNetmap,
    ) -> Result<(), SystemDnsError> {
        // A failed owner recovers itself (see `recovered`); until then its
        // hold fails with the current fault.
        if self.actor.is_none() {
            let system = self.owner(host).await?;
            let exchange =
                super::mesh::Exchange::new(host.resolver_exchange(), self.queries.clone());
            let runtime = Runtime::prepare(
                system,
                self.settings.clone(),
                exchange,
                self.exit_dns.clone(),
                self.reports.clone(),
            )
            .await?;
            self.dispatcher = Some(runtime.dispatcher.clone());
            self.actor = Some(Actor::start(runtime));
            self.acknowledged = 0;
        }
        self.actor
            .as_ref()
            .expect("prepared above")
            .hold(netmap)
            .await
    }
    pub async fn install(&self, desired: Desired) -> Result<(), SystemDnsError> {
        self.actor
            .as_ref()
            .ok_or(SystemDnsError::Backend)?
            .install(desired)
            .await
    }
    pub fn suspend(&self) {
        if let Some(actor) = &self.actor {
            actor.suspend();
        }
    }
    /// The running owner's state, live (the plane acts on its faults).
    pub fn guard(&self) -> Option<watch::Receiver<DnsFact>> {
        self.actor.as_ref().map(|actor| actor.state.clone())
    }
    /// The owner's state as Status reads it, live: the running owner's, or
    /// while none runs a failed preparation's (its check, detail and
    /// health: `blocked` when proven, as at run time).
    pub fn report_guard(&self) -> Option<watch::Receiver<DnsFact>> {
        self.guard().or_else(|| {
            self.prepare
                .faulted()
                .then(|| self.prepare.state.subscribe())
        })
    }
    /// When the worker prepares the failed owner again (see
    /// [`preparation`]); `None` while it runs or none failed.
    pub fn prepare_at(&self) -> Option<tokio::time::Instant> {
        (self.actor.is_none() && self.prepare.faulted()).then_some(self.prepare.recovery_at)
    }
    /// The Host proved its kernel ownership again in a new term: this owner's
    /// anchor and sockets belonged to the ended one. Stop it and keep the
    /// system installation as a restart keeps it; the next `plan` prepares
    /// the owner again on the new term (its anchor verified, not adopted by
    /// name).
    pub async fn detach(&mut self) -> Result<(), SystemDnsError> {
        self.suspend();
        self.dispatcher = None;
        match self.actor.take() {
            Some(actor) => actor.shutdown(true).await,
            None => Ok(()),
        }
    }
    /// Restore DNS before deleting the separately journalled kernel anchor.
    pub async fn down(&mut self, host: &mut Box<dyn Host>) -> Result<(), SystemDnsError> {
        self.suspend();
        self.dispatcher = None;
        if let Some(actor) = self.actor.take() {
            actor.shutdown(false).await?;
        } else {
            self.prepare = Faults::preparing();
            return host.restore_dns().await;
        }
        host.remove_dns_anchor()
            .await
            .map_err(|_| SystemDnsError::Backend)?;
        Ok(())
    }
    pub async fn shutdown(
        mut self,
        host: &mut Box<dyn Host>,
        keep: bool,
    ) -> Result<(), SystemDnsError> {
        if !keep {
            return self.down(host).await;
        }
        if let Some(actor) = self.actor.take() {
            actor.shutdown(true).await?;
        }
        Ok(())
    }
}
