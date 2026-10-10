use super::{Carrier, Error, PendingFrame, SecureNode, View};
use crate::exit::{Counters, ExitProjection, ExitReplyIngress, FlowIncarnation, Limits};
use meshway_authority::DataPlaneKeys;
use meshway_engine::Desired;
use meshway_model::NodeId;
use std::collections::BTreeMap;

impl SecureNode {
    /// Explicit exit budgets; ordinary construction uses `exit::Limits::default`.
    pub fn with_exit_limits(
        desired: &Desired,
        keys: DataPlaneKeys,
        limits: super::Limits,
        carriers: BTreeMap<NodeId, Carrier>,
        exit: Limits,
    ) -> Result<Self, Error> {
        let mut node = Self::new(desired, keys, limits, carriers)?;
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
        let reading = View::of(&self.desired);
        self.engine.flows.expire(reading.instant, reading.now_ms);
        for id in self.engine.flows.retired_exit_circuits() {
            // Several tuples may share one admitted cipher. The retired flow's
            // own token fences its queued outputs; do not kill other live flows.
            if self.engine.flows.has_exit_circuit(id) {
                continue;
            }
            if let Some(mut circuit) = self.circuits.remove(&id) {
                circuit.retire_for("exit", "flow_retired");
                self.bury(id, &circuit, crate::authority::now_ms());
            }
        }
    }

    /// Explicit suspend/replacement hook: invalidate flow guards first, then
    /// expose Retire operations. Drain these before dropping the owner.
    pub fn retire_exits(&mut self) {
        self.engine.flows.retire_exits();
        self.collect_exit_flows();
    }

    /// Report failure of one exact projection, including confirmation failure
    /// after a successful ACK. Cancels its queued outputs and exposes Retire.
    /// A shared cipher stays live until its last flow ends. Unknown or already
    /// retired incarnations are no-ops.
    pub fn fail_exit_projection(&mut self, flow: &FlowIncarnation) {
        if self.engine.flows.fail_exit_projection(flow) {
            self.collect_exit_flows();
        }
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
            let desired = &self.desired;
            let reading = View::of(desired);
            let exit = matches!(route.grant, super::Target::Exit { .. });
            if route.path.last() != Some(&reading.node()) || !exit {
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
