"use client"

import type { NodeSummary } from "@meshway/control-client"
import type { ComponentProps } from "react"

import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { nodeRefLabel, resolveNode } from "@/lib/policy-model"

/**
 * Picker over active nodes. A value that references a revoked or unknown node
 * stays selectable and visibly labelled, so loading a policy never silently
 * rewrites it.
 */
export function NodeSelect({
  nodes,
  value,
  onValueChange,
  placeholder,
  exclude = [],
  ...props
}: Omit<ComponentProps<typeof NativeSelect>, "value" | "onChange"> & {
  nodes: NodeSummary[]
  value: string
  onValueChange: (value: string) => void
  placeholder: string
  exclude?: string[]
}) {
  const active = nodes.filter(
    (node) => node.revoked_at === null && (node.id === value || !exclude.includes(node.id))
  )
  const current = value === "" ? null : resolveNode(nodes, value)

  return (
    <NativeSelect
      value={value}
      onChange={(event) => onValueChange(event.target.value)}
      aria-invalid={(current !== null && current.kind !== "active") || undefined}
      {...props}
    >
      <NativeSelectOption value="">{placeholder}</NativeSelectOption>
      {current !== null && current.kind !== "active" ? (
        <NativeSelectOption value={value}>{nodeRefLabel(current)}</NativeSelectOption>
      ) : null}
      {active.map((node) => (
        <NativeSelectOption key={node.id} value={node.id}>
          {node.name} · {node.mesh_ipv4}
          {node.country ? ` · ${node.country}` : ""}
        </NativeSelectOption>
      ))}
    </NativeSelect>
  )
}
