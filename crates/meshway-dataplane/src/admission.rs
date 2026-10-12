//! One entry reading for policy/flow preparation; a fresh check at commit.
use crate::{
    Engine, Error, PacketFacts, PendingRoute, Quoted, Transport,
    authority::{Lifetime, View},
    flow::{Entry, FlowTime, Policy},
    netmap::Outcome,
    secure::OutputBinding,
};
use meshway_policy::{
    admission::{AdmissionError, Traffic},
    compile::netmap::Target,
};
use std::sync::Arc;

impl From<&View<'_>> for FlowTime {
    fn from(reading: &View<'_>) -> Self {
        Self {
            instant: reading.instant,
            now_ms: reading.now_ms,
        }
    }
}

impl Engine {
    /// The secure adapter authenticates the complete facts/path/producer first.
    /// The View is borrowed only by this synchronous call, never kept in
    /// the flow table or returned PendingRoute. The latter owns its live guard.
    pub(crate) fn admit_facts(
        &mut self,
        entry: &Entry<'_>,
        facts: PacketFacts,
        reading: &View<'_>,
    ) -> Result<PendingRoute, Error> {
        self.admit_inner(entry, facts, reading, None)
    }

    pub(crate) fn admit_circuit_facts(
        &mut self,
        entry: &Entry<'_>,
        facts: PacketFacts,
        reading: &View<'_>,
        circuit: [u8; 32],
        guard: &OutputBinding,
    ) -> Result<PendingRoute, Error> {
        self.admit_inner(entry, facts, reading, Some((circuit, guard)))
    }

    fn admit_inner(
        &mut self,
        entry: &Entry<'_>,
        facts: PacketFacts,
        reading: &View<'_>,
        circuit: Option<([u8; 32], &OutputBinding)>,
    ) -> Result<PendingRoute, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("flow.admit");
        let _profile = PROFILE.enter();
        let exit = matches!(entry.grant, Target::Exit { .. });
        if exit && (facts.traffic.source.is_ipv6() || facts.traffic.destination.is_ipv6()) {
            self.exit_counters.ipv6_rejected = self.exit_counters.ipv6_rejected.saturating_add(1);
            return Err(Error::Flow);
        }
        if let Transport::Error(error) = facts.transport {
            let result = self.admit_related(entry, facts, &error.quoted, reading, circuit);
            let counters = &mut self.related_counters;
            let counter = match &result {
                Ok(_) => &mut counters.admitted,
                Err(Error::Capacity) => &mut counters.limited,
                Err(_) => &mut counters.rejected,
            };
            *counter = counter.saturating_add(1);
            return result;
        }
        let node = reading.node();
        let origin = entry.origin;
        let decisions = &mut self.decisions;
        let mut decide =
            |traffic: &Traffic| -> Result<Arc<meshway_policy::compile::netmap::Decision>, Error> {
                let netmap = reading.netmap();
                match decisions.decide(
                    netmap,
                    origin,
                    traffic.destination,
                    (traffic.protocol, traffic.destination_port),
                )? {
                    Outcome::Permitted(decision) => Ok(decision),
                    Outcome::Denied(_) => Err(Error::Admission(AdmissionError::Traffic)),
                    Outcome::Direct | Outcome::Unmanaged => Err(Error::Flow),
                }
            };
        let policy = Policy {
            netmap: &reading.netmap().netmap,
            compiler: reading.desired.compiler(),
            decide: &mut decide,
        };
        let mut prepared =
            self.flows
                .prepare(policy, node, entry, &facts, FlowTime::from(reading))?;
        if let Some((circuit, _)) = circuit {
            prepared.bind_circuit(circuit)?;
        }
        if prepared.is_exit() {
            let (_, guard) = circuit.ok_or(Error::Ingress)?;
            self.flows
                .prepare_projection(&mut prepared, guard, reading.now())?;
        }
        tracing::trace!(target: "meshway_dataplane::flow::authority_boundary", operation = "commit",
            "Validating flow admission authority");
        // A fresh clock reading checks the shorter flow/circuit bounds at the
        // commit point; it is not a policy time source.
        let current = View::of(reading.desired);
        if let Some((_, guard)) = circuit {
            guard.check_at(&current).map_err(|_| Error::Expired)?;
        }
        if !prepared.cap.live_at(current.instant, current.now_ms)
            || !prepared.exit_live_at(current.instant, current.now_ms)
        {
            return Err(Error::Expired);
        }
        let cap = prepared.cap;
        let (flow_cancellation, projection) = prepared.exit_guards();
        let exit_incarnation = prepared.exit_incarnation();
        let checked = self.flows.commit(prepared);
        let mut life = Lifetime::new(Some(cap), Default::default());
        life.flow = flow_cancellation;
        Ok(PendingRoute {
            life,
            constrained: false,
            checked,
            facts,
            projection,
            exit_incarnation,
        })
    }

    /// An ICMP error: never a flow of its own. It rides the reply direction of
    /// the live flow its quote names, on that flow's own circuit and guards,
    /// without refreshing or advancing it.
    fn admit_related(
        &mut self,
        entry: &Entry<'_>,
        facts: PacketFacts,
        quoted: &Quoted,
        reading: &View<'_>,
        circuit: Option<([u8; 32], &OutputBinding)>,
    ) -> Result<PendingRoute, Error> {
        let netmap = &reading.netmap().netmap;
        let related = self.flows.related(
            reading.node(),
            entry,
            quoted,
            circuit.map(|(id, _)| id),
            FlowTime::from(reading),
            |decision, forward| {
                meshway_policy::admission::netmap::admit_related(
                    netmap,
                    decision,
                    entry.path,
                    entry.origin,
                    entry.previous,
                    forward,
                    &quoted.traffic,
                    facts.traffic.source,
                    facts.traffic.destination,
                )
                .map_err(Error::from)
            },
        )?;
        let current = View::of(reading.desired);
        if let Some((_, guard)) = circuit {
            guard.check_at(&current).map_err(|_| Error::Expired)?;
        }
        if !related.cap.live_at(current.instant, current.now_ms) {
            return Err(Error::Expired);
        }
        let mut life = Lifetime::new(Some(related.cap), Default::default());
        life.flow = related.flow;
        Ok(PendingRoute {
            life,
            constrained: false,
            checked: related.checked,
            facts,
            projection: related.projection,
            exit_incarnation: None,
        })
    }
}
