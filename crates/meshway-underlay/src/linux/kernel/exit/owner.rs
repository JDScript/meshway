//! Host-owned control actor and its independent bounded TUN I/O task.
use super::io::ExitCutoff as Deadline;
use super::{
    CtConfirmation, ExitReplyReader, ExitScope, ProjectionAck,
    backend::Backend,
    command::Commands,
    io,
    program::Program,
    routing::Routing,
    storage::{Ledger, Store},
    tuple::OriginalTuple,
};
use crate::{
    Error, Result,
    target::{RoleIdentity, View},
};
use meshway_model::exit::ExitFamily;
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

enum Request {
    Audit {
        reply: oneshot::Sender<Result<Vec<(String, String)>>>,
    },
    Install {
        incarnation: super::ExitIncarnation,
        tuple: OriginalTuple,
        deadline: Deadline,
        scope: ExitScope,
        guard: super::super::super::protection::ProtectionScope,
        reply: oneshot::Sender<Result<ProjectionAck>>,
    },
    Renew {
        incarnation: super::ExitIncarnation,
        deadline: Deadline,
        scope: ExitScope,
        guard: super::super::super::protection::ProtectionScope,
        reply: oneshot::Sender<Result<ProjectionAck>>,
    },
    Confirm {
        incarnation: super::ExitIncarnation,
        reply: oneshot::Sender<Result<CtConfirmation>>,
    },
    Retire {
        incarnation: super::ExitIncarnation,
        reply: oneshot::Sender<Result<()>>,
    },
}

/// A handle to the Host's single exit owner. It never owns the TUN descriptor.
#[derive(Clone)]
pub struct ExitRole {
    sender: mpsc::Sender<Request>,
    writer: io::Writer,
    reader: Arc<Mutex<Option<ExitReplyReader>>>,
    stop: CancellationToken,
    done: tokio::sync::watch::Receiver<Option<bool>>,
    view: io::View,
    authority: Authority,
    pub(crate) plan: RoleIdentity,
}

impl ExitRole {
    pub(in crate::linux::kernel) async fn audit(&self) -> Result<Vec<(String, String)>> {
        let (reply, result) = oneshot::channel();
        self.submit(Request::Audit { reply })?;
        result.await.map_err(|_| Error::Worker)?
    }
    pub async fn install(
        &self,
        incarnation: super::ExitIncarnation,
        original_tuple: OriginalTuple,
        deadline: Deadline,
        scope: ExitScope,
    ) -> Result<ProjectionAck> {
        let guard = self.validate_projection(original_tuple, &deadline)?;
        let (reply, result) = oneshot::channel();
        self.submit(Request::Install {
            incarnation,
            tuple: original_tuple,
            deadline,
            scope,
            guard,
            reply,
        })?;
        result.await.map_err(|_| Error::Worker)?
    }
    pub async fn renew(
        &self,
        incarnation: super::ExitIncarnation,
        deadline: Deadline,
        scope: ExitScope,
    ) -> Result<ProjectionAck> {
        let tuple = self
            .view
            .read()
            .map_err(|_| Error::Worker)?
            .get(&incarnation)
            .map(|live| live.tuple)
            .ok_or(Error::Expired)?;
        let guard = self.validate_projection(tuple, &deadline)?;
        let (reply, result) = oneshot::channel();
        self.submit(Request::Renew {
            incarnation,
            deadline,
            scope,
            guard,
            reply,
        })?;
        result.await.map_err(|_| Error::Worker)?
    }
    pub async fn confirm(&self, incarnation: super::ExitIncarnation) -> Result<CtConfirmation> {
        let (reply, result) = oneshot::channel();
        self.submit(Request::Confirm { incarnation, reply })?;
        result.await.map_err(|_| Error::Worker)?
    }
    pub async fn retire_flow(&self, incarnation: super::ExitIncarnation) -> Result<()> {
        if let Some(live) = self
            .view
            .read()
            .map_err(|_| Error::Worker)?
            .get(&incarnation)
        {
            live.retired.cancel();
        }
        let (reply, result) = oneshot::channel();
        self.submit(Request::Retire { incarnation, reply })?;
        result.await.map_err(|_| Error::Worker)?
    }
    pub async fn send(
        &self,
        incarnation: super::ExitIncarnation,
        packet: Vec<u8>,
        packet_guard: impl super::ExitSendGuard,
    ) -> Result<()> {
        let live = self
            .view
            .read()
            .map_err(|_| Error::Worker)?
            .get(&incarnation)
            .cloned()
            .ok_or(Error::Expired)?;
        let guard = self.authority.validate(live.tuple, &live.deadline)?;
        self.writer
            .send(packet, guard, live, Box::new(packet_guard))
            .await
    }
    pub fn reply_reader(&self) -> Result<ExitReplyReader> {
        self.reader
            .lock()
            .map_err(|_| Error::Worker)?
            .take()
            .ok_or(Error::Conflict)
    }
    /// Close positive I/O immediately, then await owned kernel cleanup.
    pub async fn retire(&self) -> Result<()> {
        self.stop.cancel();
        let mut done = self.done.clone();
        loop {
            if let Some(ok) = *done.borrow() {
                return if ok { Ok(()) } else { Err(Error::Conflict) };
            }
            done.changed().await.map_err(|_| Error::Worker)?;
        }
    }
    /// The resource owner is live: its installed content still carries its
    /// role. Each operation and packet still requires its own scope.
    pub fn is_active(&self) -> bool {
        !self.stop.is_cancelled()
    }
    fn submit(&self, request: Request) -> Result<()> {
        if self.stop.is_cancelled() {
            return Err(Error::Expired);
        }
        self.sender.try_send(request).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => Error::Capacity,
            mpsc::error::TrySendError::Closed(_) => Error::Expired,
        })
    }
    fn validate_projection(
        &self,
        tuple: OriginalTuple,
        deadline: &Deadline,
    ) -> Result<super::super::super::protection::ProtectionScope> {
        self.authority.validate(tuple, deadline)
    }
}

#[derive(Clone)]
pub(super) struct Authority {
    shared: Arc<super::super::super::protection::Shared>,
    plan: RoleIdentity,
}
impl Authority {
    fn new(shared: Arc<super::super::super::protection::Shared>, plan: RoleIdentity) -> Self {
        Self { shared, plan }
    }

    /// Resource lifetime: the owner keeps its projections and CT while the
    /// installed content carries this role. Withdrawal (empty apply) or a
    /// changed role stops it; every projection then retires with its CT.
    fn keeps_role(&self) -> bool {
        self.shared.check().is_ok()
            && self
                .shared
                .installed()
                .ok()
                .flatten()
                .is_some_and(|desired| {
                    let view = View::of(&desired);
                    view.exit_enabled() && view.exit_role().as_ref() == Some(&self.plan)
                })
    }

    /// The installed content, while it carries this role.
    pub fn installed(&self) -> Result<meshway_engine::Desired> {
        self.shared.check()?;
        let desired = self.shared.installed()?.ok_or(Error::Expired)?;
        if View::of(&desired).exit_role().as_ref() != Some(&self.plan) {
            return Err(Error::Expired);
        }
        Ok(desired)
    }
    pub fn validate(
        &self,
        _tuple: OriginalTuple,
        _deadline: &Deadline,
    ) -> Result<super::super::super::protection::ProtectionScope> {
        // Packet/tuple Access admission belongs to SecureNode's unique flow
        // table, which also owns each projection's cutoff. The kernel owner
        // enforces its installed role and exact projection transaction; it
        // never builds a second policy compiler.
        self.shared.data(&self.installed()?)
    }
}

pub(in crate::linux) struct Owner {
    pub role: ExitRole,
    pub(in crate::linux::kernel) interface_name: String,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Owner {
    pub async fn shutdown(&mut self) -> Result<()> {
        let result = self.role.retire().await;
        // Cancellation retains the join handle; completed joins are not polled twice.
        if let Some(task) = self.task.as_mut() {
            let joined = task.await.map_err(|_| Error::Worker);
            self.task = None;
            joined?;
        }
        result
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.role.stop.cancel();
    }
}

/// Covers cancellation of the Host installation future after its first owned
/// resource is allocated. Restart reconciles the durable intent before reuse.
struct Installing(Option<CancellationToken>);
impl Drop for Installing {
    fn drop(&mut self) {
        if let Some(conflict) = self.0.take() {
            conflict.cancel();
        }
    }
}

/// A failed projection reconciliation stops the whole owner; say why.
fn stopped(operation: &'static str, error: &Error) {
    tracing::warn!(operation, %error, "Exit owner stopping after a failed kernel operation");
}

async fn reconcile(backend: &mut Backend, incarnation: super::ExitIncarnation) -> Result<()> {
    let result = backend.failed(incarnation).await;
    if let Err(error) = &result {
        stopped("reconcile", error);
    }
    result
}

impl super::super::Kernel {
    /// Keep uncertain cleanup attached to its owner. A stopped request queue is
    /// not an ownership conflict, and a failed cleanup is not an absence proof.
    pub(super) async fn reconcile_stopped_exit(&mut self) -> Result<()> {
        if let Some(owner) = self.exit.as_mut()
            && !owner.role.is_active()
        {
            owner.shutdown().await?;
            self.quiesce().await?;
            let mut expected = self.expected.lock().map_err(|_| Error::Worker)?;
            expected.exit_tun = None;
            expected.exit_tables.clear();
            self.exit.take();
        }
        Ok(())
    }

    pub async fn install_exit_role(&mut self, plan: RoleIdentity) -> Result<ExitRole> {
        self.ready().await?;
        self.reconcile_stopped_exit().await?;
        if let Some(owner) = &self.exit {
            if owner.role.plan == plan && owner.role.is_active() {
                return Ok(owner.role.clone());
            }
            return Err(Error::Conflict);
        }
        let desired = self.shared.installed()?.ok_or(Error::Expired)?;
        let cfg = View::of(&desired);
        if cfg.exit_role().as_ref() != Some(&plan)
            || !cfg.exit_enabled()
            || plan.family != ExitFamily::Ipv4
        {
            return Err(Error::Target);
        }
        if std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")?.trim() != "1" {
            return Err(Error::Target);
        }
        self.verify().await?;
        let network = cfg.network_id();
        let node = cfg.node_id();
        let pools: Vec<_> = cfg
            .mesh_address_pools()
            .ipv4
            .iter()
            .map(|pool| pool.parse::<ipnet::Ipv4Net>().map_err(|_| Error::Target))
            .collect::<Result<_>>()?;
        if pools.is_empty() || self.journal.start <= super::routing::RESERVED {
            return Err(Error::Target);
        }
        let rules = super::super::get_rules(&self.handle, self.options.max_kernel_objects).await?;
        let routes =
            super::super::get_routes(&self.handle, self.options.max_kernel_objects).await?;
        let mask = self
            .shared
            .selectors
            .iter()
            .fold(self.journal.mark.mask, |mask, selector| {
                mask | selector.mask
            });
        let (store, prior) = Store::open(self.store.directory()).await?;
        if prior.as_ref().is_some_and(|ledger| !ledger.removed) {
            return Err(Error::Conflict);
        }
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|_| Error::Worker)?;
        let installation: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut commands = Commands::default();
        let mut owned = Vec::new();
        if let Some(egress) = &self.egress {
            owned.extend(egress.capture.owned_tables().await?);
        }
        super::prerequisites::check(&mut commands, &owned).await?;
        let mut selected = None;
        for shift in 0..32 {
            let bit = 1u32 << ((shift + u32::from(random[0])) % 32);
            if mask & bit == 0
                && super::conntrack::mark_unused(&mut commands, bit)
                    .await
                    .map_err(|_| Error::Conflict)?
            {
                selected = Some(bit);
                break;
            }
        }
        let mark = selected.ok_or(Error::Conflict)?;
        let seed = u32::from_be_bytes(random[..4].try_into().map_err(|_| Error::Worker)?);
        let table = (0..1024)
            .map(|offset| 10000 + seed.wrapping_add(offset) % (u32::MAX - 20000))
            .find(|table| {
                *table != self.journal.table
                    && !routes
                        .iter()
                        .any(|route| super::super::plan::route_table(route) == *table)
                    && !rules
                        .iter()
                        .any(|rule| super::super::plan::rule_table(rule) == *table)
            })
            .ok_or(Error::Conflict)?;
        let local: Vec<Ipv4Addr> =
            super::super::get_addresses(&self.handle, self.options.max_kernel_objects)
                .await?
                .into_iter()
                .flat_map(|address| address.attributes)
                .filter_map(|attribute| match attribute {
                    rtnetlink::packet_route::address::AddressAttribute::Address(IpAddr::V4(ip))
                    | rtnetlink::packet_route::address::AddressAttribute::Local(IpAddr::V4(ip)) => {
                        Some(ip)
                    }
                    _ => None,
                })
                .collect();
        let mut installing = Installing(Some(self.shared.conflict.clone()));
        let tun = {
            let mut expected = self.expected.lock().map_err(|_| Error::Worker)?;
            let tun = meshway_tun::Tun::create(meshway_tun::Options::new(self.options.mtu)?)?;
            expected.exit_tun = Some(tun.if_index()?);
            expected.exit_tables.push(table);
            tun
        };
        self.quiesce().await?;
        let namespace = super::super::namespace::Identity::current()?;
        let ledger = Ledger {
            version: 2,
            boot: namespace.boot,
            namespace_device: namespace.device,
            namespace_inode: namespace.inode,
            namespace_cookie: namespace.cookie,
            base_installation: self.journal.installation.clone(),
            network,
            node,
            plan: plan.clone(),
            program: Program {
                legacy: false,
                installation,
                interface: tun.if_index()?,
                interface_name: tun.name().to_owned(),
                mark,
                pools,
            },
            tun_name: tun.name().to_owned(),
            mtu: tun.mtu(),
            route_table: table,
            rules: Vec::new(),
            routes: Vec::new(),
            receipt: None,
            projections: Vec::new(),
            quarantine: prior
                .as_ref()
                .map_or_else(Vec::new, |ledger| ledger.quarantine.clone()),
            legacy_quarantine: prior.map_or_else(Vec::new, |ledger| ledger.legacy_quarantine),
            closing: false,
            removed: false,
            pending: true,
        };
        let view = Arc::new(RwLock::new(BTreeMap::new()));
        let stop = CancellationToken::new();
        let routing = Routing {
            handle: self.handle.clone(),
            expected: self.expected.clone(),
            limit: self.options.max_kernel_objects,
            start: self.journal.start,
            end: self.journal.end(),
        };
        let mut backend =
            Backend::create(store, ledger, routing, view.clone(), local, stop.clone()).await?;
        self.quiesce().await?;
        let authority = Authority::new(self.shared.clone(), plan.clone());
        let (mut io, writer, reader) =
            io::Owner::start(tun, view.clone(), stop.clone(), authority.clone());
        let (sender, mut receiver) = mpsc::channel(64);
        let (done, status) = tokio::sync::watch::channel(None);
        let role = ExitRole {
            sender,
            writer,
            reader: Arc::new(Mutex::new(Some(reader))),
            stop: stop.clone(),
            done: status,
            view,
            authority: authority.clone(),
            plan,
        };
        let conflict = self.shared.conflict.clone();
        let interface_name = backend.ledger.tun_name.clone();
        let shutdown = self.shared.tasks.stop.clone();
        let task = tokio::spawn(async move {
            // Confirmations wait for a kernel fact; they never hold the owner.
            let mut confirming: Vec<(
                super::ExitIncarnation,
                Instant,
                oneshot::Sender<Result<CtConfirmation>>,
            )> = Vec::new();
            let mut next_poll = Instant::now();
            let mut tick = tokio::time::interval(Duration::from_millis(100));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut audit = tokio::time::interval(Duration::from_secs(1));
            audit.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! { biased;
                    _ = stop.cancelled() => break,
                    _ = conflict.cancelled() => break,
                    _ = shutdown.cancelled() => break,
                    _ = audit.tick() => { if let Err(error) = backend.verify().await { stopped("audit", &error); break; } },
                    _ = tokio::time::sleep_until(next_poll.into()), if !confirming.is_empty() => {
                        let mut failed = false;
                        let mut waiting = Vec::with_capacity(confirming.len());
                        for (incarnation, until, reply) in std::mem::take(&mut confirming) {
                            let result = match backend.poll_confirm(incarnation, until).await {
                                Ok(None) => {
                                    waiting.push((incarnation, until, reply));
                                    continue;
                                }
                                Ok(Some(confirmation)) => Ok(confirmation),
                                Err(error) => {
                                    tracing::debug!(operation = "confirm", %error, "Exit projection operation failed");
                                    Err(error)
                                }
                            };
                            let error = result.is_err();
                            let sent = reply.send(result).is_ok();
                            if (error || !sent) && reconcile(&mut backend, incarnation).await.is_err() {
                                failed = true;
                                break;
                            }
                        }
                        confirming.extend(waiting);
                        next_poll = Instant::now() + super::backend::CONFIRM_POLL;
                        if failed { break; }
                    },
                    _ = tick.tick() => {
                        // Withdrawal, Host suspension or a withdrawn role stops the
                        // owner: every projection retires with its exact CT.
                        if !authority.keeps_role() { tracing::debug!(operation = "role", "Exit owner stopping: role no longer installed"); break; }
                        // Projection cutoffs and explicit revocations.
                        if let Err(error) = backend.maintain().await { stopped("maintain", &error); break; }
                    },
                    request = receiver.recv() => {
                        let Some(request) = request else { break; };
                        match request {
                            Request::Audit { reply } => {
                                let result = backend.verify().await.map(|()| backend.ledger.program.tables().into_iter().map(|(family, name)| (family.to_owned(), name)).collect());
                                if let Err(error) = &result { stopped("audit", error); } let ok = result.is_ok(); let _ = reply.send(result); if !ok { break; }
                            },
                            Request::Install { incarnation, tuple, deadline, scope, guard, reply } => {
                                // Rejected queued work never entered the kernel
                                // transaction and must not retire a kept flow.
                                if guard.check().is_err() {
                                    let _ = reply.send(Err(Error::Expired));
                                    continue;
                                }
                                let result = backend.install(incarnation, tuple, deadline, scope).await;
                                if let Err(error) = &result { tracing::debug!(operation = "install", %error, "Exit projection operation failed"); }
                                let result = result.and_then(|ack| { guard.check()?; Ok(ack) });
                                if result.is_err() && reconcile(&mut backend, incarnation).await.is_err() { break; }
                                if reply.send(result).is_err() && reconcile(&mut backend, incarnation).await.is_err() { break; }
                            },
                            Request::Renew { incarnation, deadline, scope, guard, reply } => {
                                if guard.check().is_err() {
                                    let _ = reply.send(Err(Error::Expired));
                                    continue;
                                }
                                let result = backend.renew(incarnation, deadline, scope).await;
                                if let Err(error) = &result { tracing::debug!(operation = "renew", %error, "Exit projection operation failed"); }
                                let result = result.and_then(|ack| { guard.check()?; Ok(ack) });
                                if result.is_err() && reconcile(&mut backend, incarnation).await.is_err() { break; }
                                if reply.send(result).is_err() && reconcile(&mut backend, incarnation).await.is_err() { break; }
                            },
                            Request::Confirm { incarnation, reply } => match backend.confirmation_cutoff(incarnation) {
                                Ok(until) => {
                                    if confirming.is_empty() {
                                        next_poll = Instant::now() + super::backend::CONFIRM_POLL;
                                    }
                                    confirming.push((incarnation, until, reply));
                                }
                                Err(error) => {
                                    tracing::debug!(operation = "confirm", %error, "Exit projection operation failed");
                                    let _ = reply.send(Err(error));
                                    if reconcile(&mut backend, incarnation).await.is_err() { break; }
                                }
                            },
                            Request::Retire { incarnation, reply } => {
                                let result = backend.retire(incarnation).await;
                                if let Err(error) = &result { stopped("retire", error); }
                                let failed = result.is_err();
                                let _ = reply.send(result);
                                if failed { break; }
                            },
                        }
                    },
                }
            }
            stop.cancel();
            let cleanup = backend.begin_close(true).await;
            let io_result = io.shutdown().await;
            let finished = if cleanup.is_ok() && io_result.is_ok() {
                backend.finish_close().await
            } else {
                Err(Error::Conflict)
            };
            done.send_replace(Some(finished.is_ok()));
        });
        self.exit = Some(Owner {
            role: role.clone(),
            interface_name,
            task: Some(task),
        });
        installing.0 = None;
        Ok(role)
    }
}
