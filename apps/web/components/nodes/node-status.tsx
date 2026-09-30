import type { NodeSummary } from "@meshway/control-client"

import { Badge } from "@/components/ui/badge"
import {
  contactBucket,
  contactLabels,
  receiptLabels,
  receiptState,
  type ContactBucket,
} from "@/lib/node-status"
import { formatDateTime, formatRelative } from "@/lib/time"
import { cn } from "@/lib/utils"

const dotClass: Record<ContactBucket, string> = {
  recent: "bg-success",
  hour: "bg-warning",
  stale: "bg-muted-foreground/40",
  revoked: "bg-destructive",
}

export function ContactDot({ bucket, className }: { bucket: ContactBucket; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn("inline-block size-1.5 shrink-0 rounded-full", dotClass[bucket], className)}
    />
  )
}

/**
 * Last control-plane contact, relative to when the page read the inventory.
 * Deliberately not called "online": it is heartbeat bookkeeping only.
 */
export function LastContact({
  node,
  fetchedAt,
  detail = false,
}: {
  node: NodeSummary
  fetchedAt: number
  detail?: boolean
}) {
  const bucket = contactBucket(node, fetchedAt)
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5" title={contactLabels[bucket]}>
      <ContactDot bucket={bucket === "revoked" ? "stale" : bucket} />
      <time dateTime={node.last_seen} className="truncate">
        {formatRelative(node.last_seen, fetchedAt) ?? "—"}
      </time>
      {detail ? (
        <span className="font-mono text-[0.6875rem] text-muted-foreground">
          {formatDateTime(node.last_seen)}
        </span>
      ) : null}
      <span className="sr-only">（控制面联系，不代表在线）</span>
    </span>
  )
}

export function StatusBadge({ node }: { node: NodeSummary }) {
  return node.revoked_at !== null ? (
    <Badge variant="destructive">已撤销</Badge>
  ) : (
    <Badge variant="outline">已注册</Badge>
  )
}

/** Configuration receipt: validated and durably saved, not network applied. */
export function ReceiptBadge({ node }: { node: NodeSummary }) {
  const state = receiptState(node)
  if (state === "revoked") {
    return <span className="text-xs text-muted-foreground">{receiptLabels.revoked}</span>
  }
  const title = `节点已确认 r${node.acknowledged_revision}，目标 r${node.configuration_revision}。确认表示 agent 已校验并保存，不代表网络已生效。`
  return (
    <span className="inline-flex items-center gap-1.5" title={title}>
      <Badge
        variant={state === "current" ? "outline" : "secondary"}
        className={cn(state === "current" && "text-success", state === "never" && "text-warning")}
      >
        {receiptLabels[state]}
      </Badge>
      <span className="font-mono text-[0.6875rem] text-muted-foreground tabular-nums">
        {state === "current"
          ? `r${node.acknowledged_revision}`
          : `r${node.acknowledged_revision}→r${node.configuration_revision}`}
      </span>
    </span>
  )
}
