use crate::{Control, Error};
use meshway_configuration::execution::{BusinessTarget, DemandDecision};
use meshway_protocol::{CancelDemandRequest, DemandRequest, DemandResponse, RenewDemandRequest};

impl Control {
    /// Extend an existing demand to an explicit absolute deadline. The Server
    /// owns idempotence and the current-instance cap. Feedback does not renew a
    /// local permit: synchronize, fsync and obtain a new execution handoff first.
    pub async fn renew_demand(
        &mut self,
        request: &RenewDemandRequest,
    ) -> Result<DemandResponse, Error> {
        if request.instance != self.current_instance()? {
            return Err(Error::Lifecycle);
        }
        let response = self.client.renew_node_demand(request, self.token()?).await;
        let response = self.registered(response)?;
        self.current_instance()?;
        if let DemandDecision::Permitted { permit } = &response.decision
            && (permit.demand_id != request.request_id
                || Some(permit.source)
                    != self.record.binding.as_ref().map(|binding| binding.node_id))
        {
            self.fence();
            return Err(Error::Contract);
        }
        Ok(response)
    }

    /// Submit an explicitly identified demand through the existing Node session.
    /// Retries reuse the caller's request ID. The decision is feedback only: a
    /// packet still needs a synchronized execution target and current snapshot.
    pub async fn create_demand(
        &mut self,
        request: &DemandRequest,
    ) -> Result<DemandResponse, Error> {
        if request.instance != self.current_instance()? {
            return Err(Error::Lifecycle);
        }
        let response = self.client.create_node_demand(request, self.token()?).await;
        let response = self.registered(response)?;
        self.current_instance()?;
        if let DemandDecision::Permitted { permit } = &response.decision {
            let source = self
                .record
                .binding
                .as_ref()
                .ok_or(Error::Lifecycle)?
                .node_id;
            // An assigned Mesh IP may be canonicalized to its Node identity by
            // the Server. The synchronized target will carry its actual address
            // binding, checked by policy at packet admission.
            let target_matches = request.target == permit.target
                || matches!(
                    (&request.target, &permit.target),
                    (BusinessTarget::Ip { .. }, BusinessTarget::Mesh { .. })
                );
            if permit.demand_id != request.request_id
                || permit.source != source
                || permit.protocol != request.protocol
                || permit.port != request.port
                || permit.profile != request.profile
                || !target_matches
            {
                self.fence();
                return Err(Error::Contract);
            }
        }
        Ok(response)
    }

    /// Cancel only a demand belonging to the current instance. Synchronize the
    /// target after success; revoke local snapshots before dispatch so even an
    /// uncertain acknowledgement cannot keep using the removed local demand.
    /// Cancellation conservatively breaks continuation for the whole local
    /// owner. A same-ID regrant before synchronization cannot resurrect a flow;
    /// other Server demands remain intact and may establish fresh local state.
    pub async fn cancel_demand(&mut self, request: &CancelDemandRequest) -> Result<(), Error> {
        if request.instance != self.current_instance()? {
            return Err(Error::Lifecycle);
        }
        self.invalidate_snapshots();
        if let Some(instance) = &mut self.instance {
            instance.execution_deadline = None;
        }
        let response = self.client.cancel_node_demand(request, self.token()?).await;
        self.registered(response)?;
        self.current_instance()?;
        Ok(())
    }
}
