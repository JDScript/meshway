//! Host-owned control actor and its independent bounded TUN I/O task.
use super::super::verification::{
    Boxed, Cadence, Owner as Verified, Unverified, Verification, retried,
};
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
    net::IpAddr,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

mod install;

enum Request {
    Audit {
        reply: oneshot::Sender<Result<Vec<(String, String)>>>,
    },
    /// The projections of one driver round, answered one result each.
    Install {
        items: Vec<(
            super::backend::Install,
            super::super::super::protection::ProtectionScope,
        )>,
        reply: oneshot::Sender<Vec<Result<ProjectionAck>>>,
    },
    Renew(Box<Renew>),
    Confirm {
        incarnation: super::ExitIncarnation,
        reply: oneshot::Sender<Result<CtConfirmation>>,
    },
    Retire {
        incarnation: super::ExitIncarnation,
        reply: oneshot::Sender<Result<()>>,
    },
}

struct Renew {
    incarnation: super::ExitIncarnation,
    deadline: Deadline,
    scope: ExitScope,
    guard: super::super::super::protection::ProtectionScope,
    reply: oneshot::Sender<Result<ProjectionAck>>,
}

pub use crate::RetryPending;

/// First wait after a failed cleanup, and the longest.
const CLEANUP_RETRY: Duration = Duration::from_secs(1);
const CLEANUP_RETRY_MAX: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
enum Closing {
    Serving,
    Pending(RetryPending),
    /// Cleanup finished (`true`) or was given up with the Host (`false`:
    /// the journal keeps the resources for the next start's recovery).
    Done(bool),
}

/// A handle to the Host's single exit owner. It never owns the TUN descriptor.
#[derive(Clone)]
pub struct ExitRole {
    sender: mpsc::Sender<Request>,
    writer: io::Writer,
    reader: Arc<Mutex<Option<ExitReplyReader>>>,
    stop: CancellationToken,
    done: tokio::sync::watch::Receiver<Closing>,
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
        self.install_batch(vec![(incarnation, original_tuple, deadline, scope)])
            .await
            .pop()
            .unwrap_or(Err(Error::Worker))
    }
    /// Install several projections in one kernel round: one result per
    /// projection, in order. Each result is that projection's own: it is
    /// acknowledged only once its own element is in the kernel, and another
    /// projection's failure does not change it.
    pub async fn install_batch(
        &self,
        projections: Vec<(super::ExitIncarnation, OriginalTuple, Deadline, ExitScope)>,
    ) -> Vec<Result<ProjectionAck>> {
        let mut results: Vec<Option<Result<ProjectionAck>>> = Vec::new();
        let mut items = Vec::new();
        for (incarnation, tuple, deadline, scope) in projections {
            match self.validate_projection(tuple, &deadline) {
                Ok(guard) => {
                    results.push(None);
                    items.push((
                        super::backend::Install {
                            incarnation,
                            tuple,
                            deadline,
                            scope,
                        },
                        guard,
                    ));
                }
                Err(error) => results.push(Some(Err(error))),
            }
        }
        let mut answered = if items.is_empty() {
            Vec::new()
        } else {
            let count = items.len();
            let (reply, result) = oneshot::channel();
            match self.submit(Request::Install { items, reply }) {
                Ok(()) => result
                    .await
                    .unwrap_or_else(|_| (0..count).map(|_| Err(Error::Worker)).collect()),
                Err(error) => (0..count)
                    .map(|_| Err(super::backend::replicate(&error)))
                    .collect(),
            }
        }
        .into_iter();
        results
            .into_iter()
            .map(|result| result.unwrap_or_else(|| answered.next().unwrap_or(Err(Error::Worker))))
            .collect()
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
        self.submit(Request::Renew(Box::new(Renew {
            incarnation,
            deadline,
            scope,
            guard,
            reply,
        })))?;
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
    /// The writer of `incarnation` while its projection is live here: its
    /// packets go to the I/O owner's batches without a reply. `None` once
    /// the projection was retired or revoked, or never existed; the caller's
    /// awaited [`Self::send`] then reports what that means for its flow.
    pub fn writer(&self, incarnation: super::ExitIncarnation) -> Option<super::ExitWriter> {
        // Its cutoffs are checked at the write, with one clock reading.
        let live = self
            .view
            .read()
            .ok()?
            .get(&incarnation)
            .filter(|live| !live.ended())
            .cloned()?;
        Some(super::ExitWriter {
            live,
            writer: self.writer.clone(),
        })
    }
    /// Queue one batch of writes (at most 64) for the I/O owner without
    /// waiting for them; `Capacity`: its queue is full and none was queued.
    pub fn submit_writes(&self, writes: Vec<super::ExitWrite>) -> Result<()> {
        self.writer.submit(writes)
    }
    pub fn reply_reader(&self) -> Result<ExitReplyReader> {
        self.reader
            .lock()
            .map_err(|_| Error::Worker)?
            .take()
            .ok_or(Error::Conflict)
    }
    /// Close positive I/O immediately, then await the first cleanup attempt.
    /// A failed one is `CleanupPending` at once (unknown, not a conflict:
    /// the owner keeps retrying it, see [`Self::cleanup_pending`]).
    pub async fn retire(&self) -> Result<()> {
        self.stop.cancel();
        let mut done = self.done.clone();
        loop {
            match &*done.borrow() {
                Closing::Serving => {}
                Closing::Done(true) => return Ok(()),
                Closing::Done(false) | Closing::Pending(_) => return Err(Error::CleanupPending),
            }
            done.changed().await.map_err(|_| Error::Worker)?;
        }
    }
    /// The stopped role's cleanup is being retried.
    pub fn cleanup_pending(&self) -> Option<RetryPending> {
        match &*self.done.borrow() {
            Closing::Pending(pending) => Some(pending.clone()),
            _ => None,
        }
    }
    /// The resource owner is live: its installed content still carries its
    /// role. Each operation and packet still requires its own scope.
    pub fn is_active(&self) -> bool {
        !self.stop.is_cancelled()
    }
    /// Completes once the owner stops serving: retirement, withdrawal of
    /// its role, or a failure of its own.
    pub async fn stopped(&self) {
        self.stop.cancelled().await;
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
        // A retried cleanup keeps its task; it is joined once it finished.
        if self.role.cleanup_pending().is_some() {
            return result;
        }
        // Cancellation retains the join handle; completed joins are not polled twice.
        if let Some(task) = self.task.as_mut() {
            let joined = task.await.map_err(|_| Error::Worker);
            self.task = None;
            joined?;
        }
        result
    }
}
impl Owner {
    /// The Host's ownership term ended: stop the role and join its owner
    /// without retrying its cleanup here; the next term's recovery cleans up
    /// from the journal.
    pub(in crate::linux::kernel) async fn abandon(&mut self) {
        self.role.stop.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.role.stop.cancel();
    }
}
