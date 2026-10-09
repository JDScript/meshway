use super::super::{
    exit::{child::Fence, command::Commands},
    monitor::Expected,
};
use super::{
    backend::Backend,
    plan::{self, Ranges},
    program::Program,
    storage::{Device, Gate, Ledger, Store},
};
use crate::{Error, Result};
use meshway_configuration::execution::EgressCapturePlan;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

enum Request {
    Audit(oneshot::Sender<Result<String>>),
    Stage(EgressCapturePlan, oneshot::Sender<Result<()>>),
    Switch(EgressCapturePlan, oneshot::Sender<Result<()>>),
    Pause(oneshot::Sender<Result<()>>),
    Retire(oneshot::Sender<Result<()>>),
}
#[derive(Clone)]
struct State {
    plan: EgressCapturePlan,
    active: bool,
    retired: bool,
}

/// Owned source-capture resources. No cloned handle retains the client TUN FD.
#[derive(Clone)]
pub struct EgressCapture {
    sender: mpsc::Sender<Request>,
    state: watch::Receiver<State>,
    stop: CancellationToken,
    shared: Arc<super::super::super::protection::Shared>,
}
impl EgressCapture {
    pub(in crate::linux::kernel) async fn audit(&self) -> Result<String> {
        let (reply, result) = oneshot::channel();
        self.submit(Request::Audit(reply))?;
        result.await.map_err(|_| Error::Worker)?
    }
    pub async fn switch(&self, new_plan: EgressCapturePlan) -> Result<()> {
        let (reply, result) = oneshot::channel();
        self.submit(Request::Switch(new_plan, reply))?;
        result.await.map_err(|_| Error::Worker)?
    }
    pub async fn retire(&self) -> Result<()> {
        if self.state.borrow().retired {
            return Ok(());
        }
        let (reply, result) = oneshot::channel();
        self.submit(Request::Retire(reply))?;
        result.await.map_err(|_| Error::Worker)?
    }
    pub fn is_active(&self) -> bool {
        let state = self.state.borrow();
        !self.stop.is_cancelled() && state.active && current(&self.shared, &state.plan).is_ok()
    }
    pub fn plan(&self) -> EgressCapturePlan {
        self.state.borrow().plan.clone()
    }
    pub(super) async fn stage(&self, plan: EgressCapturePlan) -> Result<()> {
        let (reply, result) = oneshot::channel();
        self.submit(Request::Stage(plan, reply))?;
        result.await.map_err(|_| Error::Worker)?
    }
    pub(in crate::linux::kernel) async fn pause(&self) -> Result<()> {
        if self.state.borrow().retired {
            return Ok(());
        }
        let (reply, result) = oneshot::channel();
        self.submit(Request::Pause(reply))?;
        result.await.map_err(|_| Error::Worker)?
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
}
pub(in crate::linux::kernel) struct Owner {
    pub capture: EgressCapture,
    task: tokio::task::JoinHandle<Result<()>>,
}
impl Owner {
    pub async fn shutdown(&mut self, remove: bool) -> Result<()> {
        let result = if remove {
            self.capture.retire().await
        } else {
            self.capture.pause().await
        };
        self.capture.stop.cancel();
        let joined = (&mut self.task).await.map_err(|_| Error::Worker)?;
        result.and(joined)
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.capture.stop.cancel();
    }
}

struct Installing(Option<CancellationToken>);
impl Drop for Installing {
    fn drop(&mut self) {
        if let Some(conflict) = self.0.take() {
            conflict.cancel();
        }
    }
}

impl super::super::Kernel {
    pub async fn install_egress_capture(
        &mut self,
        plan: EgressCapturePlan,
    ) -> Result<EgressCapture> {
        self.prepare_egress(plan.clone()).await?;
        let capture = self.egress.as_ref().ok_or(Error::Worker)?.capture.clone();
        let matches = self
            .shared
            .execution
            .read()
            .map_err(|_| Error::Worker)?
            .as_ref()
            .is_some_and(|(snapshot, _)| {
                snapshot
                    .configuration()
                    .is_ok_and(|cfg| cfg.egress_capture == plan)
            });
        if matches {
            capture.switch(plan).await?;
        }
        self.quiesce().await?;
        Ok(capture)
    }
    pub(in crate::linux::kernel) async fn prepare_egress(
        &mut self,
        plan: EgressCapturePlan,
    ) -> Result<()> {
        plan::parse(&plan)?;
        if self
            .egress
            .as_ref()
            .is_some_and(|owner| owner.capture.state.borrow().retired)
            && let Some(mut owner) = self.egress.take()
        {
            owner.shutdown(true).await?;
            self.quiesce().await?;
            self.expected
                .lock()
                .map_err(|_| Error::Worker)?
                .egress_table = None;
        }
        if let Some(owner) = &self.egress {
            if owner.capture.plan() == plan && !owner.capture.state.borrow().retired {
                return Ok(());
            }
            return owner.capture.stage(plan).await;
        }
        self.ready().await?;
        self.verify().await?;
        let start = self.journal.end() + 1;
        let rules = super::super::get_rules(&self.handle, self.options.max_kernel_objects).await?;
        let ranges = plan::parse(&plan)?;
        let needed = u32::try_from(
            (ranges.v4.len() + ranges.v6.len()) * 2 + if ranges.v6.is_empty() { 0 } else { 2 },
        )
        .map_err(|_| Error::Capacity)?;
        if rules
            .iter()
            .any(|rule| (start..start + needed).contains(&super::super::priority(rule)))
        {
            return Err(Error::Conflict);
        }
        let (store, prior) = Store::open(self.store.directory()).await?;
        let ledger = if let Some(prior) = prior.filter(|prior| !prior.removed) {
            prior
        } else {
            let mut random = [0; 16];
            getrandom::fill(&mut random).map_err(|_| Error::Worker)?;
            let installation: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
            let routes =
                super::super::get_routes(&self.handle, self.options.max_kernel_objects).await?;
            let seed = u32::from_be_bytes(random[..4].try_into().map_err(|_| Error::Worker)?);
            let table = (0..1024)
                .map(|offset| 10000 + seed.wrapping_add(offset) % (u32::MAX - 20000))
                .find(|table| {
                    *table != self.journal.table
                        && !routes
                            .iter()
                            .any(|route| super::super::route_table(route) == *table)
                        && !rules
                            .iter()
                            .any(|rule| super::super::rule_table(rule) == *table)
                })
                .ok_or(Error::Conflict)?;
            let (boot, namespace_device, namespace_inode) =
                super::super::journal::Journal::identity()?;
            Ledger {
                version: 1,
                boot,
                namespace_device,
                namespace_inode,
                program: Program {
                    installation,
                    mark: self.journal.mark.mask,
                },
                table,
                plan: plan.clone(),
                gate: Gate {
                    ranges: Ranges::default(),
                    mesh: Ranges::default(),
                    device: None,
                },
                pending_gate: None,
                receipt: None,
                routes: Vec::new(),
                rules: Vec::new(),
                removed: false,
            }
        };
        if ledger.program.mark != self.journal.mark.mask {
            return Err(Error::Conflict);
        }
        let commands = Commands::recorded(Fence {
            directory: store.directory.clone(),
            installation: ledger.program.installation.clone(),
        });
        let mut backend = Backend {
            store,
            ledger,
            commands,
            handle: self.handle.clone(),
            expected: self.expected.clone(),
            limit: self.options.max_kernel_objects,
            start,
            monitored: true,
        };
        let mut installing = Installing(Some(self.shared.conflict.clone()));
        backend.initialize().await?;
        backend.stage(plan.clone(), mesh(&self.expected)?).await?;
        self.quiesce().await?;
        let (sender, mut receiver) = mpsc::channel(16);
        let (state, status) = watch::channel(State {
            plan,
            active: false,
            retired: false,
        });
        let stop = CancellationToken::new();
        let capture = EgressCapture {
            sender,
            state: status,
            stop: stop.clone(),
            shared: self.shared.clone(),
        };
        let shared = self.shared.clone();
        let expected = self.expected.clone();
        let task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! { biased;
                    _ = stop.cancelled() => break,
                    _ = shared.tasks.stop.cancelled() => break,
                    _ = shared.conflict.cancelled() => break,
                    _ = tick.tick() => {
                        if backend.verify().await.is_err() { shared.conflict.cancel(); break; }
                        if state.borrow().active && current(&shared, &backend.ledger.plan).is_err() {
                            state.send_modify(|state| state.active = false);
                            if backend.pause().await.is_err() { shared.conflict.cancel(); break; }
                        }
                    },
                    request = receiver.recv() => {
                        let Some(request) = request else { break; };
                        if !matches!(&request, Request::Audit(_)) { state.send_modify(|state| state.active = false); }
                        let (reply, result, active) = match request {
                            Request::Audit(reply) => {
                                let result = backend.verify().await.map(|()| backend.ledger.program.table());
                                let ok = result.is_ok(); let _ = reply.send(result);
                                if !ok { shared.conflict.cancel(); break; }
                                continue;
                            },
                            Request::Stage(plan, reply) => {
                                let result = match mesh(&expected) { Ok(mesh) => backend.stage(plan, mesh).await, Err(error) => Err(error) };
                                (reply, result, false)
                            },
                            Request::Switch(plan, reply) => {
                                let scope = current(&shared, &plan);
                                let result = async {
                                    let mesh = mesh(&expected)?;
                                    backend.stage(plan.clone(), mesh.clone()).await?;
                                    let scope = scope?;
                                    let cfg = scope.execution()?.configuration().map_err(|_| Error::Expired)?.clone();
                                    let device = if plan.managed_ipv4.is_empty() { None } else {
                                        let expected = expected.lock().map_err(|_| Error::Worker)?;
                                        let (index, name, _, _) = expected.tun.as_ref().ok_or(Error::Expired)?;
                                        Some(Device { index: *index, name: name.clone(), source: cfg.addresses.ipv4.ok_or(Error::Target)? })
                                    };
                                    backend.commit_staged(plan, mesh, device, || scope.check()).await
                                }.await;
                                (reply, result, true)
                            },
                            Request::Pause(reply) => (reply, backend.pause().await, false),
                            Request::Retire(reply) => {
                                let result = backend.retire().await;
                                let ok = result.is_ok();
                                if ok { state.send_modify(|state| { state.active = false; state.retired = true; }); }
                                let _ = reply.send(result);
                                if !ok { shared.conflict.cancel(); }
                                stop.cancel();
                                return if ok { Ok(()) } else { Err(Error::Conflict) };
                            },
                        };
                        let ok = result.is_ok();
                        if !ok && backend.pause().await.is_err() { shared.conflict.cancel(); break; }
                        state.send_modify(|state| { state.plan = backend.ledger.plan.clone(); state.active = ok && active; });
                        if reply.send(result).is_err() && active { state.send_modify(|state| state.active = false); if backend.pause().await.is_err() { shared.conflict.cancel(); break; } }
                    },
                }
            }
            stop.cancel();
            state.send_modify(|state| state.active = false);
            backend.pause().await
        });
        self.egress = Some(Owner { capture, task });
        installing.0 = None;
        Ok(())
    }
}
fn mesh(expected: &Mutex<Expected>) -> Result<Ranges> {
    let expected = expected.lock().map_err(|_| Error::Worker)?;
    Ok(Ranges {
        v4: expected
            .pools
            .iter()
            .copied()
            .filter(|net| net.addr().is_ipv4())
            .collect(),
        v6: expected
            .pools
            .iter()
            .copied()
            .filter(|net| net.addr().is_ipv6())
            .collect(),
    })
}
fn current(
    shared: &Arc<super::super::super::protection::Shared>,
    plan: &EgressCapturePlan,
) -> Result<super::super::super::protection::Scope> {
    let snapshot = shared
        .execution
        .read()
        .map_err(|_| Error::Worker)?
        .as_ref()
        .map(|(snapshot, _)| snapshot.clone())
        .ok_or(Error::Expired)?;
    if snapshot
        .configuration()
        .map_err(|_| Error::Expired)?
        .egress_capture
        != *plan
    {
        return Err(Error::Target);
    }
    shared.data(&snapshot)
}
