import type {
  AgentConnectionsSnapshot,
  ConnectionRuntimeObservation,
  ConnectionTransport,
} from "../../packages/control-client/src/index.ts";

/** Reports are self-observations. Demand both a current plan and receive freshness. */
export function applied(
  observation: ConnectionRuntimeObservation | undefined,
  expected: AgentConnectionsSnapshot,
  now = Date.now(),
): boolean {
  if (!observation || !observation.current || observation.stale || observation.state !== "applied"
    || observation.reason !== "none" || observation.epoch < 1 || observation.sequence < 1
    || observation.node_id !== expected.payload.node_id
    || observation.revision !== expected.payload.revision || observation.digest !== expected.digest
    || observation.reported_at === null || !expected.payload.valid) return false;
  const age = now - Date.parse(observation.reported_at);
  if (!Number.isFinite(age) || age < -2_000 || age > 6_000) return false;
  return observation.listeners.length === expected.payload.listeners.length
    && expected.payload.listeners.every((listener) => observation.listeners.some((actual) =>
      actual.listener_id === listener.id && actual.state === "bound" && actual.reason === "none"));
}

/** Opening a TCP socket or finishing Noise is deliberately insufficient here. */
export function probed(
  observation: ConnectionRuntimeObservation,
  peer: string,
  initiator: string,
  transport: ConnectionTransport,
  listener: string,
  candidate: string,
): boolean {
  return observation.peers.some((entry) => entry.peer_node_id === peer
    && entry.initiator_node_id === initiator && entry.transport === transport
    && entry.listener_id === listener
    && (entry.candidate_id === candidate || (initiator !== observation.node_id && entry.candidate_id === null))
    && entry.state === "authenticated" && entry.reason === "none" && entry.probe_fresh
    && entry.probe_rtt_ms !== null && Number.isInteger(entry.probe_rtt_ms) && entry.probe_rtt_ms >= 0);
}
