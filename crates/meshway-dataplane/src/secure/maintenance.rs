//! Deadline-driven collection; scheduling never authorizes a target circuit.
use super::{Error, SecureNode, State, View};
use std::time::{Duration, Instant};

// A bounded fallback also collects original-scope probe/recovery caches and
// externally cancelled entries. Their output guards still fence immediately.
const INTERVAL: Duration = Duration::from_millis(100);

#[derive(Default)]
pub(super) struct Maintenance {
    next: Option<(Instant, u64)>,
}

impl Maintenance {
    pub(super) fn changed(&mut self) {
        self.next = None;
    }

    fn due(&self, reading: &View<'_>) -> bool {
        self.next
            .is_none_or(|(instant, millis)| reading.instant >= instant || reading.now_ms >= millis)
    }
}

impl SecureNode {
    // Admission collects expired flows using its own entry View. Target
    // circuit checks are independent of this schedule, including on skipped
    // sweeps. Closed circuits leave dead-CID records until their INIT window ends.
    pub(super) fn maintain_circuits(
        &mut self,
        reading: &View<'_>,
        force: bool,
    ) -> Result<(), Error> {
        self.collect_exit_flows();
        if !force && !self.maintenance.due(reading) {
            return Ok(());
        }
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.maintain");
        let _profile = PROFILE.enter();
        tracing::trace!(target: "meshway_dataplane::secure::authority_boundary", operation = "maintain",
            "Validating circuit authority");
        self.identity
            .snapshot()
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        self.probes.maintain(reading);
        self.first.maintain(reading.instant);
        self.dead.retain(|_, until| reading.now_ms < *until);
        let mut instant = reading.instant + INTERVAL;
        let mut millis = reading.now_ms.saturating_add(INTERVAL.as_millis() as u64);
        let mut closed = Vec::new();
        for (id, circuit) in &mut self.circuits {
            circuit.recovery.maintain();
            if circuit.check_at(reading).is_err() {
                circuit.retire_for("maintain", "deadline_or_cancel");
            }
            if matches!(circuit.state, State::Closed) {
                closed.push(*id);
            } else if let Some(cap) = circuit.next_deadline() {
                instant = instant.min(cap.at);
                millis = millis.min(cap.ms);
            }
        }
        for id in closed {
            if let Some(circuit) = self.circuits.remove(&id) {
                self.bury(id, &circuit, reading.now_ms);
            }
        }
        if let Some(until) = self.dead.values().min() {
            millis = millis.min(*until);
        }
        self.maintenance.next = Some((instant, millis));
        Ok(())
    }
}
