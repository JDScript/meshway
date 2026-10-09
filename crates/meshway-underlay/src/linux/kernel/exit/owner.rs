//! Host-owned control actor and its independent bounded TUN I/O task.
use super::{
    CtConfirmation, ExitReplyReader, ExitScope, ProjectionAck,
    backend::Backend,
    command::Commands,
    io::{self, View},
    program::Program,
    routing::Routing,
    storage::{Ledger, Store},
    tuple::OriginalTuple,
};
use crate::{Error, Result};
use meshway_authority::Deadline;
use meshway_configuration::execution::ExitRolePlan;
use meshway_model::exit::ExitFamily;
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

enum Request {
    Install {
        incarnation: u128,
        tuple: OriginalTuple,
        deadline: Deadline,
        scope: ExitScope,
        reply: oneshot::Sender<Result<ProjectionAck>>,
    },
    Renew {
        incarnation: u128,
        deadline: Deadline,
        scope: ExitScope,
        reply: oneshot::Sender<Result<ProjectionAck>>,
    },
    Confirm {
        incarnation: u128,
        reply: oneshot::Sender<Result<CtConfirmation>>,
    },
    Retire {
        incarnation: u128,
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
    view: View,
    shared: Arc<super::super::super::protection::Shared>,
    pub(crate) plan: ExitRolePlan,
}

impl ExitRole {
    pub async fn install(
        &self,
        incarnation: u128,
        original_tuple: OriginalTuple,
        deadline: Deadline,
        scope: ExitScope,
    ) -> Result<ProjectionAck> {
        self.validate_projection(original_tuple, &deadline)?;
        let (reply, result) = oneshot::channel();
        self.submit(Request::Install {
            incarnation,
            tuple: original_tuple,
            deadline,
            scope,
            reply,
        })?;
        result.await.map_err(|_| Error::Worker)?
    }
    pub async fn renew(
        &self,
        incarnation: u128,
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
        self.validate_projection(tuple, &deadline)?;
        let (reply, result) = oneshot::channel();
        self.submit(Request::Renew {
            incarnation,
            deadline,
            scope,
            reply,
        })?;
        result.await.map_err(|_| Error::Worker)?
    }
    pub async fn confirm(&self, incarnation: u128) -> Result<CtConfirmation> {
        let (reply, result) = oneshot::channel();
        self.submit(Request::Confirm { incarnation, reply })?;
        result.await.map_err(|_| Error::Worker)?
    }
    pub async fn retire_flow(&self, incarnation: u128) -> Result<()> {
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
    pub async fn send(&self, incarnation: u128, packet: Vec<u8>) -> Result<()> {
        let live = self
            .view
            .read()
            .map_err(|_| Error::Worker)?
            .get(&incarnation)
            .cloned()
            .ok_or(Error::Expired)?;
        let guard = Authority {
            shared: self.shared.clone(),
            plan: self.plan.clone(),
        }
        .validate(live.tuple, &live.deadline)?;
        self.writer.send(incarnation, packet, guard).await
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
    fn validate_projection(&self, tuple: OriginalTuple, deadline: &Deadline) -> Result<()> {
        Authority {
            shared: self.shared.clone(),
            plan: self.plan.clone(),
        }
        .validate(tuple, deadline)
        .map(|_| ())
    }
}

#[derive(Clone)]
pub(super) struct Authority {
    shared: Arc<super::super::super::protection::Shared>,
    plan: ExitRolePlan,
}
impl Authority {
    pub fn snapshot(&self) -> Result<meshway_authority::ExecutionSnapshot> {
        self.shared.check()?;
        let snapshot = self
            .shared
            .execution
            .read()
            .map_err(|_| Error::Worker)?
            .as_ref()
            .map(|(snapshot, _)| snapshot.clone())
            .ok_or(Error::Expired)?;
        if snapshot
            .configuration()
            .map_err(|_| Error::Expired)?
            .exit_role
            .as_ref()
            != Some(&self.plan)
        {
            return Err(Error::Expired);
        }
        Ok(snapshot)
    }
    pub fn validate(
        &self,
        tuple: OriginalTuple,
        deadline: &Deadline,
    ) -> Result<super::super::super::protection::Scope> {
        use meshway_configuration::execution::{BusinessProtocol, BusinessTarget};
        let snapshot = self.snapshot()?;
        let cfg = snapshot.configuration().map_err(|_| Error::Expired)?;
        if deadline.expires_at() > snapshot.expires_at()
            || deadline.expires_at_ms() > snapshot.expires_at_ms()
        {
            return Err(Error::Expired);
        }
        let (protocol, _, port) = tuple.fields();
        let permitted = cfg.permits.values().any(|permit| {
            let target = match &permit.target {
                BusinessTarget::Ip { address } | BusinessTarget::Domain { address, .. } => {
                    address.0
                }
                _ => return false,
            };
            permit.source_addresses.ipv4 == Some(tuple.source)
                && target == IpAddr::V4(tuple.destination)
                && deadline.expires_at_ms() <= permit.expires_at_ms
                && matches!(
                    (permit.protocol, protocol),
                    (BusinessProtocol::Tcp, 6)
                        | (BusinessProtocol::Udp, 17)
                        | (BusinessProtocol::IcmpEcho, 1)
                )
                && (protocol == 1
                    || permit
                        .port
                        .as_ref()
                        .is_some_and(|value| value.get() == port))
                && permit
                    .exit
                    .as_ref()
                    .and_then(|exit| exit.roles.get(&cfg.node_id))
                    == Some(&self.plan)
        });
        if !permitted {
            return Err(Error::Target);
        }
        self.shared.data(&snapshot)
    }
}

pub(in crate::linux) struct Owner {
    pub role: ExitRole,
    task: tokio::task::JoinHandle<()>,
}
impl Owner {
    pub async fn shutdown(&mut self) -> Result<()> {
        let result = self.role.retire().await;
        (&mut self.task).await.map_err(|_| Error::Worker)?;
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

impl super::super::Kernel {
    pub async fn install_exit_role(&mut self, plan: ExitRolePlan) -> Result<ExitRole> {
        self.ready().await?;
        if self
            .exit
            .as_ref()
            .is_some_and(|owner| !owner.role.is_active())
            && let Some(mut owner) = self.exit.take()
        {
            owner.shutdown().await?;
            self.quiesce().await?;
            let mut expected = self.expected.lock().map_err(|_| Error::Worker)?;
            expected.exit_tun = None;
            expected.exit_tables.clear();
        }
        if let Some(owner) = &self.exit {
            if owner.role.plan == plan && owner.role.is_active() {
                return Ok(owner.role.clone());
            }
            return Err(Error::Conflict);
        }
        let snapshot = self
            .shared
            .execution
            .read()
            .map_err(|_| Error::Worker)?
            .as_ref()
            .map(|(snapshot, _)| snapshot.clone())
            .ok_or(Error::Expired)?;
        let cfg = snapshot.configuration().map_err(|_| Error::Expired)?;
        if cfg.exit_role.as_ref() != Some(&plan)
            || !cfg.exit_enabled
            || plan.family != ExitFamily::Ipv4
            || plan.instance_id != snapshot.instance().instance_id
        {
            return Err(Error::Target);
        }
        if std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")?.trim() != "1" {
            return Err(Error::Target);
        }
        self.verify().await?;
        let network = cfg.network_id;
        let node = cfg.node_id;
        let pools: Vec<_> = cfg
            .mesh_address_pools
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
        super::prerequisites::check(&mut commands).await?;
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
        let (boot, namespace_device, namespace_inode) = super::super::journal::Journal::identity()?;
        let ledger = Ledger {
            version: 1,
            boot,
            namespace_device,
            namespace_inode,
            base_installation: self.journal.installation.clone(),
            network,
            node,
            plan: plan.clone(),
            program: Program {
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
            quarantine: prior.map_or_else(Vec::new, |ledger| ledger.quarantine),
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
        snapshot.ensure_current().map_err(|_| Error::Expired)?;
        let authority = Authority {
            shared: self.shared.clone(),
            plan: plan.clone(),
        };
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
            shared: self.shared.clone(),
            plan,
        };
        let conflict = self.shared.conflict.clone();
        let shutdown = self.shared.tasks.stop.clone();
        let task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(100));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut audit = tokio::time::interval(Duration::from_secs(1));
            audit.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! { biased;
                    _ = stop.cancelled() => break,
                    _ = conflict.cancelled() => break,
                    _ = shutdown.cancelled() => break,
                    _ = audit.tick() => { if backend.verify().await.is_err() { break; } },
                    _ = tick.tick() => {
                        if authority.snapshot().is_err() || backend.maintain().await.is_err() { break; }
                        let invalid: Vec<_> = backend.live.iter().filter(|(_, live)| authority.validate(live.tuple, &live.deadline).is_err()).map(|(id, _)| *id).collect();
                        let mut failed = false;
                        for id in invalid { if backend.retire(id).await.is_err() { failed = true; break; } }
                        if failed { break; }
                    },
                    request = receiver.recv() => {
                        let Some(request) = request else { break; };
                        match request {
                            Request::Install { incarnation, tuple, deadline, scope, reply } => {
                                let guard = authority.validate(tuple, &deadline);
                                let result = match &guard {
                                    Ok(guard) if guard.check().is_ok() => backend.install(incarnation, tuple, deadline, scope).await,
                                    _ => Err(Error::Expired),
                                };
                                let result = result.and_then(|ack| { guard?.check()?; Ok(ack) });
                                if result.is_err() && backend.failed(incarnation).await.is_err() { break; }
                                if reply.send(result).is_err() && backend.failed(incarnation).await.is_err() { break; }
                            },
                            Request::Renew { incarnation, deadline, scope, reply } => {
                                let guard = backend.live.get(&incarnation).ok_or(Error::Expired).and_then(|live| authority.validate(live.tuple, &deadline));
                                let result = match &guard {
                                    Ok(guard) if guard.check().is_ok() => backend.renew(incarnation, deadline, scope).await,
                                    _ => Err(Error::Expired),
                                };
                                let result = result.and_then(|ack| { guard?.check()?; Ok(ack) });
                                if result.is_err() && backend.failed(incarnation).await.is_err() { break; }
                                if reply.send(result).is_err() && backend.failed(incarnation).await.is_err() { break; }
                            },
                            Request::Confirm { incarnation, reply } => {
                                let result = backend.confirm(incarnation).await;
                                if result.is_err() && backend.failed(incarnation).await.is_err() { break; }
                                if reply.send(result).is_err() && backend.failed(incarnation).await.is_err() { break; }
                            },
                            Request::Retire { incarnation, reply } => {
                                let result = backend.retire(incarnation).await;
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
            task,
        });
        installing.0 = None;
        Ok(role)
    }
}
