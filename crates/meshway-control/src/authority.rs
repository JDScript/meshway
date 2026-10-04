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
}

impl Control {
    pub fn execution_snapshot(&self) -> Result<ExecutionSnapshot, Error> {
        let instance = self.current_instance()?;
        let current = self.instance.as_ref().ok_or(Error::Lifecycle)?;
        let target = self.cached_target().ok_or(Error::Lifecycle)?;
        if current.lease.target.as_ref() != Some(&reference(target)) {
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

    pub fn now_ms(&self) -> Result<u64, Error> {
        self.ensure_current()?;
        self.clock.now()
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
