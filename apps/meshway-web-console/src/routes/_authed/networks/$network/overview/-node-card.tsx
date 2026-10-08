import type { AggregatedLink, NodeInventoryView } from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { ArrowLeftIcon, ArrowUpRightIcon } from "lucide-react"

import { CloseButton } from "@/components/chrome/close-button"
import { CopyButton } from "@/components/chrome/copy-button"
import { IconAction } from "@/components/chrome/icon-action"
import { NodeStatusDot } from "@/components/chrome/node-status"
import { asnText, placeText } from "@/lib/geo"
import { linkLook, type MapNode } from "@/lib/network-map"
import { statusLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

/** A selection stays put until closed; the compact card is the entry to the workspace. */
export function NodeCard({
  node,
  inventory,
  links,
  onOpen,
  onClose,
  onBack,
}: {
  node: MapNode
  inventory: NodeInventoryView | undefined
  links: readonly AggregatedLink[]
  onOpen: () => void
  onClose: () => void
  onBack?: () => void
}) {
  const connected = links.filter(
    (link) =>
      (link.a === node.id || link.b === node.id) && linkLook(link) === "matched"
  ).length
  const addresses = inventory
    ? [inventory.addresses.ipv4, inventory.addresses.ipv6].filter(
        (value) => value !== null
      )
    : []
  return (
    <div className="flex flex-col gap-4 p-4">
      <div className="flex min-w-0 items-center gap-2">
        {onBack ? (
          <IconAction
            label={m.overview_nodes()}
            icon={ArrowLeftIcon}
            onClick={onBack}
          />
        ) : null}
        <h2 className="min-w-0 flex-1 truncate text-lead font-medium">
          {node.name}
        </h2>
        <CloseButton onClick={onClose} />
      </div>
      <div className="flex items-center gap-2 text-row">
        <NodeStatusDot status={node.status} />
        {statusLabel[node.status]()}
      </div>
      {addresses.length ? (
        <div className="rounded-tile bg-fill-subtle px-3 py-1">
          {addresses.map((address) => (
            <div key={address} className="flex min-h-9 items-center gap-2">
              <span className="min-w-0 flex-1 truncate font-mono text-meta">
                {address}
              </span>
              <CopyButton value={address} label={m.nodes_mesh_access()} />
            </div>
          ))}
        </div>
      ) : null}
      <div className="min-h-12 text-meta text-muted-foreground">
        {node.geo?.kind === "found" ? (
          <>
            <div>{placeText(node.geo.geo)}</div>
            <div className="mt-1">{asnText(node.geo)}</div>
          </>
        ) : null}
      </div>
      <div className="text-meta text-muted-foreground">
        {m.overview_connected({ count: connected })}
      </div>
      <Button onClick={onOpen} className="w-full">
        {m.overview_open_node()}
        <ArrowUpRightIcon />
      </Button>
    </div>
  )
}

export function GroupCard({
  nodes,
  onSelect,
  onClose,
}: {
  nodes: readonly MapNode[]
  onSelect: (id: string) => void
  onClose: () => void
}) {
  return (
    <div className="flex min-h-0 flex-col p-4">
      <div className="mb-3 flex items-center justify-between gap-2">
        <h2 className="text-lead font-medium">
          {m.overview_group_nodes({ count: nodes.length })}
        </h2>
        <CloseButton onClick={onClose} />
      </div>
      <ul className="scroll-edge min-h-0 overflow-y-auto">
        {nodes.map((node) => (
          <li key={node.id}>
            <button
              type="button"
              onClick={() => onSelect(node.id)}
              className="list-row flex min-h-14 w-full items-center gap-3 px-2 text-left"
            >
              <NodeStatusDot status={node.status} />
              <span className="min-w-0 flex-1 truncate text-row">
                {node.name}
              </span>
              <span className="text-meta text-muted-foreground">
                {statusLabel[node.status]()}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  )
}
