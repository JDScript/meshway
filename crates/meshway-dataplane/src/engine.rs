use crate::{
    Engine, Error, Ingress, Limits, LocalFlow, Packet, PendingAction, authority::View,
    flow::FlowTable, netmap::Decisions,
};
use meshway_engine::Desired;

impl Engine {
    pub fn new(desired: &Desired, limits: Limits) -> Result<Self, Error> {
        let limits = limits.validate()?;
        Ok(Self {
            decisions: Decisions::for_desired(desired),
            desired: desired.clone(),
            flows: FlowTable::new(limits),
            exit_counters: Default::default(),
        })
    }

    pub fn process(&mut self, ingress: Ingress, packet: Packet) -> Result<PendingAction, Error> {
        let desired = self.desired.clone();
        let reading = View::of(&desired);
        let entry = ingress.entry(reading.node())?;
        self.admit_facts(&entry, packet.facts(), &reading)?
            .attach(packet)
    }

    /// Read the unique flow owner without advancing protocol state or deadlines.
    /// `None` means only that the packet could initiate a new protocol flow;
    /// it does not authorize its source, destination or a new flow.
    pub fn local_flow(&self, packet: &Packet) -> Result<Option<LocalFlow>, Error> {
        self.local_flow_at(packet, &View::of(&self.desired))
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
        match found {
            Some(found) if !found.cap.live_at(reading.instant, reading.now_ms) => {
                Err(Error::Expired)
            }
            Some(found) => Ok(Some(found.route)),
            None => Ok(None),
        }
    }

    /// Host-driven idle cleanup. Reading counters never changes state.
    pub fn maintain(&mut self) -> Result<(), Error> {
        let desired = self.desired.clone();
        self.maintain_at(&View::of(&desired));
        Ok(())
    }

    pub(crate) fn maintain_at(&mut self, reading: &View<'_>) {
        self.flows.expire(reading.instant, reading.now_ms);
    }

    /// Number of retained entries; deliberately does not refresh or expire them.
    pub fn retained_flows(&self) -> usize {
        self.flows.len()
    }
}
