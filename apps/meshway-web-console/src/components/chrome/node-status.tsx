import {
  statusTone,
  type NodeStatus,
  type StatusTone,
} from "@/lib/nodes/status"

import { StatusDot, type DotShape } from "./status-dot"

/** Each tone's dot: lit only when online; hollow when off or unknown. */
const TONE: Record<StatusTone, { color: string; shape: DotShape }> = {
  online: { color: "var(--status-online)", shape: "live" },
  attention: { color: "var(--status-attention)", shape: "solid" },
  offline: { color: "var(--status-offline)", shape: "solid" },
  inactive: { color: "var(--status-inactive)", shape: "hollow" },
}

/** A node's status as a dot in its tone (`lib/nodes/status.ts`). */
export function NodeStatusDot({
  status,
  size = "md",
}: {
  status: NodeStatus
  size?: "sm" | "md"
}) {
  const { color, shape } = TONE[statusTone[status]]
  return <StatusDot color={color} shape={shape} size={size} />
}
