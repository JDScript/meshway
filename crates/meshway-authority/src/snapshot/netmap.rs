//! Snapshots of a verified netmap bound by its lease alone.
//!
//! Session and instance deadlines never bound a netmap snapshot. Every
//! snapshot issued for one netmap content shares a lease cell: a same-content
//! renewal through [`NetmapRenewal`] raises the deadline of every live clone
//! without cancelling or replacing any guard. New content is a new issue with
//! a new cell; the issuer invalidates the old guards and consumers compare the
//! two contents with [`ExecutionSnapshot::netmap_continuation_from`].
use super::{ExecutionSnapshot, Target};
use crate::{Clock, Deadline, Error};
use meshway_configuration::netmap::CompiledNetmap;
use meshway_model::{NetmapLease, NetmapRef};
use meshway_policy::admission::netmap::Leased;
use meshway_protocol::InstanceRequest;
use std::{
    sync::{Arc, PoisonError, RwLock, Weak, atomic::AtomicBool},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

/// Everything an issuer binds into the first snapshot of one netmap content.
/// `deadline` must be the lease's `expiresAt` on the issuer's `clock`, e.g.
/// `Deadline::from_response(started, lease.expires_at_ms)`.
pub struct NetmapIssue {
    /// Verified content; its digest is checked again here.
    pub netmap: Arc<CompiledNetmap>,
    /// Lease issued for exactly `netmap`.
    pub lease: NetmapLease,
    pub deadline: Deadline,
    /// State/receipt binding only; never part of validity.
    pub instance: Option<InstanceRequest>,
    pub validity: Arc<AtomicBool>,
    pub invalidated: CancellationToken,
    pub clock: Arc<Clock>,
    /// Only the issuer holds the strong root.
    pub lineage: Weak<()>,
}

#[derive(Clone)]
struct Current {
    lease: NetmapLease,
    deadline: Deadline,
}

struct LeaseCell {
    current: RwLock<Current>,
    changed: tokio::sync::Notify,
}

impl LeaseCell {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Current> {
        self.current.read().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Clone)]
pub(super) struct Bound {
    pub(super) netmap: Arc<CompiledNetmap>,
    cell: Arc<LeaseCell>,
    pub(super) instance: Option<InstanceRequest>,
}

impl Bound {
    pub(super) fn valid_at(&self, now_ms: u64, instant: Instant) -> bool {
        self.cell.read().deadline.valid_at(now_ms, instant)
    }

    pub(super) fn deadline(&self) -> Deadline {
        self.cell.read().deadline.clone()
    }

    pub(super) async fn invalidated_at(&self, snapshot: &ExecutionSnapshot, deadline_ms: u64) {
        loop {
            let changed = self.cell.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let Ok(now_ms) = snapshot.current() else {
                return;
            };
            if now_ms >= deadline_ms {
                return;
            }
            let deadline = self.deadline();
            tokio::select! { biased;
                _ = &mut changed => {},
                _ = snapshot.clock.wait_until(
                    deadline.expires_at(),
                    deadline_ms.min(deadline.expires_at_ms()),
                    &snapshot.invalidated,
                ) => {},
            }
        }
    }
}

/// Values of one netmap guard evaluation, taken under a single lease read.
/// Borrowed from the snapshot for one synchronous section of work; never stored.
#[derive(Clone, Copy, Debug)]
pub struct NetmapReading<'a> {
    pub netmap: &'a CompiledNetmap,
    /// Lease current at this reading.
    pub lease: NetmapLease,
    pub now_ms: u64,
    pub instant: Instant,
    pub expires_at: Instant,
    pub expires_at_ms: u64,
}

impl NetmapReading<'_> {
    /// Input of `meshway_policy::admission::netmap`; pass `now_ms` with it.
    pub fn leased(&self) -> Leased<'_> {
        Leased {
            netmap: &self.netmap.netmap,
            lease: &self.lease,
        }
    }
}

/// The issuer's handle for one netmap content. Not clonable: only the owner of
/// the issue may renew its lease or issue further snapshots sharing it.
pub struct NetmapRenewal {
    template: ExecutionSnapshot,
}

impl NetmapRenewal {
    fn bound(&self) -> &Bound {
        match &self.template.target {
            Target::Netmap(bound) => bound,
            Target::Legacy(_) => unreachable!("renewal is only created for netmap snapshots"),
        }
    }

    pub fn reference(&self) -> NetmapRef {
        self.bound().netmap.reference()
    }

    pub fn netmap(&self) -> &Arc<CompiledNetmap> {
        &self.bound().netmap
    }

    pub fn lease(&self) -> NetmapLease {
        self.bound().cell.read().lease
    }

    /// Replace the lease of the same content. Every live snapshot sharing it
    /// observes the new deadline at its next check; nothing is cancelled. The
    /// lease must be issued no earlier than the current one. Once the current
    /// lease has expired or the guards were invalidated, renewal fails: an
    /// expired grant is never revived, the issuer must issue again.
    pub fn renew(&self, lease: NetmapLease, deadline: Deadline) -> Result<(), Error> {
        let bound = self.bound();
        lease.validate().map_err(|_| Error::Contract)?;
        if lease.netmap != bound.netmap.reference()
            || !deadline.shares_clock(&self.template.clock)
            || deadline.expires_at_ms() != lease.expires_at_ms
        {
            return Err(Error::Contract);
        }
        {
            let mut current = bound
                .cell
                .current
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            self.template.live()?;
            let (now_ms, instant) = self.template.clock.now_with_instant()?;
            if !current.deadline.valid_at(now_ms, instant) || !deadline.valid_at(now_ms, instant) {
                return Err(Error::Expired);
            }
            if lease.issued_at_ms < current.lease.issued_at_ms {
                return Err(Error::Contract);
            }
            *current = Current { lease, deadline };
        }
        bound.cell.changed.notify_waiters();
        Ok(())
    }

    /// Another snapshot of the same content and lease cell, e.g. after a new
    /// instance was acquired. `same_netmap` holds with every earlier snapshot.
    pub fn snapshot(&self, instance: Option<InstanceRequest>) -> Result<ExecutionSnapshot, Error> {
        let mut snapshot = self.template.clone();
        if let Target::Netmap(bound) = &mut snapshot.target {
            bound.instance = instance;
        }
        snapshot.ensure_current()?;
        Ok(snapshot)
    }
}

impl ExecutionSnapshot {
    /// Bind a verified netmap and its lease. Fails when the lease is not for
    /// this exact content, the deadline is not the lease expiry on the issuer
    /// clock, the content digest does not verify, or it has already expired.
    pub fn issue_netmap(issue: NetmapIssue) -> Result<(Self, NetmapRenewal), Error> {
        issue.lease.validate().map_err(|_| Error::Contract)?;
        if !issue.deadline.shares_clock(&issue.clock)
            || issue.deadline.expires_at_ms() != issue.lease.expires_at_ms
            || issue.lease.netmap != issue.netmap.reference()
            || issue.netmap.verify().is_err()
        {
            return Err(Error::Contract);
        }
        let snapshot = Self {
            target: Target::Netmap(Bound {
                netmap: issue.netmap,
                cell: Arc::new(LeaseCell {
                    current: RwLock::new(Current {
                        lease: issue.lease,
                        deadline: issue.deadline,
                    }),
                    changed: tokio::sync::Notify::new(),
                }),
                instance: issue.instance,
            }),
            validity: issue.validity,
            invalidated: issue.invalidated,
            clock: issue.clock,
            lineage: issue.lineage,
        };
        snapshot.ensure_current()?;
        Ok((snapshot.clone(), NetmapRenewal { template: snapshot }))
    }

    fn bound(&self) -> Result<&Bound, Error> {
        match &self.target {
            Target::Netmap(bound) => Ok(bound),
            Target::Legacy(_) => Err(Error::Contract),
        }
    }

    pub fn is_netmap(&self) -> bool {
        matches!(self.target, Target::Netmap(_))
    }

    /// Verified content while the snapshot is current.
    pub fn netmap(&self) -> Result<&CompiledNetmap, Error> {
        let bound = self.bound()?;
        self.ensure_current()?;
        Ok(&bound.netmap)
    }

    /// One complete netmap guard evaluation; see [`NetmapReading`].
    pub fn netmap_reading(&self) -> Result<NetmapReading<'_>, Error> {
        let bound = self.bound()?;
        let current = bound.cell.read();
        self.live()?;
        let (now_ms, instant) = self.clock.now_with_instant()?;
        if !current.deadline.valid_at(now_ms, instant) {
            return Err(Error::Expired);
        }
        Ok(NetmapReading {
            netmap: &bound.netmap,
            lease: current.lease,
            now_ms,
            instant,
            expires_at: current.deadline.expires_at(),
            expires_at_ms: current.deadline.expires_at_ms(),
        })
    }

    /// Exact content identity; comparison data, not authority.
    pub fn netmap_ref(&self) -> Option<NetmapRef> {
        self.bound().ok().map(|bound| bound.netmap.reference())
    }

    /// Lease current now; comparison data, not authority.
    pub fn lease(&self) -> Option<NetmapLease> {
        self.bound().ok().map(|bound| bound.cell.read().lease)
    }

    /// Same content issue and same invalidation epoch: the two snapshots are
    /// interchangeable and only differ in (optional) instance binding. This is
    /// the O(1) "lease renewal only" test; it implies no continuation check.
    pub fn same_netmap(&self, other: &Self) -> bool {
        match (&self.target, &other.target) {
            (Target::Netmap(a), Target::Netmap(b)) => {
                Arc::ptr_eq(&a.cell, &b.cell) && Arc::ptr_eq(&self.validity, &other.validity)
            }
            _ => false,
        }
    }

    /// Validate a newer content issued by the same owner after `old`. Defined
    /// only for a serial increase: same-content renewal is not a continuation
    /// (`same_netmap` holds instead). `old`'s guards are already invalidated
    /// by the issuer, but its lease must still be live: an expired netmap
    /// terminates everything. The own enablement, network, Node, identity,
    /// addresses and keys must be unchanged; otherwise this is a hard barrier.
    /// The returned old content is comparison data for the consumer's own
    /// per-flow/per-circuit decision on the new content, never authority.
    pub fn netmap_continuation_from<'a>(&self, old: &'a Self) -> Result<&'a CompiledNetmap, Error> {
        let (Ok(new), Ok(previous)) = (self.bound(), old.bound()) else {
            return Err(Error::Expired);
        };
        self.ensure_current()?;
        let (now_ms, instant) = old.clock.now_with_instant()?;
        let (a, b) = (&previous.netmap.netmap, &new.netmap.netmap);
        if !Weak::ptr_eq(&self.lineage, &old.lineage)
            || self.lineage.strong_count() == 0
            || !Arc::ptr_eq(&self.clock, &old.clock)
            || b.serial <= a.serial
            || !previous.valid_at(now_ms, instant)
            || !a.self_.enabled
            || !b.self_.enabled
            || a.network_id != b.network_id
            || a.node_id != b.node_id
            || a.self_.identity != b.self_.identity
            || a.self_.addresses != b.self_.addresses
            || a.self_.self_keys != b.self_.self_keys
        {
            return Err(Error::Expired);
        }
        self.ensure_current()?;
        Ok(&previous.netmap)
    }
}

#[cfg(test)]
mod tests;
