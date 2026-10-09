//! Opaque, bounded kernel projections of the unique FlowTable. No kernel owner
//! or socket lives here. The host maps these operations to its Underlay owner.
use crate::{Error, Packet, authority::Cap, secure::Guard};
use meshway_authority::ExecutionSnapshot;
use meshway_configuration::execution::BusinessProtocol;
use std::{
    net::Ipv4Addr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_flows: usize,
    /// Includes operations already handed to the host but not acknowledged.
    pub max_pending: usize,
    /// Live flows reserve their eventual tombstone slot before admission.
    pub max_tombstones: usize,
    pub projection_timeout: Duration,
    /// Starts at the successful kernel cleanup ACK (at retirement for origins).
    /// Failed/unknown kernel deletion never releases the reserved slot.
    pub tombstone_timeout: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_flows: 1024,
            max_pending: 128,
            max_tombstones: 4096,
            projection_timeout: Duration::from_secs(5),
            tombstone_timeout: Duration::from_secs(120),
        }
    }
}
impl Limits {
    pub(crate) fn validate(self) -> Result<Self, Error> {
        if self.max_flows == 0
            || self.max_flows > 65_536
            || self.max_pending == 0
            || self.max_pending > 65_536
            || self.max_tombstones < self.max_flows
            || self.max_tombstones > 65_536
            || self.projection_timeout.is_zero()
            || self.projection_timeout > Duration::from_secs(60)
            || self.tombstone_timeout < Duration::from_secs(120)
            || self.tombstone_timeout > Duration::from_secs(86_400)
        {
            return Err(Error::Limits);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct FlowIncarnation(pub(crate) [u8; 32]);
impl FlowIncarnation {
    pub fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Original, pre-NAT IPv4 tuple. Echo has no ports; its identifier and sequence
/// are explicit and cannot alias a TCP/UDP projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExitTuple {
    pub source: Ipv4Addr,
    pub destination: Ipv4Addr,
    pub source_port: Option<u16>,
    pub destination_port: Option<u16>,
    pub protocol: BusinessProtocol,
    pub echo: Option<(u16, u16)>,
}

/// Immutable original guard; the host must recheck at every kernel submission
/// and I/O boundary. Remaining time must never be restarted after queueing.
#[derive(Clone, Debug)]
pub struct ProjectionScope(pub(crate) Guard, Option<Arc<Acknowledgement>>);
#[derive(Debug)]
struct Acknowledgement {
    deadline: Instant,
    deadline_ms: u64,
    status: Arc<AtomicU8>,
}
impl ProjectionScope {
    pub(crate) fn new(guard: Guard) -> Self {
        Self(guard, None)
    }
    pub fn ensure_current(&self) -> Result<(), Error> {
        let cutoff = self
            .1
            .as_ref()
            .filter(|ack| ack.status.load(Ordering::Acquire) == PENDING)
            .map_or(u64::MAX, |ack| ack.deadline_ms);
        self.0.ensure_before(cutoff).map_err(|_| Error::Expired)?;
        if self
            .1
            .as_ref()
            .is_some_and(|ack| match ack.status.load(Ordering::Acquire) {
                READY => false,
                PENDING => Instant::now() >= ack.deadline,
                _ => true,
            })
        {
            return Err(Error::Expired);
        }
        Ok(())
    }
    pub async fn invalidated(&self) {
        if self.ensure_current().is_err() {
            return;
        }
        if let Some(ack) = &self.1
            && ack.status.load(Ordering::Acquire) == PENDING
        {
            tokio::select! { biased;
                _ = self.0.invalidated() => return,
                _ = self.0.snapshot().invalidated_at(ack.deadline_ms) => {},
                _ = tokio::time::sleep_until(ack.deadline.into()) => {},
            }
            if self.ensure_current().is_err() {
                return;
            }
        }
        self.0.invalidated().await;
    }
    pub fn snapshot(&self) -> &ExecutionSnapshot {
        self.0.snapshot()
    }
    /// This operation's own cutoff, or the snapshot's current one.
    pub fn deadline(&self) -> Instant {
        self.0.expiry().at
    }
    pub fn deadline_ms(&self) -> u64 {
        self.0.expiry().ms
    }
}

const PENDING: u8 = 0;
const READY: u8 = 1;
const FAILED: u8 = 2;

pub(crate) struct Gate {
    pub scope: ProjectionScope,
    pub acknowledgement_deadline: Instant,
    pub cancellation: CancellationToken,
    status: Arc<AtomicU8>,
    claimed: AtomicBool,
    budget: Arc<AtomicUsize>,
    released: AtomicBool,
    changed: tokio::sync::Notify,
}
impl Gate {
    pub fn acknowledged(&self) -> bool {
        self.status.load(Ordering::Acquire) == READY
    }
    pub fn new(
        mut scope: ProjectionScope,
        cancellation: CancellationToken,
        budget: Arc<AtomicUsize>,
        limits: Limits,
        now: Instant,
        previous_deadline: Option<Cap>,
    ) -> Result<Arc<Self>, Error> {
        budget
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < limits.max_pending).then_some(n + 1)
            })
            .map_err(|_| Error::Capacity)?;
        let acknowledgement_deadline = scope
            .deadline()
            .min(now + limits.projection_timeout)
            .min(previous_deadline.map_or(scope.deadline(), |deadline| deadline.at));
        let acknowledgement_deadline_ms = previous_deadline
            .map_or(scope.deadline_ms(), |deadline| {
                deadline.ms.min(scope.deadline_ms())
            });
        let status = Arc::new(AtomicU8::new(PENDING));
        scope.1 = Some(Arc::new(Acknowledgement {
            deadline: acknowledgement_deadline,
            deadline_ms: acknowledgement_deadline_ms,
            status: status.clone(),
        }));
        Ok(Arc::new(Self {
            acknowledgement_deadline,
            scope,
            cancellation,
            status,
            claimed: AtomicBool::new(false),
            budget,
            released: AtomicBool::new(false),
            changed: tokio::sync::Notify::new(),
        }))
    }
    fn release_budget(&self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            self.budget.fetch_sub(1, Ordering::AcqRel);
        }
    }
    pub fn ready(&self) -> Result<(), Error> {
        self.scope.ensure_current()?;
        if self.cancellation.is_cancelled() {
            return Err(Error::Expired);
        }
        match self.status.load(Ordering::Acquire) {
            READY => Ok(()),
            PENDING if Instant::now() < self.acknowledgement_deadline => {
                Err(Error::ProjectionPending)
            }
            _ => {
                self.fail();
                Err(Error::Expired)
            }
        }
    }
    pub fn failed(&self, now: Instant, now_ms: u64) -> bool {
        if self.status.load(Ordering::Acquire) == PENDING
            && (now >= self.acknowledgement_deadline
                || self
                    .scope
                    .1
                    .as_ref()
                    .is_some_and(|ack| now_ms >= ack.deadline_ms))
        {
            self.fail();
        }
        self.cancellation.is_cancelled()
    }
    fn fail(&self) {
        self.status.store(FAILED, Ordering::Release);
        self.cancellation.cancel();
        if !self.claimed.load(Ordering::Acquire) {
            self.release_budget();
        }
        self.changed.notify_waiters();
    }
    pub fn claim(self: &Arc<Self>) -> Option<ProjectionAck> {
        (self.status.load(Ordering::Acquire) == PENDING
            && !self.claimed.swap(true, Ordering::AcqRel))
        .then(|| ProjectionAck {
            gate: self.clone(),
            completed: false,
        })
    }
    pub async fn wait(&self) -> Result<(), Error> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            match self.ready() {
                Err(Error::ProjectionPending) => {}
                result => return result,
            }
            tokio::select! { biased;
                _ = self.scope.invalidated() => return Err(Error::Expired),
                _ = self.cancellation.cancelled() => return Err(Error::Expired),
                _ = tokio::time::sleep_until(self.acknowledgement_deadline.into()) => { self.fail(); return Err(Error::Expired); },
                _ = changed => {},
            }
        }
    }
    pub fn retire(&self) {
        self.fail();
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        self.release_budget();
    }
}

/// Move-only completion callback. Dropping it is a failed/unknown installation,
/// never an ACK. Success rechecks the original guard and bounded ACK deadline.
pub struct ProjectionAck {
    gate: Arc<Gate>,
    completed: bool,
}
impl ProjectionAck {
    pub fn complete(mut self, installed: bool) -> Result<(), Error> {
        self.completed = true;
        self.gate.release_budget();
        if !installed
            || self.gate.scope.ensure_current().is_err()
            || Instant::now() >= self.gate.acknowledgement_deadline
            || self.gate.cancellation.is_cancelled()
        {
            self.gate.fail();
            return Err(Error::Expired);
        }
        self.gate
            .status
            .compare_exchange(PENDING, READY, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Expired)?;
        self.gate.release_budget();
        self.gate.changed.notify_waiters();
        Ok(())
    }
}
impl Drop for ProjectionAck {
    fn drop(&mut self) {
        if !self.completed {
            self.gate.fail();
        }
        self.gate.release_budget();
    }
}

/// Cleanup ACK is separate from a live grant: cleanup remains possible after
/// expiry. A failed or dropped ACK permanently quarantines the tuple.
pub struct RetirementAck {
    pub(crate) quarantine_until: Arc<Mutex<Option<Instant>>>,
    pub(crate) quarantine: Duration,
    pub(crate) budget: Arc<AtomicUsize>,
}
impl RetirementAck {
    pub fn complete(self, removed: bool) {
        if removed && let Ok(mut until) = self.quarantine_until.lock() {
            *until = Some(Instant::now() + self.quarantine);
        }
    }
}
impl Drop for RetirementAck {
    fn drop(&mut self) {
        self.budget.fetch_sub(1, Ordering::AcqRel);
    }
}

pub enum ExitProjection {
    Install {
        flow: FlowIncarnation,
        tuple: ExitTuple,
        scope: ProjectionScope,
        ack: ProjectionAck,
    },
    Renew {
        flow: FlowIncarnation,
        tuple: ExitTuple,
        scope: ProjectionScope,
        ack: ProjectionAck,
    },
    Retire {
        flow: FlowIncarnation,
        tuple: ExitTuple,
        /// Original resource binding for cleanup, not a still-live grant.
        scope: ProjectionScope,
        ack: RetirementAck,
    },
}

/// Trusted host adapter for packets read from the owned exit TUN *after* kernel
/// reverse NAT. It carries no authorization, CID, forward facts or reply claim.
pub struct ExitReplyIngress {
    pub(crate) packet: Packet,
}
impl ExitReplyIngress {
    pub fn new(packet: Packet) -> Self {
        Self { packet }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Counters {
    pub ipv6_rejected: u64,
    pub unassociated_reply: u64,
    pub reply_rejected: u64,
}
