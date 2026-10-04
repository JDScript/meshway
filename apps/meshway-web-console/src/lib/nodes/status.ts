import type {
  Freshness,
  NodeInventoryView,
  NodeStateView,
} from "@meshway/control-client"

import { keyRevoked } from "./model"

// A node's status in one word and one colour, the same on every page: the
// controller's freshness of its report, unless the node is turned off or
// its key revoked. Green when it is online; orange when it needs a look
// but is not lost (no report yet, just restarted, an older configuration);
// red when it is gone (offline, revoked); grey when off or not in the State.

export type NodeStatus = Freshness | "disabled" | "unknown"

export type StatusTone = "online" | "attention" | "offline" | "inactive"

/**
 * `node` is the inventory's entry (absent for a peer this account cannot
 * list), `view` the State's.
 */
export function nodeStatus(
  node: Pick<
    NodeInventoryView,
    "enabled" | "controlKeyRevokedAt" | "dataplaneKeyRevokedAt"
  > | null,
  view: Pick<NodeStateView, "freshness"> | undefined
): NodeStatus {
  if (node && !node.enabled) return "disabled"
  if (node && keyRevoked(node)) return "revoked"
  return view?.freshness ?? "unknown"
}

export const statusTone: Record<NodeStatus, StatusTone> = {
  current: "online",
  awaiting_report: "attention",
  superseded: "attention",
  config_mismatch: "attention",
  expired: "offline",
  revoked: "offline",
  disabled: "inactive",
  unknown: "inactive",
}

/** The order statuses are counted in: online first, then what needs a look. */
export const STATUS_ORDER: readonly NodeStatus[] = [
  "current",
  "awaiting_report",
  "superseded",
  "config_mismatch",
  "expired",
  "revoked",
  "disabled",
  "unknown",
]

/** When the node was last heard from: its last report, else its last sign-in. */
export const lastSeen = (
  view: Pick<NodeStateView, "receivedAt" | "lastAuthenticatedAt"> | undefined
): string | null => view?.receivedAt ?? view?.lastAuthenticatedAt ?? null
