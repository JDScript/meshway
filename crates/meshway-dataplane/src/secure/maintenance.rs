//! Deadline-driven collection; scheduling never authorizes a target circuit.
use super::{Error, SecureNode, State};
use meshway_authority::Reading;
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

    fn due(&self, reading: &Reading<'_>) -> bool {
        self.next
            .is_none_or(|(instant, millis)| reading.instant >= instant || reading.now_ms >= millis)
    }
}

impl SecureNode {
    // Admission collects expired flows using its own entry Reading. Target
    // circuit checks are independent of this schedule, including on skipped
    // sweeps. Closed entries are permanent bounded replay tombstones.
    pub(super) fn maintain_circuits(
        &mut self,
        reading: &Reading<'_>,
        force: bool,
    ) -> Result<(), Error> {
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
        let mut instant = reading.instant + INTERVAL;
        let mut millis = reading.now_ms.saturating_add(INTERVAL.as_millis() as u64);
        for circuit in self.circuits.values_mut() {
            circuit.recovery.maintain();
            if circuit.check_at(reading).is_err() {
                circuit.retire_for("maintain", "deadline_or_cancel");
            }
            if !matches!(circuit.state, State::Closed) {
                instant = instant.min(circuit.expires_at);
                millis = millis.min(circuit.expires_at_ms);
                if !circuit.state.established() {
                    instant = instant.min(circuit.handshake_at);
                    millis = millis.min(circuit.handshake_at_ms);
                }
            }
        }
        self.maintenance.next = Some((instant, millis));
        Ok(())
    }
}
