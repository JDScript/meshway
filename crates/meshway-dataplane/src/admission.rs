//! One entry reading for policy/flow preparation; a fresh fence at commit.
use crate::{
    Engine, Error, PacketFacts, PendingRoute,
    authority::{Lifetime, View},
    flow::{Entry, FlowTime, Policy},
    netmap::Outcome,
    secure::Guard,
};
use meshway_policy::{
    admission::{AdmissionError, Traffic},
    compile::netmap::Target,
};
use std::sync::Arc;

impl From<&View<'_>> for FlowTime {
    fn from(reading: &View<'_>) -> Self {
        // The lease is checked at use and renews; it is never copied in.
        Self {
            instant: reading.instant,
            now_ms: reading.now_ms,
        }
    }
}

impl Engine {
    /// The secure adapter authenticates the complete facts/path/producer first.
    /// The View is borrowed only by this synchronous call, never retained in
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
        guard: &Guard,
    ) -> Result<PendingRoute, Error> {
        self.admit_inner(entry, facts, reading, Some((circuit, guard)))
    }

    fn admit_inner(
        &mut self,
        entry: &Entry<'_>,
        facts: PacketFacts,
        reading: &View<'_>,
        circuit: Option<([u8; 32], &Guard)>,
    ) -> Result<PendingRoute, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("flow.admit");
        let _profile = PROFILE.enter();
        let exit = matches!(entry.grant, Target::Exit { .. });
        if exit && (facts.traffic.source.is_ipv6() || facts.traffic.destination.is_ipv6()) {
            self.exit_counters.ipv6_rejected = self.exit_counters.ipv6_rejected.saturating_add(1);
            return Err(Error::Flow);
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
            leased: reading.leased(),
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
        // A fresh View performs the same complete dynamic guard as
        // ensure_current; its time additionally fences the shorter
        // flow/circuit bounds. This is a commit fence, not a policy time source.
        let snapshot = circuit.map_or(&self.snapshot, |(_, guard)| guard.snapshot());
        let current = View::of(snapshot)?;
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
            snapshot: snapshot.clone(),
            life,
            constrained: false,
            checked,
            facts,
            section: circuit.and_then(|(_, guard)| guard.section.clone()),
            projection,
            exit_incarnation,
        })
    }
}
