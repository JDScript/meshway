import type {
  ConnectionNeighborObservation,
  ConnectionRuntimeObservation,
  ConnectionRuntimesSnapshot,
  ConnectionTransport,
} from "@meshway/control-client"

// The agent's self-reported connection runtime. A report is what an
// authenticated agent says about itself; the controller only checks its
// scope. Nothing here is a controller trace, an IP-flow grant or proof that
// routed / TUN traffic works.

/**
 * - `unreported`: the controller has no lease for this node at all.
 * - `awaiting`: a lease exists but no report for it yet (new process / epoch).
 * - `withdrawn`: revoked or withdrawn; any resources are history.
 * - `outdated`: the report describes a different configuration revision.
 * - `expired`: received too long ago (server flag or aged past the TTL here).
 * - `fresh`: current configuration, received within the TTL.
 */
export type RuntimeFreshness =
  "unreported" | "awaiting" | "withdrawn" | "outdated" | "expired" | "fresh"

export function runtimeFor(
  snapshot: ConnectionRuntimesSnapshot | null,
  nodeId: string
): ConnectionRuntimeObservation | undefined {
  return snapshot?.nodes.find((observation) => observation.node_id === nodeId)
}

/** Seconds since the controller received the report, at `now`; null when never reported. */
function reportAge(
  observation: ConnectionRuntimeObservation,
  now: number
): number | null {
  if (observation.reported_at === null) return null
  const received = Date.parse(observation.reported_at)
  if (Number.isNaN(received)) return null
  return Math.max(0, (now - received) / 1000)
}

/**
 * Freshness at `now`. The server's `current` / `stale` flags are honoured as
 * read, and the report keeps ageing in the browser after it was fetched.
 */
export function runtimeFreshness(
  observation: ConnectionRuntimeObservation | undefined,
  now: number
): RuntimeFreshness {
  if (!observation) return "unreported"
  if (observation.state === "withdrawn") return "withdrawn"
  if (
    observation.state === "awaiting_report" ||
    observation.reported_at === null
  )
    return "awaiting"
  if (!observation.current) return "outdated"
  const age = reportAge(observation, now)
  if (
    observation.stale ||
    age === null ||
    age >= observation.stale_after_seconds
  )
    return "expired"
  return "fresh"
}

export type CandidateRelation = {
  id: string
  dialerNodeId: string
  listenerId: string
  /** Owner of the listener. */
  acceptorNodeId: string
  transport: ConnectionTransport
}

/**
 * Peers reported for one configured candidate, matched on the whole
 * relation (peer, initiator, carrier, listener), never the candidate ID
 * alone: an ID kept across a retarget must not inherit the old target's
 * observation. The dialer reports its exact candidate ID; the acceptor may
 * report null when it cannot tell configured candidates apart.
 */
export function candidateNeighbors(
  observation: ConnectionRuntimeObservation | undefined,
  candidate: CandidateRelation,
  selfNodeId: string
): ConnectionNeighborObservation[] {
  if (!observation) return []
  const id = candidate.id.trim()
  const outgoing = selfNodeId === candidate.dialerNodeId
  if (!outgoing && selfNodeId !== candidate.acceptorNodeId) return []
  return observation.peers.filter(
    (peer) =>
      peer.transport === candidate.transport &&
      peer.listener_id === candidate.listenerId &&
      peer.initiator_node_id === candidate.dialerNodeId &&
      peer.peer_node_id ===
        (outgoing ? candidate.acceptorNodeId : candidate.dialerNodeId) &&
      (outgoing
        ? peer.candidate_id === id
        : peer.candidate_id === null || peer.candidate_id === id)
  )
}
