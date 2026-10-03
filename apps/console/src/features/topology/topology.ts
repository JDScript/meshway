import type {
  ConnectionNeighborObservation,
  ConnectionRuntimeObservation,
  ConnectionRuntimesSnapshot,
  ConnectionsSnapshot,
  ConnectionTransport,
  NodeSummary,
} from "@meshway/control-client"

import {
  candidateNeighbors,
  runtimeFor,
  runtimeFreshness,
  type CandidateRelation,
  type RuntimeFreshness,
} from "./runtime.ts"

// Observed adjacencies for the topology map. Everything here is fused from
// authenticated agents' *self-reports*, scoped against the saved connection
// plan: the controller does not attest them independently, and an adjacency
// is a neighbor channel, never an IP / TUN reachability or access grant. No
// edge is ever derived that the saved plan does not describe.

/** Why a node's report cannot back a live adjacency right now. */
type ReportUnusable =
  | Exclude<RuntimeFreshness, "fresh">
  /** Fresh, but for another configuration revision than the plan shown. */
  | "revision"
  /** Fresh and current, but the agent is not applying the plan (error / disabled). */
  | "state"
  /** Revoked or absent from the active inventory. */
  | "inactive"

export type NodeReport =
  | { usable: true; observation: ConnectionRuntimeObservation }
  | {
      usable: false
      reason: ReportUnusable
      observation: ConnectionRuntimeObservation | undefined
    }

/**
 * A report may make claims only when it is fresh at `now` (it keeps ageing
 * between reads), current, of exactly the plan's revision, from an active
 * node, and the agent is applying / has applied that plan.
 */
function nodeReport(
  runtimes: ConnectionRuntimesSnapshot | null,
  nodeId: string,
  planRevision: number,
  active: boolean,
  now: number
): NodeReport {
  const observation = runtimeFor(runtimes, nodeId)
  if (!active) return { usable: false, reason: "inactive", observation }
  const freshness = runtimeFreshness(observation, now)
  if (freshness !== "fresh" || !observation) {
    return {
      usable: false,
      reason: freshness === "fresh" ? "unreported" : freshness,
      observation,
    }
  }
  if (observation.revision !== planRevision)
    return { usable: false, reason: "revision", observation }
  if (observation.state !== "applied" && observation.state !== "applying") {
    return { usable: false, reason: "state", observation }
  }
  return { usable: true, observation }
}

/** An authenticated neighbor with a recent probe and no sanitized failure. */
function isProof(peer: ConnectionNeighborObservation): boolean {
  return (
    peer.state === "authenticated" && peer.probe_fresh && peer.reason === "none"
  )
}

/** What one side of a configured channel says about it. */
export type SideClaim =
  | { kind: "proof"; peer: ConnectionNeighborObservation }
  /** Listed, but waiting, unreachable, without a fresh probe, or failing. */
  | { kind: "degraded"; peer: ConnectionNeighborObservation }
  /** Acceptor report without a candidate ID that fits several configured candidates. */
  | { kind: "ambiguous"; peer: ConnectionNeighborObservation }
  /** A usable report that does not list this channel. */
  | { kind: "absent" }
  | { kind: "unusable"; reason: ReportUnusable }

/**
 * - `confirmed`: both sides report a fresh authenticated probe.
 * - `one_sided`: exactly one side does (单侧报告); not a healthy adjacency.
 * - `degraded`: listed by a side, but not authenticated / probed.
 * - `unobserved`: no usable report mentions it.
 */
export type ChannelStatus =
  "confirmed" | "one_sided" | "degraded" | "unobserved"

export type Channel = {
  candidateId: string
  dialerNodeId: string
  acceptorNodeId: string
  listenerId: string
  transport: ConnectionTransport
  priority: number
  dialer: SideClaim
  acceptor: SideClaim
  status: ChannelStatus
}

export type Adjacency = {
  /** Unordered pair key: sorted node IDs. */
  key: string
  nodeIds: [string, string]
  /** Every configured channel between the pair, by priority. */
  channels: Channel[]
  /** Best channel status, raised to `one_sided` by an unattributable acceptor proof. */
  status: ChannelStatus
  /** An acceptor proves a channel it cannot tell apart from its sibling candidates. */
  ambiguousProof: boolean
}

export type TopologyObservation = {
  revision: number
  adjacencies: Adjacency[]
  reports: Map<string, NodeReport>
  /** Usable-report peer entries that describe no channel of the saved plan. */
  unmatchedReports: number
}

const statusRank: Record<ChannelStatus, number> = {
  unobserved: 0,
  degraded: 1,
  one_sided: 2,
  confirmed: 3,
}

function claimStatus(dialer: SideClaim, acceptor: SideClaim): ChannelStatus {
  const proofs =
    Number(dialer.kind === "proof") + Number(acceptor.kind === "proof")
  if (proofs === 2) return "confirmed"
  if (proofs === 1) return "one_sided"
  if (dialer.kind === "degraded" || acceptor.kind === "degraded")
    return "degraded"
  return "unobserved"
}

function pairKey(a: string, b: string): [string, string] {
  return a < b ? [a, b] : [b, a]
}

/**
 * Configured channels of the saved plan between active nodes, with each
 * side's claim. Matching is on the whole relation (candidate, listener,
 * peer, carrier, initiator) via `candidateNeighbors`. The dialer names its
 * exact candidate; an acceptor may not. An acceptor entry without a
 * candidate ID that fits several candidates is attributed only through a
 * dialer's exact proof, otherwise it stays ambiguous and is counted once
 * per pair rather than once per alias.
 */
export function observeTopology(
  connections: ConnectionsSnapshot,
  runtimes: ConnectionRuntimesSnapshot | null,
  inventory: NodeSummary[],
  now: number
): TopologyObservation {
  const revision = connections.revision
  const active = new Set(
    inventory
      .filter((node) => node.revoked_at === null)
      .map((node) => node.id)
      .filter((id) => {
        const entry = connections.nodes.find(
          (candidate) => candidate.node.id === id
        )
        return entry === undefined || entry.node.revoked_at === null
      })
  )
  const reports = new Map<string, NodeReport>()
  const reportOf = (id: string): NodeReport => {
    let report = reports.get(id)
    if (!report) {
      report = nodeReport(runtimes, id, revision, active.has(id), now)
      reports.set(id, report)
    }
    return report
  }
  for (const id of active) reportOf(id)

  const listeners = new Map(
    connections.configuration.listeners.map((listener) => [
      listener.id,
      listener,
    ])
  )
  const relations: (CandidateRelation & { priority: number })[] = []
  for (const candidate of connections.configuration.candidates) {
    const listener = listeners.get(candidate.listener_id)
    if (!listener) continue
    if (candidate.dialer_node_id === listener.node_id) continue
    if (!active.has(candidate.dialer_node_id) || !active.has(listener.node_id))
      continue
    relations.push({
      id: candidate.id,
      dialerNodeId: candidate.dialer_node_id,
      listenerId: listener.id,
      acceptorNodeId: listener.node_id,
      transport: listener.transport,
      priority: candidate.priority,
    })
  }

  const used = new Set<ConnectionNeighborObservation>()
  const channels: Channel[] = relations.map((relation) => {
    const dialerReport = reportOf(relation.dialerNodeId)
    let dialer: SideClaim
    if (!dialerReport.usable)
      dialer = { kind: "unusable", reason: dialerReport.reason }
    else {
      const peer = candidateNeighbors(
        dialerReport.observation,
        relation,
        relation.dialerNodeId
      )[0]
      if (peer) used.add(peer)
      dialer = peer
        ? { kind: isProof(peer) ? "proof" : "degraded", peer }
        : { kind: "absent" }
    }
    return {
      ...relation,
      candidateId: relation.id,
      dialer,
      acceptor: { kind: "absent" },
      status: "unobserved",
    }
  })

  // Acceptor side: an exact candidate ID binds directly; a null ID binds to
  // the single sibling that fits it, or to the single sibling its dialer
  // proves. Siblings share dialer and listener, hence carrier and pair.
  for (const channel of channels) {
    const report = reportOf(channel.acceptorNodeId)
    if (!report.usable)
      channel.acceptor = { kind: "unusable", reason: report.reason }
  }
  const acceptorIds = new Set(channels.map((channel) => channel.acceptorNodeId))
  for (const acceptorId of acceptorIds) {
    const report = reportOf(acceptorId)
    if (!report.usable) continue
    const mine = channels.filter(
      (channel) => channel.acceptorNodeId === acceptorId
    )
    for (const peer of report.observation.peers) {
      const fits = mine.filter((channel) =>
        candidateNeighbors(
          report.observation,
          { ...channel, id: channel.candidateId },
          acceptorId
        ).includes(peer)
      )
      if (fits.length === 0) continue
      used.add(peer)
      const exact =
        peer.candidate_id === null
          ? undefined
          : fits.find((channel) => channel.candidateId === peer.candidate_id)
      let target: Channel | undefined =
        exact ?? (fits.length === 1 ? fits[0] : undefined)
      if (!target) {
        const proven = fits.filter((channel) => channel.dialer.kind === "proof")
        if (proven.length === 1) target = proven[0]
      }
      const kind = isProof(peer) ? "proof" : "degraded"
      if (target) {
        if (target.acceptor.kind !== "proof") target.acceptor = { kind, peer }
      } else {
        for (const channel of fits) {
          if (channel.acceptor.kind === "absent")
            channel.acceptor = { kind: "ambiguous", peer }
        }
      }
    }
  }

  const pairs = new Map<string, Adjacency>()
  for (const channel of channels) {
    channel.status = claimStatus(channel.dialer, channel.acceptor)
    const nodeIds = pairKey(channel.dialerNodeId, channel.acceptorNodeId)
    const key = nodeIds.join("|")
    let pair = pairs.get(key)
    if (!pair) {
      pair = {
        key,
        nodeIds,
        channels: [],
        status: "unobserved",
        ambiguousProof: false,
      }
      pairs.set(key, pair)
    }
    pair.channels.push(channel)
  }
  for (const pair of pairs.values()) {
    pair.channels.sort(
      (a, b) =>
        a.priority - b.priority || a.candidateId.localeCompare(b.candidateId)
    )
    pair.ambiguousProof = pair.channels.some(
      (channel) =>
        channel.acceptor.kind === "ambiguous" && isProof(channel.acceptor.peer)
    )
    let status: ChannelStatus = "unobserved"
    for (const channel of pair.channels)
      if (statusRank[channel.status] > statusRank[status])
        status = channel.status
    if (pair.ambiguousProof && statusRank[status] < statusRank.one_sided)
      status = "one_sided"
    if (
      status === "unobserved" &&
      pair.channels.some((channel) => channel.acceptor.kind === "ambiguous")
    ) {
      status = "degraded"
    }
    pair.status = status
  }

  let unmatchedReports = 0
  for (const report of reports.values()) {
    if (!report.usable) continue
    for (const peer of report.observation.peers)
      if (!used.has(peer)) unmatchedReports += 1
  }

  const adjacencies = [...pairs.values()].sort((a, b) =>
    a.key.localeCompare(b.key)
  )
  return { revision, adjacencies, reports, unmatchedReports }
}

/** Live layer: a pair shows when at least one side proves it. */
export function isLive(adjacency: Adjacency): boolean {
  return adjacency.status === "confirmed" || adjacency.status === "one_sided"
}

/** Whether a live adjacency joins two nodes; a planned hop without one is not reachability. */
export function liveBetween(
  observation: TopologyObservation,
  a: string,
  b: string
): boolean {
  const key = pairKey(a, b).join("|")
  return observation.adjacencies.some(
    (adjacency) => adjacency.key === key && isLive(adjacency)
  )
}

type NodeAdjacencySummary = {
  configured: number
  confirmed: number
  oneSided: number
}

export function nodeAdjacencies(
  observation: TopologyObservation,
  nodeId: string
): Adjacency[] {
  return observation.adjacencies.filter((adjacency) =>
    adjacency.nodeIds.includes(nodeId)
  )
}

export function summarizeNode(
  observation: TopologyObservation,
  nodeId: string
): NodeAdjacencySummary {
  const mine = nodeAdjacencies(observation, nodeId)
  return {
    configured: mine.length,
    confirmed: mine.filter((adjacency) => adjacency.status === "confirmed")
      .length,
    oneSided: mine.filter((adjacency) => adjacency.status === "one_sided")
      .length,
  }
}

export function otherEnd(adjacency: Adjacency, nodeId: string): string {
  return adjacency.nodeIds[0] === nodeId
    ? adjacency.nodeIds[1]
    : adjacency.nodeIds[0]
}

/**
 * One side's own probe round trip in whole milliseconds (0 is a real
 * sub-millisecond measurement), never averaged or halved into one-way latency.
 */
export function claimRttMs(claim: SideClaim): number | null {
  return claim.kind === "proof" ? claim.peer.probe_rtt_ms : null
}
