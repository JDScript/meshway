//! One entry reading for policy/flow preparation; a fresh fence at commit.
use crate::{Engine, Error, Ingress, PacketFacts, PendingRoute, flow::FlowTime, secure::Guard};
use meshway_authority::Reading;

impl From<&Reading<'_>> for FlowTime {
    fn from(reading: &Reading<'_>) -> Self {
        Self {
            instant: reading.instant,
            now_ms: reading.now_ms,
            guard_expires_at: reading.expires_at,
            guard_expires_at_ms: reading.expires_at_ms,
        }
    }
}

impl Engine {
    /// The secure adapter authenticates the complete facts/path/producer first.
    /// The Reading is borrowed only by this synchronous call, never retained in
    /// the flow table or returned PendingRoute. The latter owns its live guard.
    pub(crate) fn admit_facts(
        &mut self,
        ingress: Ingress,
        facts: PacketFacts,
        reading: &Reading<'_>,
    ) -> Result<PendingRoute, Error> {
        self.admit_inner(ingress, facts, reading, None)
    }

    pub(crate) fn admit_circuit_facts(
        &mut self,
        ingress: Ingress,
        facts: PacketFacts,
        reading: &Reading<'_>,
        circuit: [u8; 32],
        guard: &Guard,
    ) -> Result<PendingRoute, Error> {
        self.admit_inner(ingress, facts, reading, Some((circuit, guard)))
    }

    fn admit_inner(
        &mut self,
        ingress: Ingress,
        facts: PacketFacts,
        reading: &Reading<'_>,
        circuit: Option<([u8; 32], &Guard)>,
    ) -> Result<PendingRoute, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("flow.admit");
        let _profile = PROFILE.enter();
        if reading
            .configuration
            .permits
            .get(&ingress.demand())
            .is_some_and(|permit| permit.exit.is_some())
            && (facts.traffic.source.is_ipv6() || facts.traffic.destination.is_ipv6())
        {
            self.exit_counters.ipv6_rejected = self.exit_counters.ipv6_rejected.saturating_add(1);
            return Err(Error::Flow);
        }
        let mut prepared = self.flows.prepare(
            reading.configuration,
            &ingress,
            &facts,
            FlowTime::from(reading),
        )?;
        if let Some((circuit, _)) = circuit {
            prepared.bind_circuit(circuit)?;
        }
        if prepared.is_exit() {
            let (_, guard) = circuit.ok_or(Error::Ingress)?;
            if prepared.hop_is_exit()
                && reading
                    .configuration
                    .exit_role
                    .as_ref()
                    .is_none_or(|role| role.instance_id != self.snapshot.instance().instance_id)
            {
                return Err(Error::ExecutionScope);
            }
            self.flows
                .prepare_projection(&mut prepared, guard, reading)?;
        }
        tracing::trace!(target: "meshway_dataplane::flow::authority_boundary", operation = "commit",
            "Validating flow admission authority");
        // reading() performs the same complete dynamic guard as ensure_current;
        // its fresh time additionally fences the shorter flow/circuit deadlines.
        // This is a commit fence, not another policy/admission time source.
        let snapshot = circuit.map_or(&self.snapshot, |(_, guard)| guard.snapshot());
        let current = snapshot.reading()?;
        if let Some((_, guard)) = circuit {
            guard.check_at(&current).map_err(|_| Error::Expired)?;
        }
        if current.now_ms >= prepared.permit_expires_at_ms || current.instant >= prepared.expires_at
        {
            return Err(Error::Expired);
        }
        let expires_at = prepared.expires_at;
        let permit_expires_at_ms = prepared.permit_expires_at_ms;
        let (flow_cancellation, projection) = prepared.exit_guards();
        let checked = self.flows.commit(prepared);
        Ok(PendingRoute {
            snapshot: snapshot.clone(),
            expires_at,
            permit_expires_at_ms,
            checked,
            facts,
            cancellation: None,
            section: circuit.and_then(|(_, guard)| guard.section.clone()),
            flow_cancellation,
            projection,
        })
    }
}
