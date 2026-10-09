//! Immutable execution handoff. The issuer supplies every deadline, the shared
//! invalidation and the private target; nothing here reads saved files or mints
//! a grant from caller-asserted configuration/expiry.
//!
//! Two target kinds exist while the Node migrates: the legacy per-publication
//! `CompiledNode` bound by session/instance/execution deadlines, and a verified
//! netmap bound by its `NetmapLease` alone (see [`NetmapIssue`]). Legacy
//! accessors fail or panic on netmap snapshots, as documented on each.
use crate::{Clock, Deadline, Error, reference};
use meshway_configuration::execution::{CompiledNode, NodeConfiguration, TrustedNode};
use meshway_model::{ConfigurationRef, InstanceId, NetworkId, NodeId, Version};
use std::{
    fmt,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tokio_util::sync::CancellationToken;

mod netmap;
pub use netmap::{NetmapIssue, NetmapReading, NetmapRenewal};

/// Everything an issuer binds into one snapshot. `validity`, `invalidated`,
/// `clock` and `lineage` are shared with the issuer so that later revocation
/// reaches every clone; `publication`/`predecessor` identify legal successors.
pub struct Issue {
    pub target: Arc<CompiledNode>,
    pub instance: InstanceId,
    pub generation: Version,
    pub validity: Arc<AtomicBool>,
    pub invalidated: CancellationToken,
    pub clock: Arc<Clock>,
    /// Session, instance and execution deadlines; the earliest one bounds work.
    pub deadlines: [Deadline; 3],
    /// Only the issuer holds the strong root. A snapshot must not keep inactive
    /// descriptors retainable after stop/re-enrollment/fencing.
    pub lineage: Weak<()>,
    pub publication: Arc<()>,
    pub predecessor: Option<(Arc<()>, ConfigurationRef)>,
}

/// The result of one guard evaluation: the configuration it authorizes and the
/// clock values it was judged at. Borrowed from the snapshot, never stored.
#[derive(Clone, Copy, Debug)]
pub struct Reading<'a> {
    pub configuration: &'a NodeConfiguration,
    pub now_ms: u64,
    pub instant: Instant,
    /// Earliest monotonic/UTC expiry across the snapshot's deadlines.
    pub expires_at: Instant,
    pub expires_at_ms: u64,
}

#[derive(Clone)]
struct Legacy {
    target: Arc<CompiledNode>,
    instance: InstanceId,
    generation: Version,
    deadlines: [Deadline; 3],
    publication: Arc<()>,
    predecessor: Option<(Arc<()>, ConfigurationRef)>,
}

#[derive(Clone)]
enum Target {
    Legacy(Legacy),
    Netmap(netmap::Bound),
}

/// A private target and its original control deadlines. Cloning or reading the
/// snapshot never extends them. Hosts must check immediately before admitting
/// work or releasing an output, and select `invalidated` around blocked I/O.
/// This is installation input, not proof that any component is applied.
#[derive(Clone)]
pub struct ExecutionSnapshot {
    target: Target,
    validity: Arc<AtomicBool>,
    invalidated: CancellationToken,
    clock: Arc<Clock>,
    lineage: Weak<()>,
}

impl ExecutionSnapshot {
    /// Bind an issuer's current facts. Fails when any deadline has already
    /// passed or validity was withdrawn; an expired snapshot is never returned.
    pub fn issue(issue: Issue) -> Result<Self, Error> {
        // Every check reads the issuer's clock once and applies it to all three
        // deadlines; a deadline started on another clock would be misjudged.
        if !issue
            .deadlines
            .iter()
            .all(|deadline| deadline.shares_clock(&issue.clock))
        {
            return Err(Error::Contract);
        }
        let snapshot = Self {
            target: Target::Legacy(Legacy {
                target: issue.target,
                instance: issue.instance,
                generation: issue.generation,
                deadlines: issue.deadlines,
                publication: issue.publication,
                predecessor: issue.predecessor,
            }),
            validity: issue.validity,
            invalidated: issue.invalidated,
            clock: issue.clock,
            lineage: issue.lineage,
        };
        snapshot.ensure_current()?;
        Ok(snapshot)
    }

    fn legacy(&self) -> Result<&Legacy, Error> {
        match &self.target {
            Target::Legacy(legacy) => Ok(legacy),
            Target::Netmap(_) => Err(Error::Contract),
        }
    }

    fn legacy_or_panic(&self) -> &Legacy {
        match &self.target {
            Target::Legacy(legacy) => legacy,
            Target::Netmap(_) => {
                panic!("legacy ExecutionSnapshot accessor called on a netmap snapshot")
            }
        }
    }

    /// Legacy: the instance bound into a publication snapshot.
    ///
    /// # Panics
    /// On a netmap snapshot, whose instance is optional State binding only; use
    /// [`Self::instance_binding`].
    pub fn instance(&self) -> meshway_protocol::InstanceRequest {
        let legacy = self.legacy_or_panic();
        meshway_protocol::InstanceRequest {
            instance_id: legacy.instance,
            generation: legacy.generation,
        }
    }

    /// Instance qualification for State, receipts and coordination requests.
    /// It never contributes to validity: a netmap snapshot without an instance
    /// (offline restart) or with an expired one remains current until its lease.
    pub fn instance_binding(&self) -> Option<meshway_protocol::InstanceRequest> {
        match &self.target {
            Target::Legacy(_) => Some(self.instance()),
            Target::Netmap(bound) => bound.instance.clone(),
        }
    }

    pub fn ensure_current(&self) -> Result<(), Error> {
        self.current().map(|_| ())
    }

    /// The full guard with one clock reading, returning the UTC high-water
    /// milliseconds it was evaluated at.
    fn current(&self) -> Result<u64, Error> {
        self.guard().map(|(now_ms, _)| now_ms)
    }

    fn live(&self) -> Result<(), Error> {
        if !self.validity.load(Ordering::Acquire) || self.invalidated.is_cancelled() {
            return Err(Error::Expired);
        }
        Ok(())
    }

    fn guard(&self) -> Result<(u64, Instant), Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::detail("control.guard", 1);
        let _profile = PROFILE.enter();
        self.live()?;
        let (now_ms, instant) = self.clock.now_with_instant()?;
        if !self.deadlines_valid_at(now_ms, instant) {
            return Err(Error::Expired);
        }
        Ok((now_ms, instant))
    }

    /// Legacy: one complete guard evaluation for one synchronous section of
    /// work. The caller applies the same configuration and time to every
    /// decision in that section and still re-checks the snapshot before
    /// committing state or releasing an output after any await. A reading is
    /// not transferable authority and must not outlive the section it was
    /// taken for. Netmap snapshots use [`Self::netmap_reading`].
    pub fn reading(&self) -> Result<Reading<'_>, Error> {
        let legacy = self.legacy()?;
        let (now_ms, instant) = self.guard()?;
        Ok(Reading {
            configuration: &legacy.target.configuration,
            now_ms,
            instant,
            expires_at: self.expires_at(),
            expires_at_ms: self.expires_at_ms(),
        })
    }

    fn deadlines_valid_at(&self, now_ms: u64, instant: Instant) -> bool {
        match &self.target {
            Target::Legacy(legacy) => legacy
                .deadlines
                .iter()
                .all(|deadline| deadline.valid_at(now_ms, instant)),
            Target::Netmap(bound) => bound.valid_at(now_ms, instant),
        }
    }

    /// Legacy target configuration; `Err(Contract)` on a netmap snapshot.
    pub fn configuration(&self) -> Result<&NodeConfiguration, Error> {
        let legacy = self.legacy()?;
        self.ensure_current()?;
        Ok(&legacy.target.configuration)
    }

    /// Legacy publication reference.
    ///
    /// # Panics
    /// On a netmap snapshot; use [`Self::netmap_ref`].
    pub fn reference(&self) -> ConfigurationRef {
        reference(&self.legacy_or_panic().target)
    }

    /// Network, Node and registered keys of the owner, for either target kind.
    pub(crate) fn self_binding(&self) -> Result<(NetworkId, NodeId, &TrustedNode), Error> {
        self.ensure_current()?;
        Ok(match &self.target {
            Target::Legacy(legacy) => {
                let cfg = &legacy.target.configuration;
                (cfg.network_id, cfg.node_id, &cfg.self_keys)
            }
            Target::Netmap(bound) => {
                let netmap = &bound.netmap.netmap;
                (netmap.network_id, netmap.node_id, &netmap.self_.self_keys)
            }
        })
    }

    /// Compare the local invalidation epoch as well as the Server binding. Used
    /// for explicit same-target renewal handoff, never to revive invalid state.
    /// For netmap snapshots this is [`Self::same_netmap`] plus equal instance.
    pub fn same_execution(&self, other: &Self) -> bool {
        match (&self.target, &other.target) {
            (Target::Legacy(a), Target::Legacy(b)) => {
                Arc::ptr_eq(&self.validity, &other.validity)
                    && a.instance == b.instance
                    && a.generation == b.generation
                    && reference(&a.target) == reference(&b.target)
                    && a.target.configuration.network_id == b.target.configuration.network_id
                    && a.target.configuration.node_id == b.target.configuration.node_id
            }
            (Target::Netmap(a), Target::Netmap(b)) => {
                self.same_netmap(other) && a.instance == b.instance
            }
            _ => false,
        }
    }

    /// Legacy: validate an owner-issued direct successor (or live same-target
    /// renewal). The returned predecessor is comparison data, never execution
    /// authority. A publication skipped by the consumer cannot carry state
    /// across a hidden withdrawal. Old absolute UTC and monotonic deadlines must
    /// still be live, even though replacement already cancelled its output
    /// guard. Always `Err` for netmap snapshots; see
    /// [`Self::netmap_continuation_from`].
    pub fn continuation_from<'a>(&self, old: &'a Self) -> Result<&'a NodeConfiguration, Error> {
        let (Target::Legacy(new), Target::Legacy(previous_target)) = (&self.target, &old.target)
        else {
            return Err(Error::Expired);
        };
        self.ensure_current()?;
        let previous = &previous_target.target.configuration;
        let current = &new.target.configuration;
        let direct = new.predecessor.as_ref().is_some_and(|(epoch, reference)| {
            Arc::ptr_eq(epoch, &previous_target.publication) && *reference == old.reference()
        });
        let same = Arc::ptr_eq(&new.publication, &previous_target.publication)
            && self.same_execution(old)
            && old.ensure_current().is_ok();
        let (now_ms, instant) = old.clock.now_with_instant()?;
        if !Weak::ptr_eq(&self.lineage, &old.lineage)
            || !(same || direct)
            || new.instance != previous_target.instance
            || new.generation != previous_target.generation
            || !old.deadlines_valid_at(now_ms, instant)
            || !previous.enabled
            || !current.enabled
            || previous.network_id != current.network_id
            || previous.node_id != current.node_id
            || previous.identity != current.identity
            || previous.addresses != current.addresses
            || previous.self_keys != current.self_keys
        {
            return Err(Error::Expired);
        }
        self.ensure_current()?;
        Ok(previous)
    }

    pub fn now_ms(&self) -> Result<u64, Error> {
        self.current()
    }

    /// Legacy: current execution permission for a retained physical adjacency.
    /// This is not peer authentication; the channel owner must retain that
    /// evidence. Netmap consumers use `meshway_policy::admission::netmap`.
    pub fn admit_adjacency(
        &self,
        peer: meshway_model::NodeId,
        carrier: &meshway_configuration::execution::Carrier,
    ) -> Result<u64, Error> {
        meshway_policy::admission::admit_adjacency(
            self.configuration()?,
            peer,
            carrier,
            self.now_ms()?,
        )
        .map_err(|_| Error::Expired)
    }

    /// Inactive-resource retention only, never I/O authority. A publication can
    /// invalidate this snapshot while its original UTC/monotonic lease bounds
    /// still allow an owner to await a direct `continuation_from` handoff.
    pub fn retention_deadline(&self) -> Result<Instant, Error> {
        let (now_ms, instant) = self.clock.now_with_instant()?;
        if self.lineage.strong_count() == 0 || !self.deadlines_valid_at(now_ms, instant) {
            return Err(Error::Expired);
        }
        let remaining = self
            .expires_at_ms()
            .checked_sub(now_ms)
            .ok_or(Error::Expired)?;
        Ok(self.expires_at().min(
            Instant::now()
                .checked_add(std::time::Duration::from_millis(remaining))
                .ok_or(Error::Expired)?,
        ))
    }

    /// Current earliest deadline: the netmap lease deadline, or the earliest of
    /// the legacy session/instance/execution deadlines. A later same-content
    /// renewal of a netmap snapshot raises it for every clone.
    pub fn deadline(&self) -> Deadline {
        match &self.target {
            Target::Legacy(legacy) => legacy
                .deadlines
                .iter()
                .min_by_key(|deadline| (deadline.expires_at_ms(), deadline.expires_at()))
                .expect("three deadlines")
                .clone(),
            Target::Netmap(bound) => bound.deadline(),
        }
    }

    pub fn expires_at(&self) -> Instant {
        match &self.target {
            Target::Legacy(legacy) => legacy
                .deadlines
                .iter()
                .map(Deadline::expires_at)
                .min()
                .expect("three deadlines"),
            Target::Netmap(bound) => bound.deadline().expires_at(),
        }
    }

    pub fn expires_at_ms(&self) -> u64 {
        match &self.target {
            Target::Legacy(legacy) => legacy
                .deadlines
                .iter()
                .map(Deadline::expires_at_ms)
                .min()
                .expect("three deadlines"),
            Target::Netmap(bound) => bound.deadline().expires_at_ms(),
        }
    }

    /// Preserve an admitted operation's absolute UTC and monotonic cutoffs on
    /// this issuer's clock. The caller must still retain its original I/O guard.
    /// The result is fixed at the current deadline; later renewal does not
    /// extend an operation cutoff taken earlier.
    pub fn deadline_at(&self, instant: Instant, wall: u64) -> Result<Deadline, Error> {
        self.ensure_current()?;
        let deadline = self.deadline();
        deadline.restrict(
            instant.min(deadline.expires_at()),
            wall.min(deadline.expires_at_ms()),
        )
    }

    /// Wait without owning a background task. The monotonic deadline bounds
    /// idle I/O; every admission/output additionally checks the shared UTC
    /// high-water clock, including a forward wall-clock adjustment.
    pub async fn invalidated(&self) {
        self.invalidated_at(u64::MAX).await;
    }

    /// Also bound work by an immutable permit deadline, using the issuer's same
    /// clock and notifications. This cannot extend the snapshot. A netmap
    /// snapshot keeps waiting across same-content renewals.
    pub async fn invalidated_at(&self, deadline_ms: u64) {
        match &self.target {
            Target::Legacy(_) => {
                if self.ensure_current().is_err() {
                    return;
                }
                self.clock
                    .wait_until(
                        self.expires_at(),
                        deadline_ms.min(self.expires_at_ms()),
                        &self.invalidated,
                    )
                    .await;
            }
            Target::Netmap(bound) => {
                bound.invalidated_at(self, deadline_ms).await;
            }
        }
    }
}

impl fmt::Debug for ExecutionSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.target {
            Target::Legacy(legacy) => f
                .debug_struct("ExecutionSnapshot")
                .field("network", &legacy.target.configuration.network_id)
                .field("node", &legacy.target.configuration.node_id)
                .field("instance", &legacy.instance)
                .field("generation", &legacy.generation)
                .field("target", &self.reference())
                .field("current", &self.ensure_current().is_ok())
                .finish_non_exhaustive(),
            Target::Netmap(bound) => f
                .debug_struct("ExecutionSnapshot")
                .field("network", &bound.netmap.netmap.network_id)
                .field("node", &bound.netmap.netmap.node_id)
                .field("netmap", &bound.netmap.reference())
                .field("instance", &bound.instance)
                .field("current", &self.ensure_current().is_ok())
                .finish_non_exhaustive(),
        }
    }
}
