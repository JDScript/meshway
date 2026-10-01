import type { NodeSummary } from "@meshway/control-client"

import { StatusBadge, StatusDot, type Tone } from "@/components/common/status-badge"
import {
  contactBucket,
  contactLabels,
  receiptLabels,
  receiptState,
  type ContactBucket,
  type ReceiptState,
} from "@/lib/node-status"
import { formatDateTime, formatRelative } from "@/lib/time"

const contactTone: Record<ContactBucket, Tone> = {
  recent: "success",
  hour: "warning",
  stale: "neutral",
  revoked: "neutral",
}

const receiptTone: Record<ReceiptState, Tone> = {
  current: "success",
  behind: "warning",
  never: "warning",
  revoked: "neutral",
}

export function ContactDot({ bucket, className }: { bucket: ContactBucket; className?: string }) {
  return <StatusDot tone={contactTone[bucket]} className={className} />
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
        <span className="font-mono text-[0.6875rem] text-muted-foreground">{formatDateTime(node.last_seen)}</span>
      ) : null}
      <span className="sr-only">（控制面联系，不代表在线）</span>
    </span>
  )
}

export function StatusBadgeForNode({ node }: { node: NodeSummary }) {
  return node.revoked_at !== null ? (
    <StatusBadge tone="danger">已撤销</StatusBadge>
  ) : (
    <StatusBadge tone="neutral">已注册</StatusBadge>
  )
}

export { StatusBadgeForNode as StatusBadge }

/** Configuration receipt: validated and durably saved, not network applied. */
export function ReceiptBadge({ node, compact = false }: { node: NodeSummary; compact?: boolean }) {
  const state = receiptState(node)
  if (state === "revoked") {
    return <span className="text-xs text-muted-foreground">{receiptLabels.revoked}</span>
  }
  const title = `节点已确认 r${node.acknowledged_revision}，目标 r${node.configuration_revision}。确认表示 agent 已校验并保存，不代表网络已生效。`
  return (
    <span className="inline-flex items-center gap-1.5" title={title}>
      <StatusBadge tone={receiptTone[state]}>{receiptLabels[state]}</StatusBadge>
      {compact ? null : (
        <span className="font-mono text-[0.6875rem] text-muted-foreground tabular-nums">
          {state === "current"
            ? `r${node.acknowledged_revision}`
            : `r${node.acknowledged_revision}→r${node.configuration_revision}`}
        </span>
      )}
    </span>
  )
}
