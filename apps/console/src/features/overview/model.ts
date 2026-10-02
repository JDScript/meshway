import type {
  ConnectionRuntimesSnapshot,
  ConnectionsSnapshot,
  NodeSummary,
} from "@meshway/control-client"

import type { LinkLook, Scene, SceneNode, Vitality } from "../globe/scene.ts"
import { runtimeFor, runtimeFreshness } from "../topology/runtime.ts"
import {
  observeTopology,
  type ChannelStatus,
  type TopologyObservation,
} from "../topology/topology.ts"

/** Control-plane contact this recent counts as "contact" (not data-plane evidence). */
const CONTACT_WINDOW_MS = 2 * 60_000

export function vitalityOf(
  node: NodeSummary,
  runtimes: ConnectionRuntimesSnapshot | null,
  now: number
): Vitality {
  if (runtimeFreshness(runtimeFor(runtimes, node.id), now) === "fresh")
    return "live"
  const seen = Date.parse(node.last_seen)
  return !Number.isNaN(seen) && now - seen < CONTACT_WINDOW_MS
    ? "contact"
    : "quiet"
}

export const lookOf = (status: ChannelStatus): LinkLook =>
  status === "unobserved" ? "configured" : status

export type OverviewInput = {
  nodes: NodeSummary[]
  connections: ConnectionsSnapshot | null
  runtimes: ConnectionRuntimesSnapshot | null
  now: number
}

/**
 * What the globe draws for a network. No location source exists yet, so
 * every node is unplaced (on the orbit); links come only from the saved
 * plan with the agents' own reports.
 */
export function overviewScene({
  nodes,
  connections,
  runtimes,
  now,
}: OverviewInput): {
  scene: Scene
  observation: TopologyObservation | null
} {
  const active = nodes.filter((node) => node.revoked_at === null)
  const observation = connections
    ? observeTopology(connections, runtimes, active, now)
    : null
  const sceneNodes: SceneNode[] = active.map((node) => ({
    id: node.id,
    name: node.name,
    placement: null,
    vitality: vitalityOf(node, runtimes, now),
  }))
  const links = (observation?.adjacencies ?? []).map((adjacency) => ({
    key: adjacency.key,
    nodeIds: adjacency.nodeIds,
    look: lookOf(adjacency.status),
  }))
  return { scene: { nodes: sceneNodes, links }, observation }
}

export type LinkSummary =
  | { kind: "unknown" }
  | { kind: "none" }
  | {
      kind: "known"
      total: number
      confirmed: number
      oneSided: number
      degraded: number
      unreported: number
    }

/** Unknown when the plan or every runtime read is missing; never "healthy" by default. */
export function linkSummary(
  observation: TopologyObservation | null,
  runtimesRead: boolean
): LinkSummary {
  if (!observation) return { kind: "unknown" }
  if (observation.adjacencies.length === 0) return { kind: "none" }
  if (!runtimesRead) return { kind: "unknown" }
  const count = (status: ChannelStatus) =>
    observation.adjacencies.filter((adjacency) => adjacency.status === status)
      .length
  return {
    kind: "known",
    total: observation.adjacencies.length,
    confirmed: count("confirmed"),
    oneSided: count("one_sided"),
    degraded: count("degraded"),
    unreported: count("unobserved"),
  }
}
