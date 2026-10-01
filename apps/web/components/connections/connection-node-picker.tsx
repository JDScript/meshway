"use client"

import type { ConnectionNode } from "@meshway/control-client"

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  nodeName,
  resolveConnectionNode,
  roleBlocker,
  type Role,
  type Transport,
} from "@/lib/connections-model"
import { cn } from "@/lib/utils"

/**
 * Picker over active nodes for one connection role. Nodes that cannot take
 * the role (no published key, missing capability) are listed but disabled
 * with the reason. A value referencing a revoked or unknown node, or one no
 * longer capable, stays selected and visibly flagged for explicit repair.
 */
export function ConnectionNodePicker({
  id,
  nodes,
  value,
  onValueChange,
  transport,
  role,
  exclude = [],
  placeholder,
  disabled,
  invalid,
  className,
  "aria-describedby": ariaDescribedBy,
}: {
  id?: string
  nodes: ConnectionNode[]
  value: string
  onValueChange: (value: string) => void
  transport: Transport
  role: Role
  exclude?: string[]
  placeholder: string
  disabled?: boolean
  invalid?: boolean
  className?: string
  "aria-describedby"?: string
}) {
  const active = nodes.filter((entry) => entry.node.revoked_at === null)
  const current = value === "" ? null : resolveConnectionNode(nodes, value)
  const stale = current !== null && current.kind !== "active"
  const options = active.map((entry) => ({
    entry,
    blocker: exclude.includes(entry.node.id)
      ? "不能连接自己"
      : roleBlocker(entry, transport, role),
  }))
  const currentBlocked =
    current?.kind === "active" && options.find((o) => o.entry.node.id === value)?.blocker != null

  const items = [
    ...(stale ? [{ value, label: nodeName(nodes, value) }] : []),
    ...active.map((entry) => ({ value: entry.node.id, label: entry.node.name })),
  ]

  return (
    <Select<string>
      value={value === "" ? null : value}
      items={items}
      disabled={disabled}
      onValueChange={(next) => {
        if (typeof next === "string") onValueChange(next)
      }}
    >
      <SelectTrigger
        id={id}
        aria-describedby={ariaDescribedBy}
        aria-invalid={invalid || stale || currentBlocked || undefined}
        className={cn("min-w-0", (stale || currentBlocked) && "text-destructive", className)}
      >
        <SelectValue placeholder={placeholder} className="truncate" />
      </SelectTrigger>
      <SelectContent alignItemWithTrigger={false} align="start" className="min-w-64">
        {stale ? (
          <SelectItem value={value} className="text-destructive">
            {nodeName(nodes, value)}
          </SelectItem>
        ) : null}
        {active.length === 0 ? (
          <div className="px-2 py-1.5 text-xs text-muted-foreground">没有活动节点</div>
        ) : (
          options.map(({ entry, blocker }) => (
            <SelectItem
              key={entry.node.id}
              value={entry.node.id}
              disabled={blocker !== null && entry.node.id !== value}
            >
              <span className="truncate">{entry.node.name}</span>
              <span
                className={cn(
                  "text-[0.6875rem]",
                  blocker ? "text-warning" : "font-mono text-muted-foreground"
                )}
              >
                {blocker ?? entry.node.mesh_ipv4}
              </span>
            </SelectItem>
          ))
        )}
      </SelectContent>
    </Select>
  )
}
