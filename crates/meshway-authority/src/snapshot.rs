//! Immutable execution handoff. The issuer supplies every deadline, the shared
//! invalidation and the private target; nothing here reads saved files or mints
//! a grant from caller-asserted configuration/expiry.
use crate::{Clock, Deadline, Error, reference};
use meshway_configuration::execution::{CompiledNode, NodeConfiguration};
use meshway_model::{ConfigurationRef, InstanceId, Version};
use std::{
    fmt,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tokio_util::sync::CancellationToken;

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

/// A private target and its original control deadlines. Cloning or reading the
/// snapshot never extends them. Hosts must check immediately before admitting
/// work or releasing an output, and select `invalidated` around blocked I/O.
/// This is installation input, not proof that any component is applied.
#[derive(Clone)]
pub struct ExecutionSnapshot {
    target: Arc<CompiledNode>,
    instance: InstanceId,
    generation: Version,
    validity: Arc<AtomicBool>,
    invalidated: CancellationToken,
    clock: Arc<Clock>,
    deadlines: [Deadline; 3],
    lineage: Weak<()>,
    publication: Arc<()>,
    predecessor: Option<(Arc<()>, ConfigurationRef)>,
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
            target: issue.target,
            instance: issue.instance,
            generation: issue.generation,
            validity: issue.validity,
            invalidated: issue.invalidated,
            clock: issue.clock,
            deadlines: issue.deadlines,
            lineage: issue.lineage,
            publication: issue.publication,
            predecessor: issue.predecessor,
        };
        snapshot.ensure_current()?;
        Ok(snapshot)
    }

    pub fn instance(&self) -> meshway_protocol::InstanceRequest {
        meshway_protocol::InstanceRequest {
            instance_id: self.instance,
            generation: self.generation,
        }
    }

    pub fn ensure_current(&self) -> Result<(), Error> {
        self.current().map(|_| ())
    }

    /// The full guard with one clock reading, returning the UTC high-water
    /// milliseconds it was evaluated at.
    fn current(&self) -> Result<u64, Error> {
        self.reading().map(|reading| reading.now_ms)
    }

    /// One complete guard evaluation for one synchronous section of work. The
    /// caller applies the same configuration and time to every decision in that
    /// section and still re-checks the snapshot before committing state or
    /// releasing an output after any await. A reading is not transferable
    /// authority and must not outlive the section it was taken for.
    pub fn reading(&self) -> Result<Reading<'_>, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::detail("control.guard", 1);
        let _profile = PROFILE.enter();
        if !self.validity.load(Ordering::Acquire) || self.invalidated.is_cancelled() {
            return Err(Error::Expired);
        }
        let (now_ms, instant) = self.clock.now_with_instant()?;
        if !self.deadlines_valid_at(now_ms, instant) {
            return Err(Error::Expired);
        }
        Ok(Reading {
            configuration: &self.target.configuration,
            now_ms,
            instant,
            expires_at: self.expires_at(),
            expires_at_ms: self.expires_at_ms(),
        })
    }

    fn deadlines_valid_at(&self, now_ms: u64, instant: Instant) -> bool {
        self.deadlines
            .iter()
            .all(|deadline| deadline.valid_at(now_ms, instant))
    }

    pub fn configuration(&self) -> Result<&NodeConfiguration, Error> {
        self.ensure_current()?;
        Ok(&self.target.configuration)
    }

    pub fn reference(&self) -> ConfigurationRef {
        reference(&self.target)
    }

    /// Compare the local invalidation epoch as well as the Server binding. Used
    /// for explicit same-target renewal handoff, never to revive invalid state.
    pub fn same_execution(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.validity, &other.validity)
            && self.instance == other.instance
            && self.generation == other.generation
            && self.reference() == other.reference()
            && self.target.configuration.network_id == other.target.configuration.network_id
            && self.target.configuration.node_id == other.target.configuration.node_id
    }

    /// Validate an owner-issued direct successor (or live same-target renewal).
    /// The returned predecessor is comparison data, never execution authority.
    /// A publication skipped by the consumer cannot carry state across a hidden
    /// withdrawal. Old absolute UTC and monotonic deadlines must still be live,
    /// even though replacement already cancelled its output guard.
    pub fn continuation_from<'a>(&self, old: &'a Self) -> Result<&'a NodeConfiguration, Error> {
        self.ensure_current()?;
        let previous = &old.target.configuration;
        let current = &self.target.configuration;
        let direct = self.predecessor.as_ref().is_some_and(|(epoch, reference)| {
            Arc::ptr_eq(epoch, &old.publication) && *reference == old.reference()
        });
        let same = Arc::ptr_eq(&self.publication, &old.publication)
            && self.same_execution(old)
            && old.ensure_current().is_ok();
        let (now_ms, instant) = old.clock.now_with_instant()?;
        if !Weak::ptr_eq(&self.lineage, &old.lineage)
            || !(same || direct)
            || self.instance != old.instance
            || self.generation != old.generation
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

    /// Current execution permission for a retained physical adjacency. This is
    /// not peer authentication; the channel owner must retain that evidence.
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

    pub fn expires_at(&self) -> Instant {
        self.deadlines
            .iter()
            .map(Deadline::expires_at)
            .min()
            .expect("three deadlines")
    }

    pub fn expires_at_ms(&self) -> u64 {
        self.deadlines
            .iter()
            .map(Deadline::expires_at_ms)
            .min()
            .expect("three deadlines")
    }

    /// Wait without owning a background task. The monotonic deadline bounds
    /// idle I/O; every admission/output additionally checks the shared UTC
    /// high-water clock, including a forward wall-clock adjustment.
    pub async fn invalidated(&self) {
        self.invalidated_at(u64::MAX).await;
    }

    /// Also bound work by an immutable permit deadline, using the issuer's same
    /// clock and notifications. This cannot extend the snapshot.
    pub async fn invalidated_at(&self, deadline_ms: u64) {
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
}

impl fmt::Debug for ExecutionSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutionSnapshot")
            .field("network", &self.target.configuration.network_id)
            .field("node", &self.target.configuration.node_id)
            .field("instance", &self.instance)
            .field("generation", &self.generation)
            .field("target", &self.reference())
            .field("current", &self.ensure_current().is_ok())
            .finish_non_exhaustive()
    }
}
