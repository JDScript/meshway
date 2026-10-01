"use client"

import type { ConnectionNode } from "@meshway/control-client"

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { accessRefLabel, refProblem, resolveAccessNode } from "@/lib/access-model"
import { cn } from "@/lib/utils"

/**
 * Picker over active nodes for one path position. Nodes without a published
 * key, or already used elsewhere in the path, are listed but disabled with
 * the reason. A value referencing a revoked, unknown or key-less node stays
 * selected and visibly flagged until it is explicitly replaced.
 */
export function AccessNodePicker({
  id,
  inventory,
  value,
  onValueChange,
  used,
  placeholder,
  disabled,
  invalid,
  className,
  "aria-label": ariaLabel,
  "aria-describedby": ariaDescribedBy,
  "data-testid": testId,
}: {
  id?: string
  inventory: ConnectionNode[]
  value: string
  onValueChange: (value: string) => void
  /** Node IDs taken by other positions of the same path. */
  used: string[]
  placeholder: string
  disabled?: boolean
  invalid?: boolean
  className?: string
  "aria-label"?: string
  "aria-describedby"?: string
  "data-testid"?: string
}) {
  const active = inventory.filter((entry) => entry.node.revoked_at === null)
  const current = value === "" ? null : resolveAccessNode(inventory, value)
  const flagged = current !== null && current.kind !== "ready"
  const outside = current !== null && (current.kind === "revoked" || current.kind === "unknown")

  const items = [
    ...(outside && current ? [{ value, label: accessRefLabel(current) }] : []),
    ...active.map((entry) => ({
      value: entry.node.id,
      label: accessRefLabel(resolveAccessNode(inventory, entry.node.id)),
    })),
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
        aria-label={ariaLabel}
        aria-describedby={ariaDescribedBy}
        aria-invalid={invalid || flagged || undefined}
        data-testid={testId}
        className={cn("min-w-0", flagged && "text-destructive", className)}
      >
        <SelectValue placeholder={placeholder} className="truncate" />
      </SelectTrigger>
      <SelectContent alignItemWithTrigger={false} align="start" className="min-w-64">
        {outside && current ? (
          <SelectItem value={value} className="text-destructive">
            {accessRefLabel(current)}
          </SelectItem>
        ) : null}
        {active.length === 0 ? (
          <div className="px-2 py-1.5 text-xs text-muted-foreground">没有活动节点</div>
        ) : (
          active.map((entry) => {
            const ref = resolveAccessNode(inventory, entry.node.id)
            const blocker =
              entry.node.id !== value && used.includes(entry.node.id)
                ? "已在路径中"
                : ref.kind === "no_key"
                  ? "未发布公钥"
                  : null
            return (
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
                  title={blocker === "未发布公钥" ? (refProblem(ref) ?? undefined) : undefined}
                >
                  {blocker ?? entry.node.mesh_ipv4}
                </span>
              </SelectItem>
            )
          })
        )}
      </SelectContent>
    </Select>
  )
}
