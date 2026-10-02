import type { ConnectionRuntimesSnapshot, NodeSummary } from "@meshway/control-client"

import { freshnessLabel, lookLabel, rttText, vitalityLabel } from "@/features/overview/labels"
import { lookOf, vitalityOf } from "@/features/overview/model"
import type { LinkLook, Vitality } from "@/features/globe/scene"
import { runtimeFor, runtimeFreshness } from "@/features/topology/runtime"
import { claimRttMs, nodeAdjacencies, otherEnd, type TopologyObservation } from "@/features/topology/topology"
import { relativeTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

/** One node's facts, already worded; each design concept only lays them out. */
export type NodeFacts = {
  id: string
  name: string
  vitality: Vitality
  vitalityText: string
  ipv4: string
  ipv6: string
  report: string
  contact: string
  receipt: { text: string; pending: boolean }
  version: string
  location: string
  links: { key: string; peerId: string; peer: string; look: LinkLook; lookText: string; own: string | null; theirs: string | null }[]
}

export function nodeFacts(
  node: NodeSummary,
  nodes: ReadonlyMap<string, NodeSummary>,
  observation: TopologyObservation | null,
  runtimes: ConnectionRuntimesSnapshot | null,
  now: number
): NodeFacts {
  const vitality = vitalityOf(node, runtimes, now)
  const report = runtimeFor(runtimes, node.id)
  const reportedAt = report?.reported_at ? relativeTime(report.reported_at, now) : null
  const pending = node.acknowledged_revision < node.configuration_revision
  return {
    id: node.id,
    name: node.name,
    vitality,
    vitalityText: vitalityLabel[vitality](),
    ipv4: node.mesh_ipv4,
    ipv6: node.mesh_ipv6,
    report: [freshnessLabel[runtimeFreshness(report, now)](), reportedAt].filter(Boolean).join(" · "),
    contact: relativeTime(node.last_seen, now) ?? m.node_never(),
    receipt: {
      pending,
      text: pending
        ? m.node_receipt_pending({ target: String(node.configuration_revision), acknowledged: String(node.acknowledged_revision) })
        : m.node_receipt_current({ revision: String(node.acknowledged_revision) }),
    },
    version: node.agent_version,
    location: m.node_location_unknown(),
    links: (observation ? nodeAdjacencies(observation, node.id) : []).map((adjacency) => {
      const peerId = otherEnd(adjacency, node.id)
      const look = lookOf(adjacency.status)
      const channel = adjacency.channels[0]
      const mine = channel ? (channel.dialerNodeId === node.id ? channel.dialer : channel.acceptor) : null
      const theirs = channel ? (channel.dialerNodeId === node.id ? channel.acceptor : channel.dialer) : null
      return {
        key: adjacency.key,
        peerId,
        peer: nodes.get(peerId)?.name ?? peerId.slice(0, 12),
        look,
        lookText: lookLabel[look](),
        own: mine ? rttText(claimRttMs(mine)) : null,
        theirs: theirs ? rttText(claimRttMs(theirs)) : null,
      }
    }),
  }
}
