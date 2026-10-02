import type { NodeSummary } from "@meshway/control-client"
import { useEffect, type CSSProperties } from "react"

import { CloseButton } from "@/components/chrome/close-button"
import { CopyButton } from "@/components/chrome/copy-button"
import { SegmentedTabs } from "@/components/chrome/segmented-control"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { Surface } from "@/features/shell/surface"
import { NODE_TABS, type NodeTab } from "@/features/shell/url"
import { nodeAdjacencies } from "@/features/topology/topology"
import { dateTime, relativeTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { NodeAddresses, NodeLinks, NodeStatusTiles, nodeVitality, Tile, VitalityLine, type NodeContext } from "./node-facts"

const isTab = (value: unknown): value is NodeTab => NODE_TABS.some((tab) => tab === value)

/**
 * One node in full, over the globe: everything the peek has and room for
 * what only a workspace holds. Sections that do not exist yet are not shown.
 */
export function NodeWorkspace({
  nodeId,
  tab,
  inventoryRead,
  context,
  className,
  style,
  onClose,
  onTab,
  onOpenNode,
}: {
  /** null closes the workspace. */
  nodeId: string | null
  tab: NodeTab
  inventoryRead: boolean
  context: NodeContext
  className?: string
  style?: CSSProperties
  onClose: () => void
  onTab: (tab: NodeTab) => void
  onOpenNode: (nodeId: string) => void
}) {
  const shown = nodeId !== null

  useEffect(() => {
    if (!shown) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) onClose()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [shown, onClose])

  const node = nodeId ? (context.nodes.get(nodeId) ?? null) : null
  return (
    <Surface shown={shown} from="rise" aria-labelledby="node-workspace-title" className={className} style={style}>
      {nodeId === null ? null : node ? (
        <Body key={node.id} node={node} tab={tab} context={context} onClose={onClose} onTab={onTab} onOpenNode={onOpenNode} />
      ) : (
        <Missing loading={!inventoryRead} onClose={onClose} />
      )}
    </Surface>
  )
}

function Missing({ loading, onClose }: { loading: boolean; onClose: () => void }) {
  return (
    <SurfaceHeader
      size="lg"
      className="px-6 pt-6 pb-4"
      title={(className) =>
        loading ? (
          <Skeleton className="h-6 w-40 flex-1 rounded-full" />
        ) : (
          <h2 id="node-workspace-title" className={className}>
            {m.node_not_found()}
          </h2>
        )
      }
      action={<CloseButton onClick={onClose} />}
    />
  )
}

function Body({
  node,
  tab,
  context,
  onClose,
  onTab,
  onOpenNode,
}: {
  node: NodeSummary
  tab: NodeTab
  context: NodeContext
  onClose: () => void
  onTab: (tab: NodeTab) => void
  onOpenNode: (nodeId: string) => void
}) {
  const links = context.observation ? nodeAdjacencies(context.observation, node.id).length : null
  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col">
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2 id="node-workspace-title" className={className}>
            {node.name}
          </h2>
        )}
        action={<CloseButton onClick={onClose} />}
        meta={
          <div className="flex items-center gap-3">
            <VitalityLine vitality={nodeVitality(node, context)} />
            <span className="font-mono text-xs text-muted-foreground">{node.mesh_ipv4}</span>
          </div>
        }
      />

      <Tabs
        value={tab}
        onValueChange={(value) => {
          if (isTab(value)) onTab(value)
        }}
        className="min-h-0 flex-1 gap-0"
      >
        <SegmentedTabs
          value={tab}
          className="mx-6"
          options={[
            { value: "overview", label: m.node_tab_overview() },
            {
              value: "links",
              label: (
                <>
                  {m.node_tab_links()}
                  <span className="text-muted-foreground tabular-nums">{links ?? ""}</span>
                </>
              ),
            },
          ]}
        />
        <TabsContent value="overview" className="min-h-0 overflow-y-auto px-6 pt-5 pb-6">
          <div className="grid grid-cols-2 gap-2 xl:grid-cols-4">
            <div className="col-span-2">
              <NodeAddresses node={node} />
            </div>
            <Tile label={m.node_registered()}>
              {dateTime(node.registered_at) ?? "—"}
              <span className="font-normal text-muted-foreground"> · {relativeTime(node.registered_at, context.now)}</span>
            </Tile>
            <Tile label={m.node_id()} action={<CopyButton value={node.id} label={m.node_id()} />}>
              <span className="font-mono font-normal">{node.id}</span>
            </Tile>
            <NodeStatusTiles node={node} context={context} />
          </div>
        </TabsContent>
        <TabsContent value="links" className="min-h-0 overflow-y-auto px-6 pt-5 pb-6">
          <NodeLinks node={node} context={context} onOpenPeer={onOpenNode} />
        </TabsContent>
      </Tabs>
    </div>
  )
}
