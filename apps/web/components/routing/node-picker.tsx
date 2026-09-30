"use client"

import type { NodeSummary } from "@meshway/control-client"

import {
  Select,
  SelectContent,
  SelectItem,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { nodeRefLabel, resolveNode } from "@/lib/policy-model"
import { cn } from "@/lib/utils"

// Base UI treats "" like no selection; the explicit empty choice needs a
// distinct item value. Node IDs are controller-issued and never collide.
const NONE = "meshway:none"

/**
 * Picker over active nodes. A value that references a revoked or unknown node
 * stays selected and visibly labelled, so loading a policy never silently
 * rewrites it; only an explicit choice replaces it. `noneLabel` adds an
 * explicit empty choice whose value is "".
 */
export function NodePicker({
  id,
  nodes,
  value,
  onValueChange,
  placeholder,
  noneLabel,
  exclude = [],
  disabled,
  invalid,
  className,
  "aria-label": ariaLabel,
  "aria-describedby": ariaDescribedBy,
  "data-testid": testId,
}: {
  id?: string
  nodes: NodeSummary[]
  value: string
  onValueChange: (value: string) => void
  placeholder: string
  noneLabel?: string
  exclude?: string[]
  disabled?: boolean
  invalid?: boolean
  className?: string
  "aria-label"?: string
  "aria-describedby"?: string
  "data-testid"?: string
}) {
  const active = nodes.filter(
    (node) => node.revoked_at === null && (node.id === value || !exclude.includes(node.id))
  )
  const current = value === "" ? null : resolveNode(nodes, value)
  const stale = current !== null && current.kind !== "active" ? current : null

  const items: { value: string; label: string }[] = [
    ...(noneLabel !== undefined ? [{ value: NONE, label: noneLabel }] : []),
    ...(stale ? [{ value, label: nodeRefLabel(stale) }] : []),
    ...active.map((node) => ({ value: node.id, label: node.name })),
  ]

  return (
    <Select<string>
      value={value === "" ? (noneLabel === undefined ? null : NONE) : value}
      items={items}
      disabled={disabled}
      onValueChange={(next) => {
        if (typeof next === "string") onValueChange(next === NONE ? "" : next)
      }}
    >
      <SelectTrigger
        id={id}
        aria-label={ariaLabel}
        aria-describedby={ariaDescribedBy}
        aria-invalid={invalid || stale !== null || undefined}
        data-testid={testId}
        className={cn("min-w-0", stale && "text-destructive", className)}
      >
        <SelectValue placeholder={placeholder} className="truncate" />
      </SelectTrigger>
      <SelectContent alignItemWithTrigger={false} align="start" className="min-w-56">
        {noneLabel !== undefined ? (
          <>
            <SelectItem value={NONE}>
              <span className="text-muted-foreground">{noneLabel}</span>
            </SelectItem>
            <SelectSeparator />
          </>
        ) : null}
        {stale ? (
          <SelectItem value={value} className="text-destructive">
            {nodeRefLabel(stale)}
          </SelectItem>
        ) : null}
        {active.length === 0 ? (
          <div className="px-2 py-1.5 text-xs text-muted-foreground">没有可选的活动节点</div>
        ) : (
          active.map((node) => (
            <SelectItem key={node.id} value={node.id}>
              <span className="truncate">{node.name}</span>
              <span className="font-mono text-[0.6875rem] text-muted-foreground">
                {node.mesh_ipv4}
                {node.country ? ` · ${node.country}` : ""}
              </span>
            </SelectItem>
          ))
        )}
      </SelectContent>
    </Select>
  )
}
