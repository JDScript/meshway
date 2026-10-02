import type { ConnectionRuntimesSnapshot, ConnectionsSnapshot, ConnectionTransport, NodeSummary } from "@meshway/control-client"

import {
  channelStatusLabels,
  claimLabel,
  claimRtt,
  isLive,
  observeTopology,
  type Adjacency,
  type ChannelStatus,
  type TopologyObservation,
} from "./topology-model.ts"

// The overview network map. Two inputs are kept strictly apart:
// - where a node is: only a typed placement with a source and observation
//   time. No such source exists yet (no GeoIP / location interface), so every
//   node is unplaced. Names, Mesh IPs and the legacy administrator country
//   label are never treated as locations.
// - how nodes are connected: only configured channels with the agents' own
//   fresh reports (topology-model). Grants and planned paths are not links.

/** A future geographic placement: region-level at best, always with provenance. */
export type GeoPlacement = {
  kind: "geo"
  lat: number
  lon: number
  /** Region-level aggregation; never presented as an exact position. */
  precision: "region"
  /** e.g. "controller GeoIP of the STUN-observed underlay IP". */
  source: string
  observedAt: string
}

export type NodePlacement = GeoPlacement | { kind: "unplaced" }

/**
 * Where a node may be drawn on the world map. There is no location source
 * in the controller yet, so the answer is always "unplaced"; in particular
 * `node.country` (administrator metadata) is deliberately ignored.
 */
export function nodePlacement(node: NodeSummary, sources: ReadonlyMap<string, GeoPlacement> = new Map()): NodePlacement {
  return sources.get(node.id) ?? { kind: "unplaced" }
}

/** Equirectangular projection into the dot grid's coordinate space (degrees). */
export function project(lat: number, lon: number, grid: { north: number; west: number }): { x: number; y: number } {
  return { x: lon - grid.west, y: grid.north - lat }
}

/**
 * The relationship layout for unplaced nodes: a ring in name order. It
 * states only "these are the nodes and their channels", never where they are.
 */
export function relationshipLayout(nodes: NodeSummary[], size: number): Map<string, { x: number; y: number }> {
  const sorted = [...nodes].sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id))
  const center = size / 2
  const radius = sorted.length <= 1 ? 0 : size * (sorted.length <= 3 ? 0.27 : 0.33)
  const positions = new Map<string, { x: number; y: number }>()
  sorted.forEach((node, index) => {
    const angle = -Math.PI / 2 + (index / Math.max(sorted.length, 1)) * Math.PI * 2
    positions.set(node.id, { x: center + radius * Math.cos(angle), y: center + radius * Math.sin(angle) })
  })
  return positions
}

/** How an edge is drawn. Only `confirmed` and `one_sided` carry a fresh authenticated proof. */
export type EdgeLook = "confirmed" | "one_sided" | "degraded" | "configured"

export function edgeLook(status: ChannelStatus): EdgeLook {
  return status === "unobserved" ? "configured" : status
}

export const edgeLookLabels: Record<EdgeLook, string> = {
  confirmed: channelStatusLabels.confirmed,
  one_sided: channelStatusLabels.one_sided,
  degraded: channelStatusLabels.degraded,
  configured: "仅配置 · 无报告",
}

export type ChannelFacts = {
  candidateId: string
  listenerId: string
  transport: ConnectionTransport
  dialerNodeId: string
  acceptorNodeId: string
  status: ChannelStatus
  dialer: { label: string; rtt: string | null }
  acceptor: { label: string; rtt: string | null }
}

export type EdgeFacts = {
  key: string
  nodeIds: [string, string]
  look: EdgeLook
  live: boolean
  channels: ChannelFacts[]
}

export function edgeFacts(adjacency: Adjacency): EdgeFacts {
  return {
    key: adjacency.key,
    nodeIds: adjacency.nodeIds,
    look: edgeLook(adjacency.status),
    live: isLive(adjacency),
    channels: adjacency.channels.map((channel) => ({
      candidateId: channel.candidateId,
      listenerId: channel.listenerId,
      transport: channel.transport,
      dialerNodeId: channel.dialerNodeId,
      acceptorNodeId: channel.acceptorNodeId,
      status: channel.status,
      // Each side's own measurement; never averaged, halved or summed.
      dialer: { label: claimLabel(channel.dialer), rtt: claimRtt(channel.dialer) },
      acceptor: { label: claimLabel(channel.acceptor), rtt: claimRtt(channel.acceptor) },
    })),
  }
}

/**
 * Outcome of the latest runtime read. `cached` keeps the last successful
 * snapshot for context after a failed refresh: its reports keep ageing on
 * their own lease and it is never presented as the current read.
 */
export type RuntimeRead = { kind: "current" } | { kind: "cached"; at: number; error: string } | { kind: "failed"; error: string }

export function runtimeRead(runtimes: { snapshot: unknown; fetchedAt: number | null; error: string | null }): RuntimeRead {
  if (runtimes.error === null && runtimes.snapshot !== null) return { kind: "current" }
  if (runtimes.snapshot !== null && runtimes.fetchedAt !== null) return { kind: "cached", at: runtimes.fetchedAt, error: runtimes.error ?? "" }
  return { kind: "failed", error: runtimes.error ?? "运行报告不可用" }
}

/**
 * The corner summary. Connection evidence is `unknown` when the runtime
 * reports could not be read or the plan is unavailable, and marked as cached
 * when only an earlier read is available; configured channels without any
 * proof are counted as unverified, never as healthy.
 */
export type OverviewSummary = {
  nodes: number
  placed: number
  connections:
    | { kind: "unknown"; reason: "plan" | "runtime" }
    | { kind: "none" }
    | { kind: "known"; configured: number; confirmed: number; oneSided: number; unverified: number; cachedAt: number | null }
}

export function overviewSummary({
  nodes,
  placements,
  observation,
  runtimes,
}: {
  nodes: NodeSummary[]
  placements: Map<string, NodePlacement>
  observation: TopologyObservation | null
  runtimes: RuntimeRead
}): OverviewSummary {
  const active = nodes.filter((node) => node.revoked_at === null)
  const placed = active.filter((node) => placements.get(node.id)?.kind === "geo").length
  const base = { nodes: active.length, placed }
  if (!observation) return { ...base, connections: { kind: "unknown", reason: "plan" } }
  if (observation.adjacencies.length === 0) return { ...base, connections: { kind: "none" } }
  if (runtimes.kind === "failed") return { ...base, connections: { kind: "unknown", reason: "runtime" } }
  const confirmed = observation.adjacencies.filter((adjacency) => adjacency.status === "confirmed").length
  const oneSided = observation.adjacencies.filter((adjacency) => adjacency.status === "one_sided").length
  return {
    ...base,
    connections: {
      kind: "known",
      configured: observation.adjacencies.length,
      confirmed,
      oneSided,
      unverified: observation.adjacencies.length - confirmed - oneSided,
      cachedAt: runtimes.kind === "cached" ? runtimes.at : null,
    },
  }
}

export function observeOverview(
  connections: ConnectionsSnapshot | null,
  runtimes: ConnectionRuntimesSnapshot | null,
  nodes: NodeSummary[],
  now: number
): TopologyObservation | null {
  return connections ? observeTopology(connections, runtimes, nodes, now) : null
}
