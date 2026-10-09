use crate::{
    Engine, Error, Ingress, Limits, LocalFlow, Packet, PendingAction, authority::View,
    flow::FlowTable,
};
use meshway_authority::ExecutionSnapshot;

impl Engine {
    pub fn new(snapshot: ExecutionSnapshot, limits: Limits) -> Result<Self, Error> {
        View::of(&snapshot)?;
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
            decisions: Default::default(),
        })
    }

    pub fn process(&mut self, ingress: Ingress, packet: Packet) -> Result<PendingAction, Error> {
        let snapshot = self.snapshot.clone();
        let reading = View::of(&snapshot)?;
        let entry = ingress.entry(reading.node())?;
        self.admit_facts(&entry, packet.facts(), &reading)?
            .attach(packet)
    }

    /// Read the unique flow owner without advancing protocol state or deadlines.
    /// `None` means only that the packet could initiate a new protocol flow;
    /// it does not authorize its source, destination or a new demand.
    pub fn local_flow(&self, packet: &Packet) -> Result<Option<LocalFlow>, Error> {
        let reading = View::of(&self.snapshot)?;
        self.local_flow_at(packet, &reading)
    }

    pub(crate) fn local_flow_at(
        &self,
        packet: &Packet,
        reading: &View<'_>,
    ) -> Result<Option<LocalFlow>, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("flow.lookup");
        let _profile = PROFILE.enter();
        let found = self
            .flows
            .local_flow(reading.node(), &packet.facts(), reading.into())?;
        let current = View::of(&self.snapshot)?;
        match found {
            Some(found) if !found.cap.live_at(current.instant, current.now_ms) => {
                Err(Error::Expired)
            }
            Some(found) => Ok(Some(found.route)),
            None => Ok(None),
        }
    }

    /// Host-driven idle cleanup. Reading counters never changes permits/state.
    /// Invalidated entries are unusable immediately. Authority loss retains inert
    /// state for a possible Control-checked update. An ended netmap lease or an
    /// explicit invalidated().await terminates everything.
    pub fn maintain(&mut self) -> Result<(), Error> {
        let snapshot = self.snapshot.clone();
        let reading = View::of(&snapshot)?;
        self.maintain_at(&reading)
    }

    pub(crate) fn maintain_at(&mut self, reading: &View<'_>) -> Result<(), Error> {
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
