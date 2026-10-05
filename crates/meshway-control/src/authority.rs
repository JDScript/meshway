//! Immutable execution handoff from the one control owner. No constructor takes
//! caller-asserted configuration/expiry; saved files alone cannot mint a grant.
use crate::{Control, Deadline, Error, clock::Clock, execution::validate_target, reference};
use meshway_configuration::execution::{CompiledNode, NodeConfiguration};
use meshway_model::{ConfigurationRef, InstanceId, Version};
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tokio_util::sync::CancellationToken;

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
    // Only Control owns the strong lineage root. Holding a snapshot must not
    // keep inactive descriptors retainable after stop/re-enrollment/fencing.
    lineage: std::sync::Weak<()>,
    publication: Arc<()>,
    predecessor: Option<(Arc<()>, ConfigurationRef)>,
}

impl Control {
    pub fn execution_snapshot(&self) -> Result<ExecutionSnapshot, Error> {
        let instance = self.current_instance()?;
        let current = self.instance.as_ref().ok_or(Error::Lifecycle)?;
        let target = self.cached_target().ok_or(Error::Lifecycle)?;
        if !target.configuration.enabled
            || current.lease.target.as_ref() != Some(&reference(target))
        {
            return Err(Error::Lifecycle);
        }
        validate_target(
            target,
            self.record.binding.as_ref().ok_or(Error::Lifecycle)?,
        )?;
        let snapshot = ExecutionSnapshot {
            target: Arc::new(target.clone()),
            instance: instance.instance_id,
            generation: instance.generation,
            validity: Arc::clone(&self.validity),
            invalidated: self.invalidated.clone(),
            clock: Arc::clone(&self.clock),
            lineage: Arc::downgrade(&self.lineage),
            publication: self.publication.clone(),
            predecessor: self.predecessor.clone(),
            deadlines: [
                self.session
                    .as_ref()
                    .ok_or(Error::Lifecycle)?
                    .deadline
                    .clone(),
                current.deadline.clone(),
                current.execution_deadline.clone().ok_or(Error::Expired)?,
            ],
        };
        snapshot.ensure_current()?;
        Ok(snapshot)
    }
}

impl ExecutionSnapshot {
    pub fn instance(&self) -> meshway_protocol::InstanceRequest {
        meshway_protocol::InstanceRequest {
            instance_id: self.instance,
            generation: self.generation,
        }
    }

    pub fn ensure_current(&self) -> Result<(), Error> {
        if !self.validity.load(Ordering::Acquire)
            || self.invalidated.is_cancelled()
            || !self.deadlines.iter().all(Deadline::valid)
        {
            return Err(Error::Expired);
        }
        Ok(())
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
        if !std::sync::Weak::ptr_eq(&self.lineage, &old.lineage)
            || !(same || direct)
            || self.instance != old.instance
            || self.generation != old.generation
            || !old.deadlines.iter().all(Deadline::valid)
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
        self.ensure_current()?;
        self.clock.now()
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
        if self.lineage.strong_count() == 0 || !self.deadlines.iter().all(Deadline::valid) {
            return Err(Error::Expired);
        }
        let remaining = self
            .expires_at_ms()
            .checked_sub(self.clock.now()?)
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

    /// Also bound work by an immutable permit deadline, using the control
    /// owner's same clock and notifications. This cannot extend the snapshot.
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
