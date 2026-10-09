//! Control is the production issuer of execution authority. Every snapshot is
//! bound to the current lease, the durable private target and this owner's
//! shared validity/invalidation; saved files alone cannot mint a grant.
use crate::{
    Control, Error,
    execution::{validate_exit_binding, validate_target},
};
use meshway_authority::{DataPlaneIdentity, ExecutionSnapshot, Issue, reference};
use std::sync::Arc;

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
        validate_exit_binding(&target.configuration, instance.instance_id)?;
        Ok(ExecutionSnapshot::issue(Issue {
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
        })?)
    }

    /// Packet-plane keys for exactly this owner's current snapshot. The key
    /// comparison against the registered target happens inside the identity.
    pub fn dataplane_identity(
        &self,
        snapshot: ExecutionSnapshot,
    ) -> Result<DataPlaneIdentity, Error> {
        snapshot.ensure_current()?;
        let current = self.execution_snapshot()?;
        if !current.same_execution(&snapshot) {
            return Err(Error::Contract);
        }
        let keys = self.record.keys.as_ref().ok_or(Error::Lifecycle)?;
        Ok(DataPlaneIdentity::new(
            snapshot,
            keys.signing_key()?,
            keys.noise_key()?,
        )?)
    }
}
