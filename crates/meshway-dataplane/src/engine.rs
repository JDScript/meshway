use crate::{
    Engine, Error, Ingress, Limits, LocalFlow, Packet, PendingAction,
    flow::{FlowTable, FlowTime},
};
use meshway_authority::{ExecutionSnapshot, Reading};

impl Engine {
    pub fn new(snapshot: ExecutionSnapshot, limits: Limits) -> Result<Self, Error> {
        snapshot.reading()?;
        Self::new_current(snapshot, limits)
    }

    // SecureNode already took its entry reading. Construction still performs
    // the final dynamic check here, immediately before publishing flow state.
    pub(crate) fn new_current(snapshot: ExecutionSnapshot, limits: Limits) -> Result<Self, Error> {
        let limits = limits.validate()?;
        snapshot.ensure_current()?;
        Ok(Self {
            snapshot,
            flows: FlowTable::new(limits),
            exit_counters: Default::default(),
        })
    }

    /// Same-target renewal, using the shared reconciliation implementation.
    pub fn refresh(&mut self, snapshot: ExecutionSnapshot) -> Result<(), Error> {
        if !self.snapshot.same_execution(&snapshot) {
            return Err(Error::ExecutionScope);
        }
        self.reconcile(snapshot)
    }

    /// Reconcile a Control-issued successor transactionally. Old queued actions
    /// keep their original cancelled guard. Retained flow deadlines/idle time
    /// are unchanged; only a subsequently admitted packet can refresh them.
    pub fn reconcile(&mut self, snapshot: ExecutionSnapshot) -> Result<(), Error> {
        let reading = snapshot.reading()?;
        let flows = self.reconciled_flows(&snapshot, &reading)?;
        snapshot.continuation_from(&self.snapshot)?;
        self.flows.replace(flows);
        self.snapshot = snapshot;
        Ok(())
    }

    pub(crate) fn reconciled_flows(
        &self,
        snapshot: &ExecutionSnapshot,
        reading: &Reading<'_>,
    ) -> Result<FlowTable, Error> {
        let old = snapshot.continuation_from(&self.snapshot)?;
        let mut flows =
            self.flows
                .reconciled(old, reading.configuration, FlowTime::from(reading))?;
        flows.project_successor(snapshot, reading);
        Ok(flows)
    }

    pub fn process(&mut self, ingress: Ingress, packet: Packet) -> Result<PendingAction, Error> {
        let snapshot = self.snapshot.clone();
        let reading = snapshot.reading()?;
        self.admit_facts(ingress, packet.facts(), &reading)?
            .attach(packet)
    }

    /// Read the unique flow owner without advancing protocol state or deadlines.
    /// `None` means only that the packet could initiate a new protocol flow;
    /// it does not authorize its source, destination or a new demand.
    pub fn local_flow(&self, packet: &Packet) -> Result<Option<LocalFlow>, Error> {
        let reading = self.snapshot.reading()?;
        self.local_flow_at(packet, &reading)
    }

    pub(crate) fn local_flow_at(
        &self,
        packet: &Packet,
        reading: &Reading<'_>,
    ) -> Result<Option<LocalFlow>, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("flow.lookup");
        let _profile = PROFILE.enter();
        let found = self.flows.local_flow(
            reading.configuration,
            &packet.facts(),
            FlowTime::from(reading),
        )?;
        let current = self.snapshot.reading()?;
        match found {
            Some(found)
                if current.instant >= found.expires_at || current.now_ms >= found.expires_at_ms =>
            {
                Err(Error::Expired)
            }
            Some(found) => Ok(Some(found.route)),
            None => Ok(None),
        }
    }

    /// Host-driven idle cleanup. Reading counters never changes permits/state.
    /// Invalidated entries are unusable immediately. Authority loss retains inert
    /// state for a possible Control-approved successor; reconciliation checks
    /// every original expiry. Explicit invalidated().await/drop still tears down.
    pub fn maintain(&mut self) -> Result<(), Error> {
        let snapshot = self.snapshot.clone();
        let reading = snapshot.reading()?;
        self.maintain_at(&reading)
    }

    pub(crate) fn maintain_at(&mut self, reading: &Reading<'_>) -> Result<(), Error> {
        self.snapshot.ensure_current()?;
        self.flows.expire(reading.instant, reading.now_ms);
        Ok(())
    }

    /// Number of retained entries; deliberately does not refresh or expire them.
    pub fn retained_flows(&self) -> usize {
        self.flows.len()
    }

    /// Caller-driven cancellation hook; no spawned task or independent lease.
    pub async fn invalidated(&mut self) {
        self.snapshot.invalidated().await;
        self.flows.clear();
    }
}
