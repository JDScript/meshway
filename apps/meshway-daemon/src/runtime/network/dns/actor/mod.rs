//! The DNS owner is observed independently of packet I/O and TUN address DAD.
use super::Runtime;
use meshway_configuration::netmap::NodeNetmap;
use meshway_dns::{
    SystemDnsBackend, SystemDnsError, SystemDnsFailure, SystemDnsOwner, SystemDnsReservation,
    SystemDnsSettings,
};
use meshway_engine::Desired;
use meshway_private_storage::Directory;
use meshway_underlay::Host;
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot, watch};

// A stalled observation withdraws query authority. Accepted backend writes
// remain owned by Runtime and are settled by its normal shutdown path.
const OBSERVE_TIMEOUT: Duration = Duration::from_secs(2);
/// A failed owner re-proves itself at most this often until it succeeds.
const RECOVERY_INTERVAL: Duration = Duration::from_secs(1);

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
}

enum Command {
    Hold(Box<NodeNetmap>, oneshot::Sender<Result<(), SystemDnsError>>),
    Install(Box<Desired>, oneshot::Sender<Result<(), SystemDnsError>>),
    Shutdown(bool, oneshot::Sender<Result<(), SystemDnsError>>),
}

struct Actor {
    commands: mpsc::Sender<Command>,
    scopes: watch::Sender<Option<meshway_dns::Scope>>,
    fault: watch::Receiver<Option<SystemDnsError>>,
    /// The check the current fault failed, for the plane's report.
    detail: watch::Receiver<Option<SystemDnsFailure>>,
    /// The failed owner re-proved its installation; the fault stays until
    /// the parent's next hold starts a new installation on it.
    recovered: watch::Receiver<bool>,
    task: tokio::task::JoinHandle<()>,
}
impl Actor {
    fn start(mut runtime: impl Owner) -> Self {
        let (commands, mut received) = mpsc::channel(4);
        let (faults, fault) = watch::channel(None);
        let (details, detail) = watch::channel(None::<SystemDnsFailure>);
        let (recoveries, recovered) = watch::channel(false);
        let scopes = runtime.scopes();
        let query_scopes = scopes.clone();
        let task = tokio::spawn(async move {
            let mut timer = tokio::time::interval(super::OBSERVE_INTERVAL);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut command_turn = true;
            let mut recovery_at = tokio::time::Instant::now();
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
                            let blocked = (*faults.borrow()).filter(|_| !*recoveries.borrow());
                            let result = match blocked {
                                Some(error) => Err(error),
                                None => {
                                    faults.send_replace(None);
                                    details.send_replace(None);
                                    recoveries.send_replace(false);
                                    runtime.hold(&netmap).await
                                }
                            };
                            if let Err(error) = result {
                                let failure = match blocked {
                                    Some(_) => details.borrow().clone(),
                                    None => Some(failure_of(&runtime, error)),
                                }
                                .unwrap_or_else(|| SystemDnsFailure::new(error, "unrecorded"));
                                warn_failure("hold", "DNS owner operation failed", &failure);
                            }
                            let _ = reply.send(result);
                            continue;
                        }
                        Some(Command::Install(desired, reply)) => {
                            let result = runtime.install(*desired).await;
                            if let Err(error) = result {
                                warn_failure(
                                    "install",
                                    "DNS owner operation failed",
                                    &failure_of(&runtime, error),
                                );
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
                let faulted = faults.borrow().is_some();
                if faulted && *recoveries.borrow() {
                    continue;
                }
                let result = if faulted {
                    if tokio::time::Instant::now() < recovery_at {
                        continue;
                    }
                    recovery_at = tokio::time::Instant::now() + RECOVERY_INTERVAL;
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
                        let error = failure.error;
                        // Each distinct failure once: a recovery that keeps
                        // failing the same check is not repeated every second.
                        if details.borrow().as_ref() != Some(&failure) {
                            warn_failure(
                                if faulted { "recover" } else { "observe" },
                                "DNS owner health check failed",
                                &failure,
                            );
                        }
                        meshway_dns::publish_scope(&query_scopes, None);
                        faults.send_replace(Some(error));
                        details.send_replace(Some(failure));
                    }
                    Ok(()) if faulted => {
                        tracing::info!("DNS owner recovered its system installation");
                        recoveries.send_replace(true);
                    }
                    Ok(()) => {}
                }
            }
        });
        Self {
            commands,
            scopes,
            fault,
            detail,
            recovered,
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

#[cfg(test)]
mod tests;

/// The DNS owner's health as the packet plane observes it. A fault the owner
/// already re-proved waits for the plane's next hold and is not a failure:
/// each fault is one failure and at most one recovery, never a level that
/// stays ready while the plane cannot reach the hold (another component's
/// installation failing first).
#[derive(Clone)]
pub(in crate::runtime::network) struct Health {
    fault: watch::Receiver<Option<SystemDnsError>>,
    recovered: watch::Receiver<bool>,
}
impl Health {
    pub fn new(
        fault: watch::Receiver<Option<SystemDnsError>>,
        recovered: watch::Receiver<bool>,
    ) -> Self {
        Self { fault, recovered }
    }
    /// Resolves while the owner has a fault it has not re-proved, or once
    /// the owner has gone.
    pub async fn failed(mut self) {
        loop {
            if self.fault.borrow_and_update().is_some() && !*self.recovered.borrow_and_update() {
                return;
            }
            tokio::select! {
                changed = self.fault.changed() => if changed.is_err() { return },
                changed = self.recovered.changed() => if changed.is_err() { return },
            }
        }
    }
    /// Resolves once a failed owner has re-proved its system installation.
    pub async fn recovered(mut self) {
        if self
            .recovered
            .wait_for(|recovered| *recovered)
            .await
            .is_err()
        {
            std::future::pending::<()>().await;
        }
    }
}

pub(in crate::runtime::network) struct Attachment {
    settings: SystemDnsSettings,
    directory: Arc<Directory>,
    reservation: Option<SystemDnsReservation>,
    actor: Option<Actor>,
    queries: mpsc::Sender<super::mesh::Query>,
    requests: Option<mpsc::Receiver<super::mesh::Query>>,
    dispatcher: Option<Arc<meshway_dns::OwnerDispatcher>>,
    /// The DNS owner's exit resolver fact, for the plane's report.
    exit_dns: watch::Sender<Option<meshway_model::exit::ExitDns>>,
    /// Association completions of served answers, for Control.
    reports: mpsc::Sender<meshway_protocol::DnsAssociationReport>,
    report_receiver: Option<mpsc::Receiver<meshway_protocol::DnsAssociationReport>>,
    /// Why preparing the owner failed, while no actor runs.
    prepare_failure: Option<SystemDnsFailure>,
}
impl Attachment {
    pub fn new(
        settings: SystemDnsSettings,
        directory: Arc<Directory>,
        reservation: Option<SystemDnsReservation>,
    ) -> Self {
        let (queries, requests) = mpsc::channel(128);
        let (reports, report_receiver) = mpsc::channel(meshway_dns::MAX_PENDING_REPORTS);
        Self {
            reports,
            report_receiver: Some(report_receiver),
            settings,
            directory,
            reservation,
            actor: None,
            queries,
            requests: Some(requests),
            dispatcher: None,
            exit_dns: watch::channel(None).0,
            prepare_failure: None,
        }
    }
    /// The checked reason of the current DNS fault, for the plane's report
    /// (`lastErrorDetail`).
    pub fn detail(&self) -> Option<String> {
        match &self.actor {
            Some(actor) => actor.detail.borrow().as_ref().map(ToString::to_string),
            None => self.prepare_failure.as_ref().map(ToString::to_string),
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
    async fn reserve(&mut self) -> Result<(), SystemDnsError> {
        if self.reservation.is_none() {
            self.reservation =
                Some(SystemDnsOwner::reserve(self.settings.clone(), self.directory.clone()).await?);
        }
        Ok(())
    }
    async fn owner(&mut self, host: &mut Host) -> Result<SystemDnsOwner, SystemDnsError> {
        self.reserve().await?;
        let reserved = self.reservation.as_ref().expect("reserved above");
        let anchor = if matches!(self.settings.backend, SystemDnsBackend::Resolved) {
            Some(host.dns_anchor(reserved).await.map_err(|error| {
                let failure = SystemDnsFailure::new(SystemDnsError::Backend, "anchor_create")
                    .expected("the Kernel's DNS anchor for this reservation")
                    .actual(error)
                    .source("Underlay Kernel");
                warn_failure("prepare", "DNS owner preparation failed", &failure);
                self.prepare_failure = Some(failure);
                SystemDnsError::Backend
            })?)
        } else {
            None
        };
        let prepared = SystemDnsOwner::prepare_reserved(
            self.reservation.take().expect("reserved above"),
            anchor,
        )
        .await;
        if let Err(failure) = &prepared {
            warn_failure("prepare", "DNS owner preparation failed", failure);
        }
        self.prepare_failure = prepared.as_ref().err().cloned();
        prepared.map_err(|failure| failure.error)
    }
    /// Prepare the owner if needed and hold the netmap's negative capture.
    pub async fn plan(
        &mut self,
        host: &mut Host,
        netmap: &NodeNetmap,
    ) -> Result<(), SystemDnsError> {
        // A failed owner recovers itself (see `recovered`); until then its
        // hold fails with the current fault.
        if self.actor.is_none() {
            let system = self.owner(host).await?;
            let exchange =
                super::mesh::Exchange::new(Arc::new(host.sockets()), self.queries.clone());
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
    pub fn guard(&self) -> Option<watch::Receiver<Option<SystemDnsError>>> {
        self.actor.as_ref().map(|actor| actor.fault.clone())
    }
    /// The running owner's health; `None` while no owner runs.
    pub fn health(&self) -> Option<Health> {
        self.actor
            .as_ref()
            .map(|actor| Health::new(actor.fault.clone(), actor.recovered.clone()))
    }
    /// See [`Health::failed`]; never while no owner runs.
    pub async fn failed(&self) {
        match self.health() {
            Some(health) => health.failed().await,
            None => std::future::pending().await,
        }
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
    pub async fn down(&mut self, host: &mut Host) -> Result<(), SystemDnsError> {
        self.suspend();
        self.dispatcher = None;
        if self.actor.is_none()
            && self.reservation.is_none()
            && !SystemDnsOwner::has_reservation(self.directory.clone()).await?
        {
            return Ok(());
        }
        if let Some(actor) = self.actor.take() {
            actor.shutdown(false).await?;
        } else {
            self.reserve().await?;
            if self
                .reservation
                .as_ref()
                .expect("reserved above")
                .has_backend_record()?
            {
                let mut owner = self.owner(host).await?;
                owner.shutdown(false).await?;
            } else {
                host.remove_dns_anchor()
                    .await
                    .map_err(|_| SystemDnsError::Backend)?;
                self.reservation
                    .as_mut()
                    .expect("reserved above")
                    .release_uninstalled()
                    .await?;
                self.reservation = None;
                return Ok(());
            }
        }
        host.remove_dns_anchor()
            .await
            .map_err(|_| SystemDnsError::Backend)?;
        Ok(())
    }
    pub async fn shutdown(mut self, host: &mut Host, keep: bool) -> Result<(), SystemDnsError> {
        if !keep {
            return self.down(host).await;
        }
        if let Some(actor) = self.actor.take() {
            actor.shutdown(true).await?;
        }
        Ok(())
    }
}
