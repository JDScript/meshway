import type { NodeSummary } from "@meshway/control-client"

// Presentation helpers over the real inventory. "Contact" is the last time
// the controller accepted a registration or heartbeat from the node; it says
// nothing about tunnels, reachability or data-plane health.

export type ContactBucket = "recent" | "hour" | "stale" | "revoked"

/** Default agent heartbeat interval is 15 s; allow several missed beats. */
const RECENT_MS = 2 * 60 * 1000
const HOUR_MS = 60 * 60 * 1000

export function contactBucket(node: NodeSummary, reference: number): ContactBucket {
  if (node.revoked_at !== null) return "revoked"
  const seen = Date.parse(node.last_seen)
  if (Number.isNaN(seen)) return "stale"
  const age = reference - seen
  if (age <= RECENT_MS) return "recent"
  if (age <= HOUR_MS) return "hour"
  return "stale"
}

export const contactLabels: Record<ContactBucket, string> = {
  recent: "近 2 分钟有联系",
  hour: "1 小时内有联系",
  stale: "超过 1 小时未联系",
  revoked: "已撤销",
}

export type ReceiptState = "current" | "behind" | "never" | "revoked"

/** An acknowledgement means validated and durably saved, not applied. */
export function receiptState(node: NodeSummary): ReceiptState {
  if (node.revoked_at !== null) return "revoked"
  if (node.acknowledged_revision === node.configuration_revision) return "current"
  if (node.acknowledged_revision === 0) return "never"
  return "behind"
}

export const receiptLabels: Record<ReceiptState, string> = {
  current: "已确认",
  behind: "待确认",
  never: "从未确认",
  revoked: "不再下发",
}

export type InventorySummary = {
  total: number
  active: number
  revoked: number
  recent: number
  receipt: { current: number; behind: number; never: number }
}

export function summarizeInventory(
  nodes: NodeSummary[],
  reference: number
): InventorySummary {
  const summary: InventorySummary = {
    total: nodes.length,
    active: 0,
    revoked: 0,
    recent: 0,
    receipt: { current: 0, behind: 0, never: 0 },
  }
  for (const node of nodes) {
    if (node.revoked_at !== null) {
      summary.revoked += 1
      continue
    }
    summary.active += 1
    if (contactBucket(node, reference) === "recent") summary.recent += 1
    const receipt = receiptState(node)
    if (receipt !== "revoked") summary.receipt[receipt] += 1
  }
  return summary
}

export function activeNodes(nodes: NodeSummary[]): NodeSummary[] {
  return nodes.filter((node) => node.revoked_at === null)
}

/**
 * Outcome of the latest background read of one status source. Responses can
 * arrive out of order, so an older read never overrides a newer one: a late
 * failure cannot flag data a newer read refreshed, and a late success cannot
 * hide a newer failure.
 */
export type PartStatus = { at: number; error: string | null }

export function nextPartStatus(previous: PartStatus, at: number, error: string | null): PartStatus {
  return at > previous.at ? { at, error } : previous
}

/**
 * Worth a look on the inventory: an active node whose configuration was
 * never or not yet acknowledged, or that has not contacted the controller
 * for over an hour. Neither says anything about tunnels.
 */
export function needsAttention(node: NodeSummary, reference: number): boolean {
  if (node.revoked_at !== null) return false
  const receipt = receiptState(node)
  return receipt === "behind" || receipt === "never" || contactBucket(node, reference) === "stale"
}

export type NodeCarriers = { listeners: string[]; outgoing: number }

/** Saved carriers per node: listener transports it accepts on, and how many connections it dials. */
export function carriersByNode(
  plan: { listeners: { node_id: string; transport: string }[]; candidates: { dialer_node_id: string }[] }
): Map<string, NodeCarriers> {
  const result = new Map<string, NodeCarriers>()
  const entry = (nodeId: string) => {
    const existing = result.get(nodeId)
    if (existing) return existing
    const created: NodeCarriers = { listeners: [], outgoing: 0 }
    result.set(nodeId, created)
    return created
  }
  for (const listener of plan.listeners) entry(listener.node_id).listeners.push(listener.transport)
  for (const candidate of plan.candidates) entry(candidate.dialer_node_id).outgoing += 1
  return result
}
