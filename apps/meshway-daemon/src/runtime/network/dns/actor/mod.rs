//! The DNS owner is observed independently of packet I/O and TUN address DAD.
use super::{Runtime, intents::Plan};
use meshway_authority::ExecutionSnapshot;
use meshway_configuration::execution::NodeConfiguration;
use meshway_dns::{
    ResolverSource, SystemDnsBackend, SystemDnsError, SystemDnsOwner, SystemDnsReservation,
    SystemDnsSettings,
};
use meshway_model::RequestId;
use meshway_private_storage::Directory;
use meshway_underlay::Host;
use std::{collections::BTreeMap, future::Future, sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot, watch};

// A stalled observation withdraws query authority. Accepted backend writes
// remain owned by Runtime and are settled by its normal shutdown path.
const OBSERVE_TIMEOUT: Duration = Duration::from_secs(2);

/// Private I/O seam for the actor loop. Runtime is the sole production owner;
/// tests substitute observation latency, never DNS selection or installation.
trait Owner: Send + 'static {
    fn scopes(&self) -> watch::Sender<Option<meshway_dns::Scope>>;
    fn observe(&mut self) -> impl Future<Output = Result<(), SystemDnsError>> + Send;
    fn hold(
        &mut self,
        cfg: &NodeConfiguration,
    ) -> impl Future<Output = Result<Vec<meshway_dns::ResolverDemand>, SystemDnsError>> + Send;
    fn install(
        &mut self,
        snapshot: ExecutionSnapshot,
        demands: &BTreeMap<ResolverSource, RequestId>,
    ) -> impl Future<Output = Result<(), SystemDnsError>> + Send;
    fn shutdown(self, keep: bool) -> impl Future<Output = Result<(), SystemDnsError>> + Send;
}

impl Owner for Runtime {
    fn scopes(&self) -> watch::Sender<Option<meshway_dns::Scope>> {
        self.scopes.clone()
    }
    async fn observe(&mut self) -> Result<(), SystemDnsError> {
        Runtime::observe(self).await
    }
    async fn hold(
        &mut self,
        cfg: &NodeConfiguration,
    ) -> Result<Vec<meshway_dns::ResolverDemand>, SystemDnsError> {
        Runtime::hold(self, cfg).await?;
        self.demands(cfg)
    }
    async fn install(
        &mut self,
        snapshot: ExecutionSnapshot,
        demands: &BTreeMap<ResolverSource, RequestId>,
    ) -> Result<(), SystemDnsError> {
        Runtime::install(self, snapshot, demands).await
    }
    async fn shutdown(self, keep: bool) -> Result<(), SystemDnsError> {
        Runtime::shutdown(self, keep).await
    }
}

enum Command {
    Hold(
        Box<NodeConfiguration>,
        oneshot::Sender<Result<Vec<meshway_dns::ResolverDemand>, SystemDnsError>>,
    ),
    Install(
        Box<ExecutionSnapshot>,
        BTreeMap<ResolverSource, RequestId>,
        oneshot::Sender<Result<(), SystemDnsError>>,
    ),
    Shutdown(bool, oneshot::Sender<Result<(), SystemDnsError>>),
}

struct Actor {
    commands: mpsc::Sender<Command>,
    scopes: watch::Sender<Option<meshway_dns::Scope>>,
    fault: watch::Receiver<Option<SystemDnsError>>,
    task: tokio::task::JoinHandle<()>,
}
impl Actor {
    fn start(mut runtime: impl Owner) -> Self {
        let (commands, mut received) = mpsc::channel(4);
        let (faults, fault) = watch::channel(None);
        let scopes = runtime.scopes();
        let query_scopes = scopes.clone();
        let task = tokio::spawn(async move {
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
                        Some(Command::Hold(cfg, reply)) => {
                            let result = runtime.hold(&cfg).await;
                            if let Err(error) = result {
                                tracing::warn!(
                                    ?error,
                                    operation = "hold",
                                    "DNS owner operation failed"
                                );
                            }
                            let _ = reply.send(result);
                            continue;
                        }
                        Some(Command::Install(snapshot, demands, reply)) => {
                            let result = runtime.install(*snapshot, &demands).await;
                            if let Err(error) = result {
                                tracing::warn!(
                                    ?error,
                                    operation = "install",
                                    "DNS owner operation failed"
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
                let result = tokio::time::timeout(OBSERVE_TIMEOUT, runtime.observe())
                    .await
                    .unwrap_or(Err(SystemDnsError::Backend));
                if let Err(error) = result {
                    if *faults.borrow() != Some(error) {
                        tracing::warn!(
                            ?error,
                            operation = "observe",
                            "DNS owner health check failed"
                        );
                    }
                    query_scopes.send_replace(None);
                    faults.send_replace(Some(error));
                }
            }
        });
        Self {
            commands,
            scopes,
            fault,
            task,
        }
    }
    fn suspend(&self) {
        self.scopes.send_replace(None);
    }
    async fn hold(
        &self,
        cfg: &NodeConfiguration,
    ) -> Result<Vec<meshway_dns::ResolverDemand>, SystemDnsError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Hold(Box::new(cfg.clone()), reply))
            .await
            .map_err(|_| SystemDnsError::Backend)?;
        result.await.map_err(|_| SystemDnsError::Backend)?
    }
    async fn install(
        &self,
        snapshot: ExecutionSnapshot,
        ids: BTreeMap<ResolverSource, RequestId>,
    ) -> Result<(), SystemDnsError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Install(Box::new(snapshot), ids, reply))
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

pub(in crate::runtime::network) struct Attachment {
    settings: SystemDnsSettings,
    directory: Arc<Directory>,
    reservation: Option<SystemDnsReservation>,
    actor: Option<Actor>,
    gateway: mpsc::Sender<super::super::Request>,
    recoveries: u8,
}
impl Attachment {
    pub fn new(
        settings: SystemDnsSettings,
        directory: Arc<Directory>,
        reservation: Option<SystemDnsReservation>,
        gateway: mpsc::Sender<super::super::Request>,
    ) -> Self {
        Self {
            settings,
            directory,
            reservation,
            actor: None,
            gateway,
            recoveries: 0,
        }
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
            Some(
                host.dns_anchor(reserved)
                    .await
                    .map_err(|_| SystemDnsError::Backend)?,
            )
        } else {
            None
        };
        SystemDnsOwner::prepare_reserved(self.reservation.take().expect("reserved above"), anchor)
            .await
    }
    pub async fn plan(
        &mut self,
        host: &mut Host,
        target: &super::super::Target,
    ) -> Result<Plan, SystemDnsError> {
        if self
            .actor
            .as_ref()
            .is_some_and(|actor| actor.fault.borrow().is_some())
        {
            if self.recoveries >= 2 {
                return Err(SystemDnsError::Backend);
            }
            self.recoveries += 1;
            self.actor
                .take()
                .expect("failed actor")
                .shutdown(true)
                .await?;
        }
        if self.actor.is_none() {
            let system = self.owner(host).await?;
            let runtime = Runtime::prepare(
                system,
                self.settings.clone(),
                Arc::new(super::super::discovery::Gateway::new(self.gateway.clone())),
                Arc::new(host.sockets()),
            )
            .await?;
            self.actor = Some(Actor::start(runtime));
        }
        let cfg = target
            .identity
            .as_ref()
            .ok_or(SystemDnsError::ResolverPlan)?
            .snapshot()
            .configuration()
            .map_err(|_| SystemDnsError::ResolverPlan)?;
        let demands = self
            .actor
            .as_ref()
            .expect("prepared above")
            .hold(cfg)
            .await?;
        Ok(Plan::new(target.ticket.instance().clone(), cfg, demands))
    }
    pub async fn install(
        &self,
        snapshot: ExecutionSnapshot,
        ids: BTreeMap<ResolverSource, RequestId>,
    ) -> Result<(), SystemDnsError> {
        self.actor
            .as_ref()
            .ok_or(SystemDnsError::Backend)?
            .install(snapshot, ids)
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
    pub async fn failed(&self) {
        match &self.actor {
            Some(actor) => {
                let mut fault = actor.fault.clone();
                let _ = fault.wait_for(|error| error.is_some()).await;
            }
            None => std::future::pending().await,
        }
    }
    /// Restore DNS before deleting the separately journalled kernel anchor.
    pub async fn down(&mut self, host: &mut Host) -> Result<(), SystemDnsError> {
        self.suspend();
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
                self.recoveries = 0;
                return Ok(());
            }
        }
        host.remove_dns_anchor()
            .await
            .map_err(|_| SystemDnsError::Backend)?;
        self.recoveries = 0;
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
