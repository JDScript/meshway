import { NodeStatusDot } from "@/components/chrome/node-status"
import { useNow } from "@/hooks/use-now"
import type { NodeStatus } from "@/lib/nodes/status"

import { statusText } from "../-labels"

/**
 * A node's status as its dot and words. An offline node's "last seen"
 * ticks by this browser's clock, not with the reports.
 */
export function LiveStatus({
  status,
  seen,
}: {
  status: NodeStatus
  seen: string | null
}) {
  const now = useNow(1_000)
  return (
    <span className="inline-flex min-w-0 items-center gap-2">
      <NodeStatusDot status={status} />
      <span className="truncate">{statusText(status, seen, now)}</span>
    </span>
  )
}
