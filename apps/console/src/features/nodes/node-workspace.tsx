import type { NodeSummary } from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"

import { CloseButton } from "@/components/chrome/close-button"
import { CopyButton } from "@/components/chrome/copy-button"
import {
  SegmentCount,
  SegmentedTabs,
} from "@/components/chrome/segmented-control"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Tile } from "@/components/chrome/tile"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { useCanManage } from "@/features/network/role"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { nodeAdjacencies } from "@/features/topology/topology"
import { dateTime, relativeTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { listenersOf } from "./listener-model"
import {
  NodeAddresses,
  NodeStatusTiles,
  nodeVitality,
  VitalityLine,
  type NodeContext,
} from "./node-facts"
import { NodeLinksTab } from "./node-links"
import { NodeManage } from "./node-manage"
import { NodeTransport } from "./node-transport"
import { managedListenersQuery } from "./queries"
import { NODE_TABS, type NodeDetail, type NodeTab } from "./url"

const isTab = (value: unknown): value is NodeTab =>
  NODE_TABS.some((tab) => tab === value)

/**
 * One node in full: its facts, its links with their history, its managed
 * transport with what the node reports about it, and its management. The
 * shell frames it (a surface over the globe, or a sheet on small screens).
 */
export function NodeWorkspace({
  networkId,
  nodeId,
  tab,
  detail,
  inventoryRead,
  context,
  onClose,
  onTab,
  onDetail,
  onOpenNode,
}: {
  networkId: string
  nodeId: string
  tab: NodeTab
  detail: NodeDetail
  inventoryRead: boolean
  context: NodeContext
  onClose: () => void
  onTab: (tab: NodeTab) => void
  onDetail: (detail: NodeDetail) => void
  onOpenNode: (nodeId: string) => void
}) {
  const node = context.nodes.get(nodeId) ?? null
  return node ? (
    <Body
      key={node.id}
      networkId={networkId}
      node={node}
      tab={tab}
      detail={detail}
      context={context}
      onClose={onClose}
      onTab={onTab}
      onDetail={onDetail}
      onOpenNode={onOpenNode}
    />
  ) : (
    <Missing loading={!inventoryRead} onClose={onClose} />
  )
}

function Missing({
  loading,
  onClose,
}: {
  loading: boolean
  onClose: () => void
}) {
  return (
    <SurfaceHeader
      size="lg"
      className="px-6 pt-6 pb-4"
      title={(className) =>
        loading ? (
          <Skeleton className="h-6 w-40 flex-1 rounded-full" />
        ) : (
          <h2 id={surfaceTitleId("nodes", "workspace")} className={className}>
            {m.node_not_found()}
          </h2>
        )
      }
      action={<CloseButton onClick={onClose} />}
    />
  )
}

const PANE = "min-h-0 overflow-y-auto px-6 pt-5 pb-(--commit-space)"

function Body({
  networkId,
  node,
  tab,
  detail,
  context,
  onClose,
  onTab,
  onDetail,
  onOpenNode,
}: {
  networkId: string
  node: NodeSummary
  tab: NodeTab
  detail: NodeDetail
  context: NodeContext
  onClose: () => void
  onTab: (tab: NodeTab) => void
  onDetail: (detail: NodeDetail) => void
  onOpenNode: (nodeId: string) => void
}) {
  const canManage = useCanManage(networkId)
  const listeners = useQuery(managedListenersQuery(networkId))
  const links = context.observation
    ? nodeAdjacencies(context.observation, node.id).length
    : null
  const listenerCount = listeners.data
    ? listenersOf(listeners.data, node.id).length
    : null

  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col">
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2 id={surfaceTitleId("nodes", "workspace")} className={className}>
            {node.name}
          </h2>
        )}
        action={<CloseButton onClick={onClose} />}
        meta={
          <div className="flex items-center gap-3">
            <VitalityLine
              vitality={nodeVitality(node, context)}
              revoked={node.revoked_at !== null}
            />
            <span className="font-mono text-meta text-muted-foreground">
              {node.mesh_ipv4}
            </span>
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
        <div className="mx-6 overflow-x-auto">
          <SegmentedTabs
            value={tab}
            options={[
              { value: "overview", label: m.node_tab_overview() },
              {
                value: "links",
                label: (
                  <SegmentCount label={m.node_tab_links()} count={links} />
                ),
              },
              {
                value: "transport",
                label: (
                  <SegmentCount
                    label={m.node_tab_transport()}
                    count={listenerCount}
                  />
                ),
              },
              { value: "manage", label: m.node_tab_manage() },
            ]}
          />
        </div>
        <TabsContent value="overview" className={PANE}>
          <div className="grid grid-cols-2 gap-2 xl:grid-cols-4">
            <div className="col-span-2">
              <NodeAddresses node={node} />
            </div>
            <div className="col-span-2 xl:col-span-1">
              <Tile label={m.node_registered()}>
                {dateTime(node.registered_at) ?? "—"}
                <span className="font-normal text-muted-foreground">
                  {" "}
                  · {relativeTime(node.registered_at, context.now)}
                </span>
              </Tile>
            </div>
            <Tile
              label={m.node_id()}
              action={<CopyButton value={node.id} label={m.node_id()} />}
            >
              <span className="font-mono font-normal">{node.id}</span>
            </Tile>
            <NodeStatusTiles node={node} context={context} />
            {node.country !== null ? (
              <Tile label={m.node_legacy_country()}>
                <span className="font-mono font-normal">{node.country}</span>
              </Tile>
            ) : null}
          </div>
        </TabsContent>
        <TabsContent value="links" className={PANE}>
          <NodeLinksTab
            networkId={networkId}
            node={node}
            context={context}
            detail={detail}
            onDetail={onDetail}
            onOpenNode={onOpenNode}
          />
        </TabsContent>
        <TabsContent value="transport" className={PANE}>
          <NodeTransport
            networkId={networkId}
            node={node}
            context={context}
            canManage={canManage}
            editing={detail.listener}
            onEdit={(listener) => onDetail({ ...detail, listener })}
          />
        </TabsContent>
        <TabsContent value="manage" className={PANE}>
          <NodeManage networkId={networkId} node={node} canManage={canManage} />
        </TabsContent>
      </Tabs>
    </div>
  )
}
