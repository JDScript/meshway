use super::{Carrier, Error, PendingFrame, SecureNode};
use crate::exit::{Counters, ExitProjection, ExitReplyIngress, Limits};
use meshway_authority::DataPlaneIdentity;
use meshway_model::NodeId;
use std::{collections::BTreeMap, time::Instant};

impl SecureNode {
    /// Explicit exit budgets; ordinary construction uses `exit::Limits::default`.
    pub fn with_exit_limits(
        identity: DataPlaneIdentity,
        limits: super::Limits,
        carriers: BTreeMap<NodeId, Carrier>,
        exit: Limits,
    ) -> Result<Self, Error> {
        let mut node = Self::new(identity, limits, carriers)?;
        node.engine
            .flows
            .set_exit_limits(exit)
            .map_err(|_| Error::Limits)?;
        Ok(node)
    }

    /// Drain one move-only event. The host executes it with the given scope and
    /// invokes its ACK callback. No task, blocking command or second flow table
    /// is needed. Retire can be drained after execution authority is gone.
    pub fn next_exit_projection(&mut self) -> Option<ExitProjection> {
        self.collect_exit_flows();
        self.engine.flows.next_exit_projection()
    }

    pub(super) fn collect_exit_flows(&mut self) {
        if !self.engine.flows.has_exit_work() {
            return;
        }
        if let Ok(reading) = self.identity.snapshot().reading() {
            self.engine.flows.expire(reading.instant, reading.now_ms);
        } else {
            if self.identity.snapshot().retention_deadline().is_err() {
                self.engine.flows.retire_exits();
            }
            // Original monotonic deadlines and failed ACKs still retire while
            // awaiting an explicit Control continuation. Never invent UTC time.
            self.engine.flows.expire(Instant::now(), 0);
        }
        for id in self.engine.flows.retired_exit_circuits() {
            // Several tuples may share one admitted cipher. The retired flow's
            // own token fences its queued outputs; do not kill other live flows.
            if self.engine.flows.has_exit_circuit(id) {
                continue;
            }
            if let Some(circuit) = self.circuits.get_mut(&id) {
                circuit.retire_for("exit", "flow_retired");
            }
        }
    }

    /// Explicit suspend/replacement hook: invalidate flow guards first, then
    /// expose Retire operations. Drain these before dropping the owner.
    pub fn retire_exits(&mut self) {
        self.engine.flows.retire_exits();
        self.collect_exit_flows();
    }

    /// The owned exit TUN's reverse-NAT input. Lookup and reply admission use the
    /// same FlowTable and original CID/path. No reverse demand is compiled.
    pub fn exit_reply(&mut self, ingress: ExitReplyIngress) -> Result<PendingFrame, Error> {
        self.collect_exit_flows();
        if ingress.packet.facts().traffic.source.is_ipv6()
            || ingress.packet.facts().traffic.destination.is_ipv6()
        {
            self.engine.exit_counters.ipv6_rejected =
                self.engine.exit_counters.ipv6_rejected.saturating_add(1);
            return Err(Error::Packet);
        }
        let result = (|| {
            let route = match self.local_flow(&ingress.packet) {
                Ok(Some(route)) => route,
                Ok(None) | Err(Error::Flow) => {
                    self.engine.exit_counters.unassociated_reply = self
                        .engine
                        .exit_counters
                        .unassociated_reply
                        .saturating_add(1);
                    return Err(Error::Flow);
                }
                Err(error) => return Err(error),
            };
            let snapshot = self.identity.snapshot();
            let reading = snapshot.reading().map_err(|_| Error::Authority)?;
            if route.path.last() != Some(&reading.configuration.node_id)
                || !reading
                    .configuration
                    .permits
                    .get(&route.demand)
                    .is_some_and(|permit| permit.exit.is_some())
            {
                return Err(Error::Flow);
            }
            let id = route.circuit.ok_or(Error::Flow)?;
            self.send(id, ingress.packet)
        })();
        if result.is_err() {
            self.engine.exit_counters.reply_rejected =
                self.engine.exit_counters.reply_rejected.saturating_add(1);
        }
        result
    }

    pub fn exit_counters(&self) -> Counters {
        self.engine.exit_counters
    }
}
