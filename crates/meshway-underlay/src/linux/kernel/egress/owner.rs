use super::super::{
    monitor::Expected,
    tool::{child::Fence, command::Commands},
};
use super::{
    backend::Backend,
    plan::{self, Ranges},
    program::Program,
    storage::{Device, Gate, Ledger, Store},
};
use crate::{Error, Result, target::View};
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
    /// Replace the MSS clamp table; serialized with the capture's nft work.
    Clamp(Option<super::mss::Clamp>),
}
#[derive(Clone)]
struct State {
    plan: EgressCapturePlan,
    active: bool,
    retired: bool,
}

/// The receipt of the last MSS clamp replacement: after a failed nft
/// transaction the same clamp is due again after a backoff (1 s doubling to
/// 60 s), so an unchanged clamp is not taken as installed.
#[derive(Debug, Default)]
struct ClampRetry {
    failures: u32,
    at: Option<std::time::Instant>,
}

impl ClampRetry {
    fn succeeded(&mut self) {
        *self = Self::default();
    }
    fn failed(&mut self, now: std::time::Instant) -> Duration {
        self.failures = self.failures.saturating_add(1);
        let wait = Duration::from_secs(1)
            .saturating_mul(1 << self.failures.saturating_sub(1).min(6))
            .min(Duration::from_secs(60));
        self.at = Some(now + wait);
        wait
    }
    /// Whether the failed clamp is due again; once true, it is not due again
    /// until its next receipt.
    fn take_due(&mut self, now: std::time::Instant) -> bool {
        if self.at.is_some_and(|at| now >= at) {
            self.at = None;
            return true;
        }
        false
    }
}

/// Owned source-capture resources. No cloned handle retains the client TUN FD.
#[derive(Clone)]
pub struct EgressCapture {
    sender: mpsc::Sender<Request>,
    state: watch::Receiver<State>,
    stop: CancellationToken,
    shared: Arc<super::super::super::protection::Shared>,
    clamp_retry: Arc<Mutex<ClampRetry>>,
}
impl EgressCapture {
    pub(in crate::linux::kernel) async fn audit(&self) -> Result<String> {
        let (reply, result) = oneshot::channel();
        self.submit(Request::Audit(reply))?;
        result.await.map_err(|_| Error::Worker)?
    }
    /// The nft tables this owner holds, audited: the capture table and its
    /// MSS clamp table.
    pub(in crate::linux::kernel) async fn owned_tables(&self) -> Result<Vec<(String, String)>> {
        let table = self.audit().await?;
        let installation = table
            .strip_prefix("meshway_egress_")
            .ok_or(Error::Journal)?;
        Ok(vec![
            ("inet".into(), super::mss::table(installation)),
            ("inet".into(), table),
        ])
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
    /// Replace the MSS clamp, without waiting: its outcome changes no
    /// capture state (see [`super::mss`]).
    pub(in crate::linux::kernel) fn clamp(&self, clamp: Option<super::mss::Clamp>) -> Result<()> {
        self.submit(Request::Clamp(clamp))
    }
    /// Whether the last clamp replacement failed and its retry is due now.
    pub(in crate::linux::kernel) fn clamp_retry_due(&self) -> bool {
        self.clamp_retry
            .lock()
            .is_ok_and(|mut retry| retry.take_due(std::time::Instant::now()))
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

/// The egress owner ends the host's kernel ownership (every owner then
/// fails closed until restart): say which operation failed and how.
fn stopped(
    shared: &super::super::super::protection::Shared,
    operation: &'static str,
    error: &dyn std::fmt::Display,
) {
    tracing::warn!(
        operation,
        error = %error,
        "Egress capture owner stopping after a failed kernel operation; kernel ownership ends"
    );
    shared.conflict.cancel();
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
            .is_some_and(|installed| View::of(&installed.desired).egress_capture() == Some(&plan));
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
            self.clamp = None;
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
        // One selector rule at `start`.
        if rules
            .iter()
            .any(|rule| super::super::priority(rule) == start)
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
            let namespace = super::super::namespace::Identity::current()?;
            Ledger {
                version: 2,
                boot: namespace.boot,
                namespace_device: namespace.device,
                namespace_inode: namespace.inode,
                namespace_cookie: namespace.cookie,
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
        let clamp_retry = Arc::new(Mutex::new(ClampRetry::default()));
        let capture = EgressCapture {
            sender,
            state: status,
            stop: stop.clone(),
            shared: self.shared.clone(),
            clamp_retry: clamp_retry.clone(),
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
                        if let Err(error) = backend.verify().await { stopped(&shared, "verify", &error); break; }
                        // Only withdrawal or a different plan withdraws an
                        // active capture.
                        if state.borrow().active && !keeps(&shared, &backend.ledger.plan) {
                            state.send_modify(|state| state.active = false);
                            if let Err(error) = backend.pause().await { stopped(&shared, "pause", &error); break; }
                        }
                    },
                    request = receiver.recv() => {
                        let Some(request) = request else { break; };
                        if let Request::Clamp(clamp) = &request {
                            let batch = super::mss::batch(&backend.ledger.program.installation, clamp.as_ref());
                            let applied = super::super::exit::objects::apply(&mut backend.commands, &batch).await;
                            if let Ok(mut retry) = clamp_retry.lock() {
                                match applied {
                                    Ok(_) => retry.succeeded(),
                                    Err(_) => {
                                        let wait = retry.failed(std::time::Instant::now());
                                        tracing::warn!(failures = retry.failures, retry_ms = wait.as_millis() as u64,
                                            "TCP MSS clamp table could not be replaced; retrying");
                                    }
                                }
                            }
                            continue;
                        }
                        if !matches!(&request, Request::Audit(_)) { state.send_modify(|state| state.active = false); }
                        let (reply, result, active) = match request {
                            Request::Audit(reply) => {
                                let result = backend.verify().await.map(|()| backend.ledger.program.table());
                                let failed = result.as_ref().err().map(ToString::to_string); let _ = reply.send(result);
                                if let Some(error) = failed { stopped(&shared, "audit", &error); break; }
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
                                    let desired = scope.execution()?;
                                    let source = View::of(&desired).addresses().ipv4;
                                    let device = if plan.managed_ipv4.is_empty() { None } else {
                                        let expected = expected.lock().map_err(|_| Error::Worker)?;
                                        let (index, name, _, _) = expected.tun.as_ref().ok_or(Error::Expired)?;
                                        Some(Device { index: *index, name: name.clone(), source: source.ok_or(Error::Target)? })
                                    };
                                    backend.commit_staged(plan, mesh, device, || scope.check()).await
                                }.await;
                                (reply, result, true)
                            },
                            Request::Pause(reply) => (reply, backend.pause().await, false),
                            Request::Clamp(_) => unreachable!("handled above"),
                            Request::Retire(reply) => {
                                let _ = super::super::exit::objects::apply(&mut backend.commands, &super::mss::batch(&backend.ledger.program.installation, None)).await;
                                let result = backend.retire().await;
                                let ok = result.is_ok();
                                if ok { state.send_modify(|state| { state.active = false; state.retired = true; }); }
                                let failed = result.as_ref().err().map(ToString::to_string);
                                let _ = reply.send(result);
                                if let Some(error) = failed { stopped(&shared, "retire", &error); }
                                stop.cancel();
                                return if ok { Ok(()) } else { Err(Error::Conflict) };
                            },
                        };
                        let ok = result.is_ok();
                        if !ok && let Err(error) = backend.pause().await { stopped(&shared, "pause", &error); break; }
                        state.send_modify(|state| { state.plan = backend.ledger.plan.clone(); state.active = ok && active; });
                        if reply.send(result).is_err() && active { state.send_modify(|state| state.active = false); if let Err(error) = backend.pause().await { stopped(&shared, "pause", &error); break; } }
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
    Ok(Ranges::of(expected.pools.iter().copied()))
}
fn current(
    shared: &Arc<super::super::super::protection::Shared>,
    plan: &EgressCapturePlan,
) -> Result<super::super::super::protection::ProtectionScope> {
    let desired = shared.installed()?.ok_or(Error::Expired)?;
    if View::of(&desired).egress_capture() != Some(plan) {
        return Err(Error::Target);
    }
    shared.data(&desired)
}

/// Content is installed and still selects `plan`. Resource retention only,
/// never I/O authority.
fn keeps(shared: &super::super::super::protection::Shared, plan: &EgressCapturePlan) -> bool {
    shared
        .installed()
        .ok()
        .flatten()
        .is_some_and(|desired| View::of(&desired).egress_capture() == Some(plan))
}

#[cfg(test)]
mod clamp_tests {
    use super::ClampRetry;
    use std::time::{Duration, Instant};

    /// A failed replacement is due again after its backoff, once per receipt;
    /// a success clears it.
    #[test]
    fn a_failed_clamp_is_retried_with_backoff_until_it_applies() {
        let mut retry = ClampRetry::default();
        let now = Instant::now();
        assert!(!retry.take_due(now));
        let waits: Vec<_> = (0..8).map(|_| retry.failed(now).as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 32, 60, 60]);
        assert!(!retry.take_due(now));
        assert!(retry.take_due(now + Duration::from_secs(60)));
        assert!(
            !retry.take_due(now + Duration::from_secs(61)),
            "once per receipt"
        );
        retry.failed(now);
        retry.succeeded();
        assert!(!retry.take_due(now + Duration::from_secs(3600)));
        assert_eq!(retry.failures, 0);
    }
}
